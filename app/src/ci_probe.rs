//! `wm ci-probe [--summary ARCHIVO]` — spike 9 (architecture.md §13.2): qué
//! se puede probar con ventanas reales en esta máquina. Pensado para los
//! runners de GitHub Actions (x64 y ARM64), pero corre en cualquier PC.
//!
//! Abre sus propios títeres (en otro proceso) y prueba, con plazo:
//! 1. **foco**: traer cada títere al frente (`SetForegroundWindow`, con la
//!    tecla neutra si hace falta);
//! 2. **hook + `SendInput`**: Caps + Q inyectado como si fuera del usuario
//!    tiene que llegar como acción al brain, sin dejar teclas trabadas;
//! 3. **esconder**: las cuatro estrategias (cloak, minimizar, fuera de
//!    pantalla, `SW_HIDE`), mirando DWM, y volver a mostrar;
//! 4. **marco DWM**: colocar por el marco visible con error ≤ 1 px;
//! 5. **overlay**: crear los overlays de Composition con el mapa de teclado.
//!
//! Imprime una línea por chequeo y, con `--summary`, agrega una tabla en
//! Markdown a ese archivo (en la CI, `$GITHUB_STEP_SUMMARY`). Sale con 0 si
//! todo dio bien, 1 si algo falló y 3 si se pasó del plazo total.
//!
//! Roba el foco unos segundos: no usar la PC mientras corre.

use std::io::Write;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use wm_model::keyboard::HyperAction;
use wm_model::{Placement, place};
use wm_platform::hide::{self, HideMethod, Hider};
use wm_platform::input::{self, InputEvent, Wake};
use wm_platform::overlay_demo::{Demo, Motion};
use wm_platform::overlay_text::TextPath;
use wm_platform::windows_list::{self, WindowId};
use wm_platform::{geometry, puppet, session};

/// Plazo total: si algo se traba, se restaura todo y se sale con 3.
const TOTAL: Duration = Duration::from_secs(90);
const PUPPETS: usize = 3;
const CHORDS: usize = 10;
const VK_CAPITAL: u8 = 0x14;

static PUPPET_PID: AtomicU32 = AtomicU32::new(0);

/// Como `println!`, pero sin pánico si la salida se cerró (por ejemplo, un
/// pipe a `head`): la sonda tiene que llegar siempre a limpiar.
macro_rules! out {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}

/// Al salir por cualquier camino (también un pánico): mostrar lo escondido
/// y cerrar los títeres.
struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = hide::restore_all();
        kill_puppets();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Fail,
    Skip,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::Fail => "FALLA",
            Status::Skip => "SIN PROBAR",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Status::Ok => "✅",
            Status::Fail => "❌",
            Status::Skip => "➖",
        }
    }
}

struct Report {
    env: Vec<(String, String)>,
    checks: Vec<(String, Status, String)>,
}

impl Report {
    fn env(&mut self, key: &str, value: String) {
        out!("  {key}: {value}");
        self.env.push((key.to_owned(), value));
    }

    fn check(&mut self, name: &str, status: Status, detail: String) {
        out!("[{}] {name}: {detail}", status.label());
        let _ = std::io::stdout().flush();
        self.checks.push((name.to_owned(), status, detail));
    }

    fn markdown(&self) -> String {
        let mut md = String::from("## wm ci-probe\n\n| Entorno | |\n|---|---|\n");
        for (k, v) in &self.env {
            md.push_str(&format!("| {k} | {v} |\n"));
        }
        md.push_str("\n| Chequeo | | Detalle |\n|---|---|---|\n");
        for (name, status, detail) in &self.checks {
            md.push_str(&format!("| {name} | {} {} | {detail} |\n", status.icon(), status.label()));
        }
        md.push('\n');
        md
    }
}

