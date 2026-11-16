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
//! 5. **overlay**: crear los overlays de Composition con el mapa de teclado;
//! 6. **`wm core`**: el core real, manejado con teclas inyectadas: `Caps` + W
//!    lleva a otro títere (son `wm.exe`) y `Caps` + Esc lo cierra con código 0;
//! 7. **capa de ventana** (con el mismo core): `Caps` + Alt + A = mitad
//!    izquierda, W = mitad de arriba, D + S juntas = esquina, con
//!    el marco visible a ≤ 1 px;
//! 8. **empujar** (solo con dos monitores o más): repetir la dirección lleva
//!    la ventana al monitor vecino, entrando por la mitad opuesta, y vuelve;
//! 9. **ventanas difíciles** (`wm puppet-hard`): tamaño mínimo y fijo (quedan
//!    pegadas al borde), lenta, que se reacomoda sola (no se pelea), sin borde
//!    y sin DPI awareness;
//! 10. **colgada**: con una app congelada, `wm` sigue respondiendo y la ventana
//!     llega a su lugar cuando la app despierta;
//! 11. **vuelo** (hito 1e): mitad izquierda → derecha se anima (la real se ve
//!     con cloak durante el vuelo) y aterriza visible, a tiempo;
//! 12. **vuelo + caída**: matar el core en pleno vuelo deja la ventana con
//!     cloak, y `restore_all` (lo que hace el guardián al relanzar) la
//!     devuelve.
//!
//! "Llegó" exige además que la ventana no esté escondida: durante un vuelo,
//! el marco real ya está en el destino pero con cloak.
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
        core(&mut r, &puppets);
        fly_crash_check(&mut r, puppets[1]);
        hook(&mut r, puppets[0]);
        hiding(&mut r, puppets[1]);
        frame(&mut r, puppets[2]);
    } else {
        for name in [
            "foco",
            "wm core",
            "capa de ventana",
            "F, C y Z",
            "empujar",
            "ventanas difíciles",
            "colgada",
            "vuelo",
            "vuelo + caída",
            "hook + SendInput",
            "esconder",
            "marco DWM",
        ] {
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
    let reduced = windows_list::reduced_motion();
    r.env("Reducir animaciones", if reduced { "sí" } else { "no" }.into());
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

/// El core real (`wm core`) en otro proceso, manejado con teclas inyectadas.
/// Lanza `wm core` y espera su banner (plazo 5 s). Las líneas del core
/// siguen llegando por el canal.
fn spawn_core() -> Result<(std::process::Child, std::sync::mpsc::Receiver<String>), String> {
    use std::io::BufRead;
    let exe = std::env::current_exe().map_err(|_| "no se encontró el ejecutable".to_owned())?;
    let mut child = Command::new(exe)
        .arg("core")
        .env_remove("WM_TEST")
        .env_remove("WM_ANIM")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("no se pudo lanzar: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let started = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) if line.starts_with("wm core activo") => return Ok((child, rx)),
            Ok(_) => {}
            Err(_) if started.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                return Err("no arrancó en 5 s".into());
            }
            Err(_) => {}
        }
    }
}

/// Llegó a `target` y se ve: durante un vuelo el marco real ya está en el
/// destino, pero con cloak.
fn landed(id: WindowId, target: wm_model::Rect) -> bool {
    !wm_platform::ghost::is_cloaked(id)
        && hide::hidden_with(id).is_none()
        && wm_platform::ghost::frame_bounds(id)
            .is_some_and(|f| wm_model::place::edge_error(f, target) <= 1)
}

fn core(r: &mut Report, puppets: &[WindowId]) {
    let name = "wm core";
    let started = Instant::now();
    let (mut child, _lines) = match spawn_core() {
        Ok(x) => x,
        Err(e) => {
            r.check(name, Status::Fail, e);
            return;
        }
    };
    let boot = started.elapsed();

    let from = puppets[0];
    let _ = windows_list::activate(from);
    std::thread::sleep(Duration::from_millis(100));
    session::send_hyper_chord(b'W');
    let switch = Instant::now();
    let moved = loop {
        let fg = windows_list::foreground();
        if fg.is_some_and(|id| id != from && puppets.contains(&id)) {
            break Some(switch.elapsed());
        }
        if switch.elapsed() > Duration::from_secs(1) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(2));
    };

    // Capa de ventana sobre el títere que quedó al frente.
    if let Some(id) = windows_list::foreground().filter(|id| puppets.contains(id)) {
        arrange_checks(r, id);
    } else {
        r.check("capa de ventana", Status::Skip, "no quedó un títere al frente".into());
    }

    hard_checks(r);
    hung_check(r);

    session::send_hyper_chord(0x1B); // Caps + Esc: salir
    let exit = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if exit.elapsed() > Duration::from_secs(3) => {
                let _ = child.kill();
                break None;
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let ok = moved.is_some() && code == Some(0);
    let detail = format!(
        "arrancó en {}; Caps + W → otro títere: {}; Caps + Esc → {}",
        ms(boot),
        moved.map_or_else(|| "no".into(), |d| format!("sí, en {}", ms(d))),
        code.map_or_else(|| "no salió en 3 s".into(), |c| format!("código {c}")),
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
}

/// `Caps` + Alt + teclas con el core real corriendo; mide cuándo el marco
/// visible llega a cada ubicación.
fn arrange_checks(r: &mut Report, id: WindowId) {
    use wm_model::place::{MonitorObs, monitor_of};
    use wm_model::{Placement, place};
    const CAPS: u8 = 0x14;
    const ALT: u8 = 0xA4;

    let monitors: Vec<MonitorObs> = wm_platform::monitors()
        .iter()
        .map(|m| MonitorObs { bounds: m.bounds, work: m.work_area, dpi: m.dpi })
        .collect();
    let Some(frame) = wm_platform::ghost::frame_bounds(id) else {
        r.check("capa de ventana", Status::Fail, "no se pudo leer el marco".into());
        return;
    };
    let Some(m) = monitor_of(&monitors, frame) else {
        r.check("capa de ventana", Status::Fail, "no hay monitores".into());
        return;
    };
    let work = monitors[m].work;
    // (teclas después de Caps + Alt, ubicación esperada, nombre)
    let steps: [(&[u8], Placement, &str); 3] = [
        (b"A", Placement::LeftHalf, "A → mitad izquierda"),
        (b"W", Placement::TopHalf, "W → mitad de arriba"),
        (b"DS", Placement::BottomRight, "D + S juntas → esquina"),
    ];
    let mut parts = Vec::new();
    let mut all_ok = true;
    for (keys, placement, label) in steps {
        let mut seq = vec![(CAPS, true), (ALT, true)];
        seq.extend(keys.iter().map(|&k| (k, true)));
        seq.extend(keys.iter().rev().map(|&k| (k, false)));
        seq.extend([(ALT, false), (CAPS, false)]);
        let target = place(work, placement);
        let start = Instant::now();
        session::send_keys(&seq);
        let reached = loop {
            if landed(id, target) {
                break Some(start.elapsed());
            }
            if start.elapsed() > Duration::from_secs(1) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        all_ok &= reached.is_some();
        parts.push(match reached {
            Some(d) => format!("{label} en {}", ms(d)),
            None => format!("{label}: NO llegó"),
        });
        std::thread::sleep(Duration::from_millis(150));
    }
    let status = if all_ok { Status::Ok } else { Status::Fail };
    r.check("capa de ventana", status, parts.join("; "));
    fcz_checks(r, id, monitors[m].work);
    push_checks(r, id, &monitors, m);
    fly_check(r, id, monitors[m].work);
}

/// Hito 1e: mitad izquierda → derecha es del mismo tamaño, así que vuela.
/// Durante el vuelo la real está con cloak (debajo del fantasma); al final,
/// en el destino y visible, a tiempo.
/// Con "reducir animaciones" el core no anima (a propósito): los chequeos de
/// vuelo se omiten.
fn skip_if_reduced_motion(r: &mut Report, name: &str) -> bool {
    let reduced = windows_list::reduced_motion();
    if reduced {
        r.check(name, Status::Skip, "el sistema pide reducir animaciones: no se anima".into());
    }
    reduced
}

fn fly_check(r: &mut Report, id: WindowId, work: wm_model::Rect) {
    let name = "vuelo";
    if skip_if_reduced_motion(r, name) {
        return;
    }
    let left = place(work, Placement::LeftHalf);
    let right = place(work, Placement::RightHalf);
    std::thread::sleep(Duration::from_millis(200));
    window_chord(b'A');
    if wait_for(Duration::from_secs(2), || landed(id, left)).is_none() {
        r.check(name, Status::Fail, "no llegó a la mitad izquierda".into());
        return;
    }
    std::thread::sleep(Duration::from_millis(300));
    let expected = wm_model::world::fly_ms(left, right);
    let start = Instant::now();
    window_chord(b'D');
    let mut cloak_at = None;
    let got = wait_for(Duration::from_secs(2), || {
        if cloak_at.is_none() && wm_platform::ghost::is_cloaked(id) {
            cloak_at = Some(start.elapsed());
        }
        landed(id, right)
    });
    // Despegue (unos 4 cuadros) + vuelo + llegada y uncloak (unos 3 cuadros).
    let limit = Duration::from_millis(u64::from(expected) + 250);
    // Redirigir: A a mitad de un vuelo hacia la izquierda... D de nuevo.
    std::thread::sleep(Duration::from_millis(300));
    window_chord(b'A');
    std::thread::sleep(Duration::from_millis(70));
    window_chord(b'D');
    let redirected = wait_for(Duration::from_secs(2), || landed(id, right));
    // Esquina → esquina con las teclas a destiempo (W, y 25 ms después D con
    // W apretada): la primera tecla sola no tiene que mandarla a una mitad.
    let top_left = place(work, Placement::TopLeft);
    let top_right = place(work, Placement::TopRight);
    std::thread::sleep(Duration::from_millis(300));
    session::send_keys(&[(CAPS, true), (ALT, true), (b'W', true), (b'A', true)]);
    session::send_keys(&[(b'A', false), (b'W', false), (ALT, false), (CAPS, false)]);
    let in_corner = wait_for(Duration::from_secs(2), || landed(id, top_left)).is_some();
    std::thread::sleep(Duration::from_millis(300));
    session::send_keys(&[(CAPS, true), (ALT, true), (b'W', true)]);
    std::thread::sleep(Duration::from_millis(25));
    session::send_keys(&[(b'D', true), (b'D', false), (b'W', false), (ALT, false), (CAPS, false)]);
    let mut corner_cloak = false;
    let corner = wait_for(Duration::from_secs(2), || {
        corner_cloak |= wm_platform::ghost::is_cloaked(id);
        landed(id, top_right)
    });
    let corner_ok = in_corner && corner_cloak && corner.is_some();
    let ok =
        cloak_at.is_some() && got.is_some_and(|d| d <= limit) && redirected.is_some() && corner_ok;
    let detail = format!(
        "A → D: cloak {}; aterrizó visible {} (vuelo de {expected} ms, tope {}); D a mitad de un vuelo a la izquierda: de vuelta a la derecha y visible {}",
        cloak_at.map_or_else(|| "NUNCA (¿sin animación?)".into(), |d| format!("a los {}", ms(d))),
        got.map_or_else(|| "NUNCA".into(), |d| format!("a los {}", ms(d))),
        ms(limit),
        redirected.map_or_else(|| "NUNCA".into(), |d| format!("a los {}", ms(d))),
    ) + &format!(
        "; esquina → esquina con W y D a 25 ms: {}",
        match (in_corner, corner_cloak, corner) {
            (false, _, _) => "no llegó a la primera esquina".into(),
            (true, true, Some(d)) => format!("voló, visible a los {}", ms(d)),
            (true, false, Some(_)) => "llegó SIN vuelo".into(),
            (true, _, None) => "NO llegó".into(),
        }
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
}

/// Matar el core en pleno vuelo: la ventana queda con cloak (con nuestra
/// marca) y `restore_all`, lo que hace el guardián al relanzar, la devuelve.
fn fly_crash_check(r: &mut Report, id: WindowId) {
    let name = "vuelo + caída";
    if skip_if_reduced_motion(r, name) {
        return;
    }
    let (mut child, _lines) = match spawn_core() {
        Ok(x) => x,
        Err(e) => {
            r.check(name, Status::Fail, e);
            return;
        }
    };
    let _ = windows_list::activate(id);
    std::thread::sleep(Duration::from_millis(150));
    let Some(frame) = wm_platform::ghost::frame_bounds(id) else {
        let _ = child.kill();
        r.check(name, Status::Fail, "no se pudo leer el marco".into());
        return;
    };
    let monitors = wm_platform::monitors();
    let Some(mon) = monitors.iter().find(|m| {
        let (cx, cy) = ((frame.left + frame.right) / 2, (frame.top + frame.bottom) / 2);
        cx >= m.bounds.left && cx < m.bounds.right && cy >= m.bounds.top && cy < m.bounds.bottom
    }) else {
        let _ = child.kill();
        r.check(name, Status::Fail, "no hay monitores".into());
        return;
    };
    let left = place(mon.work_area, Placement::LeftHalf);
    window_chord(b'A');
    if wait_for(Duration::from_secs(2), || landed(id, left)).is_none() {
        let _ = child.kill();
        r.check(name, Status::Fail, "no llegó a la mitad izquierda".into());
        return;
    }
    std::thread::sleep(Duration::from_millis(300));
    window_chord(b'D');
    let cloaked =
        wait_for(Duration::from_millis(500), || wm_platform::ghost::is_cloaked(id)).is_some();
    let _ = child.kill();
    let _ = child.wait();
    let marked = hide::hidden_with(id).is_some();
    let report = hide::restore_all();
    let back = wait_for(Duration::from_secs(1), || {
        !wm_platform::ghost::is_cloaked(id) && hide::fully_visible(hide::visibility(id))
    });
    let ok = cloaked && back.is_some();
    let detail = format!(
        "core matado en pleno vuelo (cloak: {}; con marca: {}); restore_all mostró {} y la ventana {}",
        if cloaked { "sí" } else { "NO, ¿sin animación?" },
        if marked { "sí" } else { "no" },
        report.restored.len(),
        back.map_or_else(|| "SIGUE ESCONDIDA".into(), |d| format!("se ve a los {}", ms(d))),
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
}

/// F (maximizar y volver), C (centrar) y Z (deshacer) con el core real. Parte
/// de la esquina de abajo a la derecha y termina ahí.
fn fcz_checks(r: &mut Report, id: WindowId, work: wm_model::Rect) {
    use wm_model::place::centered;
    use wm_model::{Placement, place};
    const CAPS: u8 = 0x14;
    const ALT: u8 = 0xA4;
    let corner = place(work, Placement::BottomRight);
    let center = centered(work, corner.width(), corner.height());
    let maximized = move || wm_platform::hide::is_maximized(id);
    let at =
        |target: wm_model::Rect| move || !wm_platform::hide::is_maximized(id) && landed(id, target);
    type Done = Box<dyn Fn() -> bool>;
    let steps: [(u8, &str, Done); 6] = [
        (b'F', "F → maximizada", Box::new(maximized)),
        (b'F', "F → vuelve", Box::new(at(corner))),
        (b'C', "C → centrada", Box::new(at(center))),
        (b'Z', "Z → deshace C", Box::new(at(corner))),
        (b'Z', "Z → deshace F", Box::new(maximized)),
        (b'Z', "Z → deshace F", Box::new(at(corner))),
    ];
    let mut parts = Vec::new();
    let mut all_ok = true;
    for (k, label, done) in steps {
        let start = Instant::now();
        session::send_keys(&[
            (CAPS, true),
            (ALT, true),
            (k, true),
            (k, false),
            (ALT, false),
            (CAPS, false),
        ]);
        let reached = loop {
            if done() {
                break Some(start.elapsed());
            }
            if start.elapsed() > Duration::from_secs(2) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        all_ok &= reached.is_some();
        parts.push(match reached {
            Some(d) => format!("{label} en {}", ms(d)),
            None => format!("{label}: NO"),
        });
        std::thread::sleep(Duration::from_millis(250));
    }
    r.check("F, C y Z", if all_ok { Status::Ok } else { Status::Fail }, parts.join("; "));
}

const CAPS: u8 = 0x14;
const ALT: u8 = 0xA4;

fn window_chord(k: u8) {
    session::send_keys(&[
        (CAPS, true),
        (ALT, true),
        (k, true),
        (k, false),
        (ALT, false),
        (CAPS, false),
    ]);
}

/// Espera hasta `limit` a que `done` dé verdadero; devuelve cuánto tardó.
fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> Option<Duration> {
    let start = Instant::now();
    loop {
        if done() {
            return Some(start.elapsed());
        }
        if start.elapsed() > limit {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Cada títere difícil, con el core real: `Caps` + Alt + D (mitad derecha).
fn hard_checks(r: &mut Report) {
    use wm_model::place::{MonitorObs, anchor, edge_error, monitor_of};
    use wm_model::{Placement, place};
    use wm_platform::puppet::{HardKind, RELOCATE_BY, find_hard_puppet};
    let name = "ventanas difíciles";
    let Ok(exe) = std::env::current_exe() else {
        r.check(name, Status::Fail, "no se encontró el ejecutable".into());
        return;
    };
    let frame = |id| wm_platform::ghost::frame_bounds(id);
    let mut parts = Vec::new();
    let mut all_ok = true;
    for kind in HardKind::ALL {
        let Ok(mut child) = Command::new(&exe)
            .args(["puppet-hard", kind.name()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            all_ok = false;
            parts.push(format!("{}: no se pudo abrir", kind.name()));
            continue;
        };
        let pid = child.id();
        let mut id = None;
        let _ = wait_for(Duration::from_secs(3), || {
            id = find_hard_puppet(pid);
            id.is_some()
        });
        let Some(id) = id else {
            let _ = child.kill();
            all_ok = false;
            parts.push(format!("{}: no apareció", kind.name()));
            continue;
        };
        let _ = windows_list::activate(id);
        std::thread::sleep(Duration::from_millis(250));
        let monitors: Vec<MonitorObs> = wm_platform::monitors()
            .iter()
            .map(|m| MonitorObs { bounds: m.bounds, work: m.work_area, dpi: m.dpi })
            .collect();
        let Some(start_frame) = frame(id) else { continue };
        let Some(m) = monitor_of(&monitors, start_frame) else { continue };
        let work = monitors[m].work;
        let target = place(work, Placement::RightHalf);
        window_chord(b'D');
        let limit = Duration::from_millis(1500);
        let reached = match kind {
            HardKind::MinSize | HardKind::Fixed => wait_for(limit, || {
                frame(id).is_some_and(|f| {
                    f != start_frame
                        && edge_error(f, anchor(target, work, f.width(), f.height())) <= 1
                })
            }),
            HardKind::Relocate => {
                let moved = wm_model::Rect::new(
                    target.left + RELOCATE_BY.0,
                    target.top + RELOCATE_BY.1,
                    target.right + RELOCATE_BY.0,
                    target.bottom + RELOCATE_BY.1,
                );
                // Que llegue a donde la movió la app y se quede (wm no se pelea).
                let got = wait_for(limit, || frame(id).is_some_and(|f| edge_error(f, moved) <= 1));
                std::thread::sleep(Duration::from_millis(400));
                got.filter(|_| frame(id).is_some_and(|f| edge_error(f, moved) <= 1))
            }
            _ => wait_for(limit, || frame(id).is_some_and(|f| edge_error(f, target) <= 1)),
        };
        all_ok &= reached.is_some();
        parts.push(match reached {
            Some(d) => format!("{} en {}", kind.name(), ms(d)),
            None => format!(
                "{}: NO ({:?} → {:?})",
                kind.name(),
                frame(id).map(|f| (f.left, f.top, f.width(), f.height())),
                (target.left, target.top, target.width(), target.height())
            ),
        });
        let _ = child.kill();
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(150));
    }
    r.check(name, if all_ok { Status::Ok } else { Status::Fail }, parts.join("; "));
}

/// Con el títere colgable congelado: `Caps` + Alt + D no traba a wm (un
/// `Caps` + W inmediato cambia de ventana) y la ventana llega a la mitad
/// derecha cuando la app despierta.
fn hung_check(r: &mut Report) {
    use wm_model::place::edge_error;
    use wm_model::{Placement, place};
    let name = "colgada";
    let Ok(exe) = std::env::current_exe() else { return };
    let Ok(mut child) =
        Command::new(&exe).arg("hang-puppet").stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    else {
        r.check(name, Status::Fail, "no se pudo abrir el títere colgable".into());
        return;
    };
    let mut id = None;
    let _ = wait_for(Duration::from_secs(3), || {
        id = puppet::find_hang_puppet();
        id.is_some()
    });
    let Some(id) = id else {
        let _ = child.kill();
        r.check(name, Status::Fail, "el títere colgable no apareció".into());
        return;
    };
    let _ = windows_list::activate(id);
    std::thread::sleep(Duration::from_millis(250));
    let monitors = wm_platform::monitors();
    let Some(primary) = monitors.iter().find(|m| m.primary).or(monitors.first()) else { return };
    let target = place(primary.work_area, Placement::RightHalf);
    let _ = puppet::hang(id, 2500, false);
    std::thread::sleep(Duration::from_millis(100));
    window_chord(b'D');
    std::thread::sleep(Duration::from_millis(30));
    session::send_hyper_chord(b'W'); // otra ventana de wm.exe: el core sigue vivo
    let responsive = wait_for(Duration::from_millis(500), || {
        windows_list::foreground().is_some_and(|fg| fg != id)
    });
    let arrived = wait_for(Duration::from_secs(5), || {
        wm_platform::ghost::frame_bounds(id).is_some_and(|f| edge_error(f, target) <= 1)
    });
    let _ = child.kill();
    let _ = child.wait();
    let ok = responsive.is_some() && arrived.is_some();
    let detail = format!(
        "con la app congelada 2,5 s, Caps + W cambió de ventana {}; la colgada llegó a la mitad derecha {}",
        responsive.map_or_else(|| "NO".into(), |d| format!("en {}", ms(d))),
        arrived.map_or_else(|| "NUNCA".into(), |d| format!("a los {}", ms(d))),
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
}

/// Con dos monitores o más: llevar la ventana a la mitad del lado que tiene
/// vecino, repetir para empujarla y volver.
fn push_checks(r: &mut Report, id: WindowId, monitors: &[wm_model::place::MonitorObs], m: usize) {
    use wm_model::place::{Dir, neighbor};
    use wm_model::{Placement, place};
    const CAPS: u8 = 0x14;
    const ALT: u8 = 0xA4;
    let (dir, key, back_key, half, other_half) = if neighbor(monitors, m, Dir::Right).is_some() {
        (Dir::Right, b'D', b'A', Placement::RightHalf, Placement::LeftHalf)
    } else if neighbor(monitors, m, Dir::Left).is_some() {
        (Dir::Left, b'A', b'D', Placement::LeftHalf, Placement::RightHalf)
    } else {
        r.check("empujar", Status::Skip, format!("{} monitor: no hay vecino", monitors.len()));
        return;
    };
    let n = neighbor(monitors, m, dir).expect("recién visto");
    // Pasado el tiempo de combinación, para que la primera tecla sea una mitad.
    std::thread::sleep(Duration::from_millis(200));
    // Desde la esquina de abajo a la derecha (la deja `arrange_checks`), D da
    // la mitad derecha. De ahí en adelante, lado opuesto → mitad → empujar →
    // volver → seguir dentro del monitor (plan-core.md §3.1).
    let steps = [
        (b'D', place(monitors[m].work, Placement::RightHalf), "D → mitad"),
        (back_key, place(monitors[m].work, other_half), "al otro lado"),
        (key, place(monitors[m].work, half), "mitad"),
        (key, place(monitors[n].work, other_half), "empujada al vecino"),
        (back_key, place(monitors[m].work, half), "de vuelta"),
        // La misma tecla otra vez: dentro del monitor, no "no hay otro".
        (back_key, place(monitors[m].work, other_half), "y sigue dentro del monitor"),
    ];
    let mut parts = Vec::new();
    let mut all_ok = true;
    for (k, target, label) in steps {
        let start = Instant::now();
        session::send_keys(&[
            (CAPS, true),
            (ALT, true),
            (k, true),
            (k, false),
            (ALT, false),
            (CAPS, false),
        ]);
        let reached = loop {
            if landed(id, target) {
                break Some(start.elapsed());
            }
            if start.elapsed() > Duration::from_secs(1) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        all_ok &= reached.is_some();
        parts.push(match reached {
            Some(d) => format!("{label} en {}", ms(d)),
            None => format!("{label}: NO llegó"),
        });
        std::thread::sleep(Duration::from_millis(150));
    }
    let (a, b) = (monitors[m].dpi, monitors[n].dpi);
    parts.push(format!("DPI {a} → {b}"));
    r.check("empujar", if all_ok { Status::Ok } else { Status::Fail }, parts.join("; "));
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