fn kill_puppets() {
    let pid = PUPPET_PID.swap(0, Ordering::Relaxed);
    if pid != 0 {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

pub fn run(args: &[String]) -> ExitCode {
    let summary = args.iter().position(|a| a == "--summary").and_then(|i| args.get(i + 1)).cloned();

    // Vigía: pase lo que pase, nada queda escondido ni colgado.
    std::thread::spawn(|| {
        std::thread::sleep(TOTAL);
        eprintln!("ci-probe: se pasó del plazo de {} s; restauro y salgo.", TOTAL.as_secs());
        let _ = hide::restore_all();
        kill_puppets();
        std::process::exit(3);
    });

    let cleanup = Cleanup;
    let mut r = Report { env: Vec::new(), checks: Vec::new() };
    out!("Entorno");
    environment(&mut r);

    out!("\nChequeos");
    let puppets = open_puppets(&mut r);
    if puppets.len() >= PUPPETS {
        focus(&mut r, &puppets);
        hook(&mut r, puppets[0]);
        hiding(&mut r, puppets[1]);
        frame(&mut r, puppets[2]);
    } else {
        for name in ["foco", "hook + SendInput", "esconder", "marco DWM"] {
            r.check(name, Status::Skip, "sin títeres".into());
        }
    }
    overlay(&mut r);

    drop(cleanup);

    if let Some(path) = summary {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path);
        match file.and_then(|mut f| f.write_all(r.markdown().as_bytes())) {
            Ok(()) => out!("\nResumen agregado a {path}"),
            Err(e) => eprintln!("No se pudo escribir el resumen en {path}: {e}"),
        }
    }
    let failed = r.checks.iter().filter(|(_, s, _)| *s == Status::Fail).count();
    let ok = r.checks.iter().filter(|(_, s, _)| *s == Status::Ok).count();
    out!("\n{ok} OK, {failed} con fallas, de {}", r.checks.len());
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn environment(r: &mut Report) {
    let build = wm_platform::os_build().unwrap_or_else(|| "?".into());
    let edition = wm_platform::os_edition().unwrap_or_default();
    r.env("Windows", format!("{edition} (build {build})"));
    r.env("Arquitectura", std::env::consts::ARCH.into());
    let s = session::session_info();
    let session = s.session_id.map_or_else(|| "?".into(), |id| id.to_string());
    r.env("Sesión", format!("{session} (consola: {})", s.console_session_id));
    r.env("Estación de ventanas", s.window_station.clone());
    r.env(
        "Escritorio de entrada",
        s.input_desktop.clone().unwrap_or_else(|| "no se puede abrir".into()),
    );
    let elevated = match s.elevated {
        Some(true) => "sí",
        Some(false) => "no",
        None => "?",
    };
    r.env("Elevado", elevated.into());
    r.env("Shell", if s.shell { "sí" } else { "no" }.into());
    // Solo la clase: el título puede tener datos personales y el resumen de
    // la CI es público. La clase alcanza para ver, por ejemplo, si quedó
    // abierta la configuración inicial de Windows.
    let fg = s.foreground.map_or_else(|| "ninguna".into(), |(class, _)| class);
    r.env("Clase de la ventana con foco al empezar", fg);
    let monitors = wm_platform::monitors();
    let list: Vec<String> = monitors
        .iter()
        .map(|m| {
            let hz = m.refresh_hz.map_or_else(|| "?".into(), |hz| hz.to_string());
            format!("{}x{} a {hz} Hz, {} DPI", m.bounds.width(), m.bounds.height(), m.dpi)
        })
        .collect();
    r.env("Monitores", format!("{}: {}", monitors.len(), list.join("; ")));
}

fn open_puppets(r: &mut Report) -> Vec<WindowId> {
    let Ok(exe) = std::env::current_exe() else {
        r.check("títeres", Status::Fail, "no se encontró el ejecutable".into());
        return Vec::new();
    };
    let child = Command::new(exe)
        .args(["puppet", &PUPPETS.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => {
            r.check("títeres", Status::Fail, format!("no se pudo lanzar: {e}"));
            return Vec::new();
        }
    };
    PUPPET_PID.store(child.id(), Ordering::Relaxed);
    let start = Instant::now();
    loop {
        let ours: Vec<WindowId> = puppet::find_puppets()
            .into_iter()
            .filter(|&id| session::window_pid(id) == child.id())
            .collect();
        if ours.len() >= PUPPETS {
            r.check("títeres", Status::Ok, format!("{} en {}", ours.len(), ms(start.elapsed())));
            // Que terminen de pintarse antes de tocarlos.
            std::thread::sleep(Duration::from_millis(300));
            return ours;
        }
        if start.elapsed() > Duration::from_secs(10) {
            r.check("títeres", Status::Fail, format!("aparecieron {} de {PUPPETS}", ours.len()));
            return ours;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn focus(r: &mut Report, puppets: &[WindowId]) {
    // Solo cuentan los cambios de verdad: pedir el foco para la ventana que
    // ya lo tiene "funciona" siempre y escondería el problema.
    let (mut ok, mut neutral, mut tries, mut already) = (0, 0, 0, 0);
    let mut times = Vec::new();
    for _ in 0..3 {
        for &id in puppets {
            if windows_list::foreground() == Some(id) {
                already += 1;
                continue;
            }
            let a = windows_list::activate(id);
            tries += 1;
            if a.ok {
                ok += 1;
                times.push(a.elapsed);
            }
            if a.used_neutral_key {
                neutral += 1;
            }
        }
    }
    times.sort();
    let p50 = times.get(times.len() / 2).map_or_else(|| "-".into(), |d| ms(*d));
    let status = if tries > 0 && ok == tries { Status::Ok } else { Status::Fail };
    let fg = match windows_list::foreground() {
        Some(id) if puppets.contains(&id) => "un títere",
        Some(_) => "otra ventana",
        None => "ninguna",
    };
    let detail = format!(
        "{ok}/{tries} cambios (con tecla neutra: {neutral}; ya tenía el foco: \
         {already}), p50 {p50}; al final, el foco lo tiene {fg}"
    );
    r.check("foco", status, detail);
}

fn hook(r: &mut Report, target: WindowId) {
    input::ensure_message_queue();
    let brain = input::current_thread_id();
    if let Err(e) = input::start_keyboard_hook(brain, false) {
        r.check("hook + SendInput", Status::Fail, format!("no se pudo instalar: {e}"));
        return;
    }
    // Que las teclas vayan a un títere y no a otra ventana.
    let _ = windows_list::activate(target);

    let (mut seen, mut accepted) = (0, 0);
    for _ in 0..CHORDS {
        accepted += session::send_hyper_chord(b'Q');
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match input::wait(Some(deadline)) {
                Wake::Input(event) => {
                    let hit = matches!(
                        event,
                        InputEvent::Action { action: HyperAction::Letter { letter: b'Q', .. }, .. }
                    );
                    let _ = input::acknowledge();
                    if hit {
                        seen += 1;
                        break;
                    }
                }
                Wake::Timeout | Wake::Quit => break,
                Wake::Ops | Wake::Ping(_) => {}
            }
        }
    }
    std::thread::sleep(Duration::from_millis(50));
    let stuck: Vec<&str> = [(VK_CAPITAL, "Caps"), (b'Q', "Q")]
        .into_iter()
        .filter(|&(vk, _)| session::key_down(vk))
        .map(|(_, name)| name)
        .collect();
    let stats = input::hook_stats();
    let status = if seen == CHORDS && stuck.is_empty() { Status::Ok } else { Status::Fail };
    let stuck = if stuck.is_empty() { "ninguna".into() } else { stuck.join(", ") };
    let detail = format!(
        "{seen}/{CHORDS} acordes llegaron al brain; SendInput aceptó {accepted}/{}; \
         teclas trabadas: {stuck}; callback p99 ≤ {} µs, máx {:.0} µs",
        CHORDS * 4,
        stats.plain.p99_us.max(stats.injecting.p99_us),
        stats.plain.max_us.max(stats.injecting.max_us),
    );
    r.check("hook + SendInput", status, detail);
}

fn hiding(r: &mut Report, id: WindowId) {
    let methods =
        [HideMethod::ShellCloak, HideMethod::Minimize, HideMethod::Offscreen, HideMethod::Hide];
    for method in methods {
        let name = format!("esconder: {}", method.name());
        let hider = match Hider::new(method) {
            Ok(h) => h,
            Err(e) => {
                r.check(&name, Status::Fail, format!("no disponible: {e}"));
                continue;
            }
        };
        let start = Instant::now();
        if let Err(e) = hider.hide(id) {
            r.check(&name, Status::Fail, format!("al esconder: {e}"));
            let _ = hider.show(id);
            continue;
        }
        let hide_time = start.elapsed();
        std::thread::sleep(Duration::from_millis(150));
        let hidden = hide::visibility(id);
        let start = Instant::now();
        let shown = hider.show(id);
        let show_time = start.elapsed();
        std::thread::sleep(Duration::from_millis(150));
        let back = hide::visibility(id);
        let ok_hidden = hide::looks_hidden(method, hidden);
        let ok_back = shown.is_ok() && hide::fully_visible(back);
        let status = if ok_hidden && ok_back { Status::Ok } else { Status::Fail };
        let detail = format!(
            "escondida: {} (DWMWA_CLOAKED={}), vuelve: {}; {} / {}",
            if ok_hidden { "sí" } else { "no" },
            hidden.cloaked,
            if ok_back { "sí" } else { "no" },
            ms(hide_time),
            ms(show_time),
        );
        r.check(&name, status, detail);
    }
}

fn frame(r: &mut Report, id: WindowId) {
    let monitors = wm_platform::monitors();
    let Some(m) = monitors.iter().find(|m| m.primary).or(monitors.first()) else {
        r.check("marco DWM", Status::Fail, "no hay monitores".into());
        return;
    };
    let target = place(m.work_area, Placement::LeftHalf);
    match geometry::place_frame(id, target) {
        Some(p) => {
            let status = if p.error <= 1 { Status::Ok } else { Status::Fail };
            let detail = format!(
                "error {} px en {} pasada(s), {}; la app se movió sola {} px",
                p.error,
                p.passes,
                ms(p.elapsed),
                p.drift
            );
            r.check("marco DWM", status, detail);
        }
        None => r.check("marco DWM", Status::Fail, "no se pudo leer el marco".into()),
    }
}

fn overlay(r: &mut Report) {
    let start = Instant::now();
    let result = Demo::new(Motion::Once(Duration::from_millis(300)), Some(TextPath::Shapes))
        .and_then(|mut demo| {
            demo.show()?;
            let built = start.elapsed();
            std::thread::sleep(Duration::from_millis(400));
            demo.hide()?;
            Ok(built)
        });
    match result {
        Ok(t) => {
            r.check("overlay (Composition + mapa)", Status::Ok, format!("creado en {}", ms(t)))
        }
        Err(e) => r.check("overlay (Composition + mapa)", Status::Fail, e.to_string()),
    }
}
