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
//!     devuelve;
//! 13. **G y V** (hito 1f): desde la mitad izquierda, G la agranda un 10 %
//!     del área pegada al borde y V la devuelve a la mitad;
//! 14. **mapa** (hito 2a): mantener Caps lo muestra a ~400 ms y soltar lo
//!     oculta; un toque y un atajo rápido no lo muestran; Bloq Mayús intacto.
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
const TOTAL: Duration = Duration::from_secs(150);
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
    /// Falló un chequeo que depende de tomar el primer plano en un runner
    /// que no lo garantiza (ARM64, architecture.md §13.11): se informa, no
    /// cuenta como falla.
    Flaky,
}

/// Chequeos que dependen de que la sonda pueda tomar el primer plano.
const FOCUS_CHECKS: &[&str] = &[
    "foco",
    "capa de ventana",
    "mapa",
    "bandeja",
    "recarga",
    "opciones y avisos",
    "ventanas difíciles",
    "colgada",
    "escritorios",
    "pantallas",
    "vuelo",
    "G y V",
    "vuelo + caída",
    "arrastre",
];

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::Fail => "FALLA",
            Status::Skip => "SIN PROBAR",
            Status::Flaky => "INESTABLE",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Status::Ok => "✅",
            Status::Fail => "❌",
            Status::Skip => "➖",
            Status::Flaky => "⚠️",
        }
    }
}

struct Report {
    env: Vec<(String, String)>,
    checks: Vec<(String, Status, String)>,
    /// `--focus-informative` (el job de ARM64): las fallas de los chequeos
    /// de [`FOCUS_CHECKS`] salen como INESTABLE.
    focus_informative: bool,
}

impl Report {
    fn env(&mut self, key: &str, value: String) {
        out!("  {key}: {value}");
        self.env.push((key.to_owned(), value));
    }

    fn check(&mut self, name: &str, status: Status, detail: String) {
        let status =
            if status == Status::Fail && self.focus_informative && FOCUS_CHECKS.contains(&name) {
                Status::Flaky
            } else {
                status
            };
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

    // Con un `wm` del usuario abierto, el `restore_all` de la sonda le
    // mostraría las ventanas de otros escritorios y las dos instancias
    // recibirían las mismas teclas.
    if wm_platform::tray::user_instance_running() {
        eprintln!("ci-probe: hay un wm abierto; cerralo (Salir en la bandeja) y volvé a correrlo.");
        return ExitCode::from(2);
    }
    let cleanup = Cleanup;
    let focus_informative = args.iter().any(|a| a == "--focus-informative");
    let mut r = Report { env: Vec::new(), checks: Vec::new(), focus_informative };
    out!("Entorno");
    environment(&mut r);

    out!("\nChequeos");
    let puppets = open_puppets(&mut r);
    if puppets.len() >= PUPPETS {
        focus(&mut r, &puppets);
        core(&mut r, &puppets);
        fly_crash_check(&mut r, puppets[1]);
        recovery_check(&mut r, &puppets);
        hook(&mut r, puppets[0]);
        hiding(&mut r, puppets[1]);
        batch_check(&mut r, &puppets[..3]);
        frame(&mut r, puppets[2]);
    } else {
        for name in [
            "foco",
            "wm core",
            "capa de ventana",
            "F, C y Z",
            "empujar",
            "G y V",
            "ventanas difíciles",
            "colgada",
            "mapa",
            "bandeja",
            "recarga",
            "vuelo",
            "vuelo + caída",
            "recuperación",
            "hook + SendInput",
            "esconder",
            "lotes",
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
    let flaky = r.checks.iter().filter(|(_, s, _)| *s == Status::Flaky).count();
    if flaky > 0 {
        out!("\n{ok} OK, {failed} con fallas, {flaky} inestables (foco), de {}", r.checks.len());
    } else {
        out!("\n{ok} OK, {failed} con fallas, de {}", r.checks.len());
    }
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
/// Hito 2a: el mapa de teclado con el core real. Mantener Caps 600 ms lo
/// muestra (a los ~400 ms) y soltar lo oculta, sin tocar Bloq Mayús; un
/// toque no lo muestra (y sí cambia Bloq Mayús); un atajo rápido lo suprime
/// toda la pulsación; con el mapa visible, una acción no lo cierra.
/// Hito 3b: órdenes de la bandeja con el core real (por `wm.command`, como
/// el menú). En pausa, `Caps` + W no cambia de ventana y un toque de `Caps`
/// cambia Bloq Mayús; al reanudar, `Caps` + W vuelve a funcionar.
fn tray_checks(r: &mut Report, puppets: &[WindowId]) {
    use wm_platform::tray::{Order, send_order_in};
    let name = "bandeja";
    let order = |o| send_order_in("ci-probe", o);
    let focus_moves = |from: WindowId| {
        let _ = windows_list::activate(from);
        std::thread::sleep(Duration::from_millis(150));
        session::send_hyper_chord(b'W');
        wait_for(Duration::from_millis(600), || {
            windows_list::foreground().is_some_and(|fg| fg != from && puppets.contains(&fg))
        })
        .is_some()
    };
    if !order(Order::Pause) {
        r.check(name, Status::Fail, "no se encontró la ventana wm-ui".into());
        return;
    }
    std::thread::sleep(Duration::from_millis(200));
    let moved_paused = focus_moves(puppets[0]);
    let before = session::caps_lock_on();
    session::send_keys(&[(CAPS, true), (CAPS, false)]);
    std::thread::sleep(Duration::from_millis(100));
    let toggled = session::caps_lock_on() != before;
    if toggled {
        session::send_keys(&[(CAPS, true), (CAPS, false)]); // dejarlo como estaba
        std::thread::sleep(Duration::from_millis(100));
    }
    let resumed = order(Order::Resume);
    std::thread::sleep(Duration::from_millis(200));
    let moved_after = focus_moves(puppets[0]);
    let ok = !moved_paused && toggled && resumed && moved_after;
    let detail = format!(
        "en pausa: Caps + W {}, toque de Caps {}; reanudado: Caps + W {}",
        if moved_paused { "CAMBIÓ de ventana" } else { "no hace nada" },
        if toggled { "cambia Bloq Mayús" } else { "NO cambia Bloq Mayús" },
        if moved_after { "cambia de ventana" } else { "NO funciona" },
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
}

fn map_checks(r: &mut Report, core_pid: u32) {
    use wm_platform::ui::map_visible_in;
    let name = "mapa";
    if std::env::var_os("WM_MAP").is_some_and(|v| v == "0") {
        r.check(name, Status::Skip, "WM_MAP=0".into());
        return;
    }
    let visible = || map_visible_in(core_pid);
    let mut parts = Vec::new();
    let mut ok = true;
    let caps_before = session::caps_lock_on();

    // 1. Mantener: aparece a ~400 ms y se oculta al soltar.
    std::thread::sleep(Duration::from_millis(300));
    session::send_keys(&[(CAPS, true)]);
    let start = Instant::now();
    let shown = wait_for(Duration::from_millis(900), visible);
    let held = start.elapsed();
    if held < Duration::from_millis(600) {
        std::thread::sleep(Duration::from_millis(600) - held);
    }
    let still = visible();
    session::send_keys(&[(CAPS, false)]);
    let hidden = wait_for(Duration::from_millis(300), || !visible());
    let in_time =
        shown.is_some_and(|d| d >= Duration::from_millis(380) && d <= Duration::from_millis(520));
    let kept_caps = session::caps_lock_on() == caps_before;
    ok &= in_time && still && hidden.is_some() && kept_caps;
    parts.push(format!(
        "Caps 600 ms → visible {}; al soltar se oculta {}; Bloq Mayús {}",
        shown.map_or_else(|| "NUNCA".into(), |d| format!("a los {}", ms(d))),
        hidden.map_or_else(|| "NO".into(), |d| format!("en {}", ms(d))),
        if kept_caps { "igual" } else { "CAMBIÓ" },
    ));

    // 2. Toque: no aparece y Bloq Mayús cambia (se restaura).
    std::thread::sleep(Duration::from_millis(200));
    let before = session::caps_lock_on();
    session::send_keys(&[(CAPS, true)]);
    std::thread::sleep(Duration::from_millis(80));
    session::send_keys(&[(CAPS, false)]);
    let appeared = wait_for(Duration::from_millis(700), visible).is_some();
    std::thread::sleep(Duration::from_millis(50));
    let toggled = session::caps_lock_on() != before;
    if toggled {
        session::send_keys(&[(CAPS, true), (CAPS, false)]); // dejarlo como estaba
        std::thread::sleep(Duration::from_millis(100));
    }
    ok &= !appeared && toggled;
    parts.push(format!(
        "toque → {}, Bloq Mayús {}",
        if appeared { "APARECIÓ" } else { "sin mapa" },
        if toggled { "cambió" } else { "NO cambió" },
    ));

    // 3. Atajo rápido (Caps + Q a los 50 ms, Caps hasta 800 ms): nunca aparece.
    std::thread::sleep(Duration::from_millis(200));
    session::send_keys(&[(CAPS, true)]);
    std::thread::sleep(Duration::from_millis(50));
    session::send_keys(&[(b'Q', true), (b'Q', false)]);
    let appeared = wait_for(Duration::from_millis(750), visible).is_some();
    session::send_keys(&[(CAPS, false)]);
    ok &= !appeared;
    parts.push(format!("Caps + Q rápido → {}", if appeared { "APARECIÓ" } else { "sin mapa" }));

    // 4. Con el mapa visible, una acción no lo cierra (D7).
    std::thread::sleep(Duration::from_millis(200));
    session::send_keys(&[(CAPS, true)]);
    let shown = wait_for(Duration::from_millis(900), visible).is_some();
    session::send_keys(&[(b'Q', true), (b'Q', false)]);
    std::thread::sleep(Duration::from_millis(150));
    let kept = visible();
    session::send_keys(&[(CAPS, false)]);
    let gone = wait_for(Duration::from_millis(300), || !visible()).is_some();
    ok &= shown && kept && gone;
    parts.push(format!(
        "visible + Caps + Q → {}",
        if shown && kept { "sigue visible" } else { "SE CERRÓ" }
    ));

    // 5. Con una app a pantalla completa al frente (un títere sin borde que
    // cubre su monitor), el mapa no aparece.
    if let Some(full) = fullscreen_map_check(core_pid) {
        ok &= full;
        parts.push(format!("pantalla completa → {}", if full { "sin mapa" } else { "APARECIÓ" }));
    }

    r.check(name, if ok { Status::Ok } else { Status::Fail }, parts.join("; "));
}

/// `Some(true)` si el mapa no apareció sobre una app a pantalla completa;
/// `None` si no se pudo armar la prueba.
fn fullscreen_map_check(core_pid: u32) -> Option<bool> {
    use wm_platform::puppet::find_hard_puppet;
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(&exe)
        .args(["puppet-hard", "borderless"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut id = None;
    let _ = wait_for(Duration::from_secs(3), || {
        id = find_hard_puppet(child.id());
        id.is_some()
    });
    let result = id.and_then(|id| {
        let frame = wm_platform::ghost::frame_bounds(id)?;
        let (cx, cy) = ((frame.left + frame.right) / 2, (frame.top + frame.bottom) / 2);
        let m = wm_platform::monitors().into_iter().find(|m| {
            let b = m.bounds;
            cx >= b.left && cx < b.right && cy >= b.top && cy < b.bottom
        })?;
        let _ = windows_list::place_async(id, m.bounds);
        let _ = windows_list::activate(id);
        wait_for(Duration::from_secs(1), || {
            wm_platform::ghost::frame_bounds(id) == Some(m.bounds)
        })?;
        std::thread::sleep(Duration::from_millis(200));
        session::send_keys(&[(CAPS, true)]);
        let appeared =
            wait_for(Duration::from_millis(800), || wm_platform::ui::map_visible_in(core_pid))
                .is_some();
        session::send_keys(&[(CAPS, false)]);
        Some(!appeared)
    });
    let _ = child.kill();
    let _ = child.wait();
    std::thread::sleep(Duration::from_millis(150));
    result
}

/// Lanza `wm core` y espera su banner (plazo 5 s). Las líneas del core
/// siguen llegando por el canal.
fn spawn_core() -> Result<(std::process::Child, std::sync::mpsc::Receiver<String>), String> {
    spawn_core_relaunched(false)
}

/// Como [`spawn_core`]; `relaunched`: como lo relanza el guardián tras una
/// caída (`WM_RELAUNCH=1`, D4).
fn spawn_core_relaunched(
    relaunched: bool,
) -> Result<(std::process::Child, std::sync::mpsc::Receiver<String>), String> {
    use std::io::BufRead;
    let exe = std::env::current_exe().map_err(|_| "no se encontró el ejecutable".to_owned())?;
    let mut cmd = Command::new(exe);
    if relaunched {
        cmd.env("WM_RELAUNCH", "1");
    }
    let mut child = cmd
        .arg("core")
        // Su propia instancia: el usuario puede tener un core abierto.
        .env("WM_INSTANCE", "ci-probe")
        // Sus propias asignaciones: no toca el apps.txt del usuario.
        .env("APPDATA", probe_appdata())
        // Escritorios: solo esconde sus títeres (app "wm"), nunca las
        // ventanas del usuario.
        .env("WM_SCOPE", "wm")
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

/// La carpeta de datos del core de la sonda (vacía al empezar).
fn probe_appdata() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("wm-ci-probe-appdata");
    let apps = dir.join("wm").join("apps.txt");
    let _ = std::fs::create_dir_all(apps.parent().expect("tiene carpeta"));
    let _ = std::fs::remove_file(&apps);
    // Sin avisos para los chequeos del mapa (un aviso usa la misma ventana
    // overlay); "opciones y avisos" los vuelve a prender.
    let _ = std::fs::write(apps.with_file_name("settings.txt"), "avisos = no\n");
    dir
}

/// Hito 3c: guardar `apps.txt` como lo hacen los ajustes (temporal y
/// renombrar) → el core lo recarga solo y `Caps` + la letra nueva funciona.
fn reload_check(r: &mut Report, lines: &std::sync::mpsc::Receiver<String>, puppets: &[WindowId]) {
    let name = "recarga";
    let apps = std::env::temp_dir().join("wm-ci-probe-appdata").join("wm").join("apps.txt");
    while lines.try_recv().is_ok() {}
    let tmp = apps.with_extension("tmp");
    let start = Instant::now();
    let written = std::fs::write(
        &tmp, "q = wm
",
    )
    .and_then(|()| std::fs::rename(&tmp, &apps));
    if let Err(e) = written {
        r.check(name, Status::Fail, format!("no se pudo escribir {}: {e}", apps.display()));
        return;
    }
    let reloaded = loop {
        match lines.recv_timeout(Duration::from_millis(50)) {
            Ok(l) if l.starts_with("apps.txt recargado") => break Some(start.elapsed()),
            Ok(_) => {}
            Err(_) if start.elapsed() > Duration::from_secs(2) => break None,
            Err(_) => {}
        }
    };
    // Q = wm: los títeres son wm.exe; desde uno, va a otro.
    let from = puppets[0];
    let _ = windows_list::activate(from);
    std::thread::sleep(Duration::from_millis(150));
    session::send_hyper_chord(b'Q');
    let focused = wait_for(Duration::from_millis(600), || {
        windows_list::foreground().is_some_and(|fg| fg != from && puppets.contains(&fg))
    });
    let ok = reloaded.is_some() && focused.is_some();
    let detail = format!(
        "apps.txt guardado (temporal + renombrar) → recargado {}; Caps + Q → {}",
        reloaded.map_or_else(|| "NUNCA".into(), |d| format!("a los {}", ms(d))),
        if focused.is_some() { "otro títere (wm)" } else { "NO cambió" },
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
}

/// Opciones (`settings.txt`): se recargan solas; "el puntero sigue al foco"
/// lleva el puntero a la ventana; el borde de foco pinta la del frente; un
/// aviso aparece en pantalla cuando una tecla no hace nada y se va solo.
fn options_check(
    r: &mut Report,
    lines: &std::sync::mpsc::Receiver<String>,
    puppets: &[WindowId],
    core_pid: u32,
) {
    let name = "opciones y avisos";
    let file = std::env::temp_dir().join("wm-ci-probe-appdata").join("wm").join("settings.txt");
    while lines.try_recv().is_ok() {}
    let tmp = file.with_extension("tmp");
    let written = std::fs::write(&tmp, "puntero_sigue_al_foco = sí\navisos = sí\n")
        .and_then(|()| std::fs::rename(&tmp, &file));
    if let Err(e) = written {
        return r.check(name, Status::Fail, format!("no se pudo escribir {}: {e}", file.display()));
    }
    let start = Instant::now();
    let reloaded = loop {
        match lines.recv_timeout(Duration::from_millis(50)) {
            Ok(l) if l.starts_with("Opciones recargadas") => break Some(start.elapsed()),
            Ok(_) => {}
            Err(_) if start.elapsed() > Duration::from_secs(2) => break None,
            Err(_) => {}
        }
    };
    // Puntero lejos; Caps + Q (q = wm, de la recarga) va a otro títere y el
    // puntero tiene que quedar adentro.
    let from = puppets[0];
    let _ = windows_list::activate(from);
    std::thread::sleep(Duration::from_millis(150));
    let monitors = wm_platform::monitors();
    let primary = monitors.iter().find(|m| m.primary).map(|m| m.bounds);
    if let Some(b) = primary {
        session::set_cursor(b.right - 5, b.bottom - 5);
    }
    session::send_hyper_chord(b'Q');
    let inside = |id: WindowId| {
        let Some(f) = wm_platform::ghost::frame_bounds(id) else { return false };
        session::cursor()
            .is_some_and(|(x, y)| x >= f.left && x < f.right && y >= f.top && y < f.bottom)
    };
    let followed = wait_for(Duration::from_millis(800), || {
        windows_list::foreground()
            .is_some_and(|fg| fg != from && puppets.contains(&fg) && inside(fg))
    });
    // El borde propio rodea el marco de la ventana al frente.
    let fg = windows_list::foreground();
    let border = wait_for(Duration::from_millis(500), || {
        let (Some(fg), Some(b)) = (fg, wm_platform::focus_border::shown_rect(core_pid)) else {
            return false;
        };
        wm_platform::ghost::frame_bounds(fg).is_some_and(|f| {
            b.left < f.left
                && b.top < f.top
                && b.right > f.right
                && b.bottom > f.bottom
                && f.left - b.left <= 4
        })
    });
    // Y lo sigue al moverla (WinEvents de su proceso).
    let follows =
        fg.zip(wm_platform::ghost::window_rect(fg.unwrap_or(WindowId(0)))).and_then(|(fg, r)| {
            let moved = wm_model::Rect::new(r.left + 60, r.top + 40, r.right + 60, r.bottom + 40);
            let before = wm_platform::ghost::frame_bounds(fg)?;
            let _ = windows_list::place_async(fg, moved);
            wait_for(Duration::from_millis(600), || {
                let (Some(f), Some(b)) = (
                    wm_platform::ghost::frame_bounds(fg),
                    wm_platform::focus_border::shown_rect(core_pid),
                ) else {
                    return false;
                };
                f.left >= before.left + 50
                    && f.left - b.left <= 4
                    && f.left - b.left > 0
                    && b.top < f.top
                    && f.top - b.top <= 4
            })
        });
    let border_ok = border.is_some() && follows.is_some();
    // Una letra sin ninguna app: aviso en pantalla.
    let used: Vec<char> = windows_list::user_windows()
        .iter()
        .filter_map(|w| w.app.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let free = (b'A'..=b'Z').map(char::from).find(|c| *c != 'Q' && !used.contains(c));
    let notice = free.map(|c| {
        session::send_hyper_chord(c as u8);
        let shown =
            wait_for(Duration::from_millis(800), || wm_platform::ui::map_visible_in(core_pid));
        let gone = shown.and_then(|_| {
            wait_for(Duration::from_secs(4), || !wm_platform::ui::map_visible_in(core_pid))
        });
        (c, shown, gone)
    });
    let notice_ok = notice.is_none_or(|(_, s, g)| s.is_some() && g.is_some());
    let ok = reloaded.is_some() && followed.is_some() && border_ok && notice_ok;
    let detail = format!(
        "settings.txt recargado {}; puntero a la ventana enfocada: {}; borde de foco: {}; aviso: {}",
        reloaded.map_or_else(|| "NUNCA".into(), |d| format!("a los {}", ms(d))),
        if followed.is_some() { "sí" } else { "NO" },
        match (border, follows) {
            (Some(_), Some(d)) => format!("rodea la ventana y la sigue al moverla ({})", ms(d)),
            (Some(_), None) => "rodea la ventana pero NO la sigue".to_owned(),
            _ => "NO".to_owned(),
        },
        match notice {
            None => "sin letra libre para probarlo".to_owned(),
            Some((c, s, g)) => format!(
                "Caps + {c} → {} y se fue {}",
                s.map_or_else(|| "NO apareció".into(), |d| format!("apareció a los {}", ms(d))),
                g.map_or_else(|| "NUNCA".into(), |d| format!("{} después", ms(d)))
            ),
        }
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
    let _ = std::fs::remove_file(&file);
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
    let (mut child, lines) = match spawn_core() {
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

    map_checks(r, child.id());
    tray_checks(r, puppets);
    reload_check(r, &lines, puppets);
    options_check(r, &lines, puppets, child.id());
    hard_checks(r);
    hung_check(r);
    desks_check(r, &lines, puppets);
    screens_check(r, &lines, puppets);
    let drag = drag_check(puppets[1]);

    // "Salir" como en el menú de la bandeja (`Caps` + Esc ya no sale fuera
    // del modo prueba).
    let quit_sent = wm_platform::tray::send_order_in("ci-probe", wm_platform::tray::Order::Quit);
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
    // Salida limpia con una ventana en otro escritorio: todo visible y sin
    // `wm.desk` (hito 4c).
    let clean = wait_for(Duration::from_millis(500), || {
        puppets
            .iter()
            .all(|&p| hide::fully_visible(hide::visibility(p)) && hide::desk_of(p).is_none())
    })
    .is_some();
    // El hook de mouse se puso y se quitó la misma cantidad de veces (el
    // resumen llega por la salida del core al terminar).
    std::thread::sleep(Duration::from_millis(150));
    let hooks = lines.try_iter().find_map(|l| {
        let rest = l.split("Hook de mouse: puesto ").nth(1)?.to_owned();
        let (a, b) = rest.split_once(" veces, quitado ")?;
        Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?))
    });
    drag(r, hooks);
    let ok = moved.is_some() && code == Some(0) && clean;
    let detail = format!(
        "arrancó en {}; Caps + W → otro títere: {}; Salir → {}; todo visible y sin wm.desk: {clean}",
        ms(boot),
        moved.map_or_else(|| "no".into(), |d| format!("sí, en {}", ms(d))),
        if quit_sent {
            code.map_or_else(|| "no salió en 3 s".into(), |c| format!("código {c}"))
        } else {
            "no se encontró la ventana wm-ui".into()
        },
    );
    r.check(name, if ok { Status::Ok } else { Status::Fail }, detail);
}

/// Hito 4a: llevar, ir y deshacer con el core real y teclas inyectadas.
/// Hito 4d: avisos de pantallas falsos, posteados solo a la ventana de
/// avisos del core de la sonda (nada cambia en Windows). Títeres corridos
/// fuera de pantalla (uno visible, otro escondido en el escritorio 2) → tras
/// la calma vuelven, sin cambiar de tamaño, y la escondida sigue escondida;
/// con la sesión "bloqueada" no se mueve nada hasta desbloquear.
fn screens_check(r: &mut Report, lines: &std::sync::mpsc::Receiver<String>, puppets: &[WindowId]) {
    use wm_model::screens::{min_visible, reachable};
    use wm_platform::display_watch::{
        MSG_DISPLAYCHANGE, MSG_SESSION, SESSION_LOCK, SESSION_UNLOCK, poke,
    };
    const NAME: &str = "pantallas";
    if puppets.len() < 3 {
        return r.check(NAME, Status::Skip, "hacen falta 3 títeres".into());
    }
    let Ok(monitors) = wm_platform::displays::topology() else {
        return r.check(NAME, Status::Skip, "no pude leer las pantallas".into());
    };
    let areas: Vec<wm_model::Rect> = monitors.iter().map(|m| m.work).collect();
    let min = min_visible(&monitors);
    let on = |id: WindowId| {
        wm_platform::ghost::frame_bounds(id).is_some_and(|f| reachable(f, &areas, min))
    };
    let far_left = monitors.iter().map(|m| m.bounds.left).min().unwrap_or(0) - 5000;
    let away = |id: WindowId| {
        let _ = windows_list::move_async(id, far_left, 200);
    };
    let size = |id: WindowId| wm_platform::ghost::frame_bounds(id).map(|f| (f.width(), f.height()));
    let hidden = |id: WindowId| hide::visibility(id).cloaked == hide::DWM_CLOAKED_SHELL;
    let instance = Some("ci-probe");
    let (p, q, still) = (puppets[0], puppets[1], puppets[2]);
    if !reset_desks(puppets) {
        return r.check(
            NAME,
            Status::Fail,
            "no se pudo empezar con todo en el escritorio 1".into(),
        );
    }

    // Q al escritorio 2 (con cloak).
    let _ = windows_list::activate(q);
    std::thread::sleep(Duration::from_millis(100));
    window_chord(b'2');
    if wait_for(Duration::from_secs(1), || hidden(q)).is_none() {
        return r.check(NAME, Status::Fail, "no se pudo llevar un títere al escritorio 2".into());
    }
    let before = (size(p), size(q), wm_platform::ghost::frame_bounds(still));
    let home: Vec<(WindowId, wm_model::Rect)> = [p, q]
        .iter()
        .filter_map(|&id| wm_platform::ghost::window_rect(id).map(|r| (id, r)))
        .collect();
    away(p);
    away(q);
    if wait_for(Duration::from_millis(500), || !on(p) && !on(q)).is_none() {
        return r.check(NAME, Status::Fail, "no se pudieron correr los títeres".into());
    }
    while lines.try_recv().is_ok() {}
    let mut poked = true;
    for _ in 0..5 {
        poked &= poke(instance, MSG_DISPLAYCHANGE, 0);
        std::thread::sleep(Duration::from_millis(100));
    }
    if !poked {
        return r.check(NAME, Status::Fail, "no encontré la ventana de avisos del core".into());
    }
    let last = Instant::now();
    std::thread::sleep(Duration::from_millis(250));
    let early = on(p) || on(q);
    // Vuelven enteras a una pantalla, no solo "alcanzables".
    let whole = |id: WindowId| {
        wm_platform::ghost::frame_bounds(id).is_some_and(|f| {
            areas.iter().any(|a| {
                f.left >= a.left && f.top >= a.top && f.right <= a.right && f.bottom <= a.bottom
            })
        })
    };
    let back = wait_for(Duration::from_secs(3), || whole(p) && whole(q)).map(|_| last.elapsed());
    let still_hidden = hidden(q);
    let sizes_kept = (size(p), size(q)) == (before.0, before.1);
    let untouched = wm_platform::ghost::frame_bounds(still) == before.2;
    std::thread::sleep(Duration::from_millis(100));
    let calm: Vec<String> =
        lines.try_iter().filter(|l| l.starts_with("Pantallas: en calma")).collect();

    // Bloqueo simulado: nada se mueve hasta desbloquear.
    let _ = poke(instance, MSG_SESSION, SESSION_LOCK);
    std::thread::sleep(Duration::from_millis(100));
    away(p);
    let _ = poke(instance, MSG_DISPLAYCHANGE, 0);
    std::thread::sleep(Duration::from_millis(1500));
    let waited_locked = !on(p);
    let _ = poke(instance, MSG_SESSION, SESSION_UNLOCK);
    let unlocked = wait_for(Duration::from_secs(3), || on(p));

    // "Mostrar todas" trae a Q al escritorio activo.
    let shown_all = wm_platform::tray::send_order_in("ci-probe", wm_platform::tray::Order::ShowAll)
        && wait_for(Duration::from_secs(2), || !hidden(q) && on(q)).is_some();

    // Los títeres vuelven a donde estaban (los chequeos siguientes los
    // quieren enteros en pantalla).
    for (id, r) in &home {
        let _ = windows_list::move_async(*id, r.left, r.top);
    }
    let _ = wait_for(Duration::from_millis(500), || {
        home.iter().all(|(id, r)| wm_platform::ghost::window_rect(*id) == Some(*r))
    });
    let ok = !early
        && back.is_some()
        && still_hidden
        && sizes_kept
        && untouched
        && calm.len() == 1
        && waited_locked
        && unlocked.is_some()
        && shown_all;
    let b = |v: bool| if v { "sí" } else { "NO" };
    let detail = format!(
        "5 avisos falsos → antes de la calma se movió algo: {}; volvieron enteras {}; la escondida sigue escondida: {}; \
         mismo tamaño: {}; la otra quieta: {}; líneas de calma: {}; bloqueada sin mover: {}; \
         al desbloquear volvió {}; Mostrar todas la trae: {}",
        if early { "SÍ" } else { "no" },
        back.map_or_else(|| "NUNCA".into(), |d| format!("a los {} del último aviso", ms(d))),
        b(still_hidden),
        b(sizes_kept),
        b(untouched),
        calm.len(),
        b(waited_locked),
        unlocked.map_or_else(|| "NUNCA".into(), |d| format!("a los {}", ms(d))),
        b(shown_all),
    );
    r.check(NAME, if ok { Status::Ok } else { Status::Fail }, detail);
}

/// Escritorios en un estado conocido: todas las ventanas de la sonda en el
/// 1, visibles y sin marca ("Mostrar todas" desde el 1). Cada paso del
/// chequeo de escritorios (y "pantallas") parte de acá: una falla ya no
/// arrastra a las siguientes (Fable, diagnóstico de la CI).
fn reset_desks(puppets: &[WindowId]) -> bool {
    const CAPS: u8 = 0x14;
    session::send_keys(&[(CAPS, true), (b'1', true), (b'1', false), (CAPS, false)]);
    std::thread::sleep(Duration::from_millis(150));
    let sent = wm_platform::tray::send_order_in("ci-probe", wm_platform::tray::Order::ShowAll);
    sent && wait_for(Duration::from_secs(2), || {
        puppets.iter().all(|&id| {
            hide::fully_visible(hide::visibility(id))
                && hide::hidden_with(id).is_none()
                && hide::desk_of(id).is_none_or(|d| d == 1)
        })
    })
    .is_some()
}

/// Espera la línea del core que cierra una transacción de escritorios
/// ("… ms desde la tecla"), para verificar el estado ya terminado.
fn wait_desk_line(lines: &std::sync::mpsc::Receiver<String>, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Ok(l) = lines.recv_timeout(Duration::from_millis(20))
            && l.contains("ms desde la tecla")
            && (l.contains("escritorio") || l.contains("Mostrar todas"))
        {
            return true;
        }
    }
    false
}

fn desks_check(r: &mut Report, lines: &std::sync::mpsc::Receiver<String>, puppets: &[WindowId]) {
    const NAME: &str = "escritorios";
    const CAPS: u8 = 0x14;
    const ALT: u8 = 0xA4;
    let go = |digit: u8| {
        while lines.try_recv().is_ok() {}
        session::send_keys(&[(CAPS, true), (digit, true), (digit, false), (CAPS, false)]);
    };
    let carry = |key: u8| {
        while lines.try_recv().is_ok() {}
        session::send_keys(&[
            (CAPS, true),
            (ALT, true),
            (key, true),
            (key, false),
            (ALT, false),
            (CAPS, false),
        ]);
    };
    let hidden = |id: WindowId| hide::visibility(id).cloaked == hide::DWM_CLOAKED_SHELL;
    let shown = |id: WindowId| hide::fully_visible(hide::visibility(id));
    let limit = Duration::from_secs(1);
    let (a, others) = (puppets[0], &puppets[1..]);
    let state = || {
        puppets
            .iter()
            .map(|&id| {
                let v = hide::visibility(id);
                format!(
                    "{}{}",
                    if v.cloaked == hide::DWM_CLOAKED_SHELL { "esc" } else { "vis" },
                    hide::desk_of(id).map_or_else(String::new, |d| format!("@{d}"))
                )
            })
            .collect::<Vec<_>>()
            .join("/")
    };
    let mut parts = Vec::new();
    let mut ok = true;
    let step = |label: &str, t: Option<Duration>, parts: &mut Vec<String>, ok: &mut bool| {
        *ok &= t.is_some();
        parts.push(format!("{label}: {}", t.map_or_else(|| "NO".into(), ms)));
    };
    if !reset_desks(puppets) {
        return r.check(NAME, Status::Fail, format!("no se pudo empezar en el 1 ({})", state()));
    }
    // `a` al escritorio 2 desde el 1 (estado de partida de varios pasos).
    let carry_a = || {
        let _ = windows_list::activate(a);
        std::thread::sleep(Duration::from_millis(100));
        carry(b'2');
        let t = wait_for(limit, || {
            hidden(a)
                && hide::desk_of(a) == Some(2)
                && windows_list::foreground().is_some_and(|f| others.contains(&f))
        });
        let _ = wait_desk_line(lines, Duration::from_millis(200));
        t
    };

    let t = carry_a();
    step("Caps + Alt + 2 lleva", t, &mut parts, &mut ok);

    go(b'2');
    let t = wait_for(limit, || {
        shown(a) && windows_list::foreground() == Some(a) && others.iter().all(|&o| hidden(o))
    });
    step("Caps + 2", t, &mut parts, &mut ok);

    go(b'1');
    let t = wait_for(limit, || hidden(a) && others.iter().all(|&o| shown(o)));
    step("Caps + 1", t, &mut parts, &mut ok);

    // Alternar y medir hasta el estado verificado (con la línea de fin de
    // la transacción); 60 ms de calma entre cambios, fuera de la medición.
    let mut times = Vec::new();
    let mut failed_at = None;
    for i in 0..20 {
        std::thread::sleep(Duration::from_millis(60));
        let two = i % 2 == 0;
        go(if two { b'2' } else { b'1' });
        let t = wait_for(limit, || {
            if two {
                shown(a) && others.iter().all(|&o| hidden(o))
            } else {
                hidden(a) && others.iter().all(|&o| shown(o))
            }
        });
        let _ = wait_desk_line(lines, Duration::from_millis(200));
        match t {
            Some(d) => times.push(d),
            None => {
                failed_at = Some((i + 1, state()));
                break;
            }
        }
    }
    times.sort();
    match &failed_at {
        None => {
            if let Some(p95) = times.get(times.len() * 95 / 100).or(times.last()) {
                parts.push(format!("20 cambios: p95 {}", ms(*p95)));
            }
        }
        Some((n, st)) => {
            ok = false;
            parts.push(format!("20 cambios: falló el cambio {n} ({st})"));
            // Volver al estado de partida para que lo que sigue valga.
            if !reset_desks(puppets) || carry_a().is_none() {
                return r.check(NAME, Status::Fail, parts.join("; "));
            }
        }
    }

    // Deshacer el "llevar" (estamos en el 1).
    carry(b'Z');
    let t = wait_for(limit, || shown(a) && hide::desk_of(a) == Some(1));
    let _ = wait_desk_line(lines, Duration::from_millis(200));
    step("Caps + Alt + Z la trae", t, &mut parts, &mut ok);

    // "Mostrar todas las ventanas" (menú de la bandeja o `wm mostrar`).
    let _ = windows_list::activate(a);
    std::thread::sleep(Duration::from_millis(100));
    carry(b'3');
    let carried = wait_for(limit, || hidden(a)).is_some();
    let _ = wait_desk_line(lines, Duration::from_millis(200));
    let sent = wm_platform::tray::send_order_in("ci-probe", wm_platform::tray::Order::ShowAll);
    let t = (carried && sent)
        .then(|| wait_for(limit, || shown(a) && hide::desk_of(a) == Some(1)))
        .flatten();
    step("Mostrar todas", t, &mut parts, &mut ok);
    if t.is_none() {
        parts.push(format!(
            "(llevada al 3: {carried}; orden enviada: {sent}; estado: {}, al frente {})",
            state(),
            windows_list::foreground() == Some(a)
        ));
        if !reset_desks(puppets) {
            return r.check(NAME, Status::Fail, parts.join("; "));
        }
    }

    // Como el clic en la barra de tareas: activar una escondida salta a su
    // escritorio (Windows la destapa y avisa FOREGROUND).
    let carried = carry_a().is_some();
    let _ = windows_list::activate(a);
    let t = carried
        .then(|| {
            wait_for(limit, || {
                shown(a)
                    && windows_list::foreground() == Some(a)
                    && others.iter().all(|&o| hidden(o))
            })
        })
        .flatten();
    let _ = wait_desk_line(lines, Duration::from_millis(200));
    step("activar una escondida salta", t, &mut parts, &mut ok);
    // Queda en el 2 a propósito: "Salir" tiene que mostrar todo y borrar
    // `wm.desk` (lo revisa el chequeo "wm core").
    go(b'1');
    let t = wait_for(limit, || hidden(a) && others.iter().all(|&o| shown(o)));
    step("y Caps + 1 vuelve", t, &mut parts, &mut ok);
    if !ok {
        parts.push(format!("(al final: {})", state()));
    }

    r.check(NAME, if ok { Status::Ok } else { Status::Fail }, parts.join("; "));
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
    resize_check(r, id, monitors[m].work);
}

/// Hito 1f: desde la mitad izquierda, G la agranda un 10 % del área pegada
/// al borde izquierdo y V la devuelve a la mitad.
fn resize_check(r: &mut Report, id: WindowId, work: wm_model::Rect) {
    let name = "G y V";
    let half = place(work, Placement::LeftHalf);
    let wider =
        wm_model::place::anchor(half, work, half.width() + work.width() / 10, half.height());
    std::thread::sleep(Duration::from_millis(300));
    window_chord(b'A');
    if wait_for(Duration::from_secs(2), || landed(id, half)).is_none() {
        r.check(name, Status::Fail, "no llegó a la mitad izquierda".into());
        return;
    }
    let mut parts = Vec::new();
    let mut ok = true;
    for (k, target, label) in [(b'G', wider, "G → 60 % de ancho"), (b'V', half, "V → la mitad")]
    {
        std::thread::sleep(Duration::from_millis(300));
        window_chord(k);
        let got = wait_for(Duration::from_secs(2), || landed(id, target));
        ok &= got.is_some();
        parts.push(match got {
            Some(d) => format!("{label} en {}", ms(d)),
            None => format!("{label}: NO"),
        });
    }
    r.check(name, if ok { Status::Ok } else { Status::Fail }, parts.join("; "));
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

/// Hito 5: `Caps` + arrastrar mueve, `Caps` + clic derecho + arrastrar cambia
/// el tamaño desde la esquina más cercana, Bloq Mayús no cambia y Z deshace.
/// Devuelve una función que informa el chequeo cuando se sabe si el hook de
/// mouse quedó equilibrado (después de "Salir").
fn drag_check(id: WindowId) -> impl FnOnce(&mut Report, Option<(u32, u32)>) {
    const NAME: &str = "arrastre";
    const CAPS: u8 = 0x14;
    const ALT: u8 = 0xA4;
    let mut parts: Vec<String> = Vec::new();
    let mut ok = true;
    let mut ran = true;
    let monitors = wm_platform::monitors();
    let work = monitors.iter().find(|m| m.primary).map(|m| m.work_area);
    let frame_now = || wm_platform::ghost::frame_bounds(id);
    if let (Some(work), Some(_)) = (work, frame_now()) {
        let _ = windows_list::activate(id);
        let start =
            wm_model::Rect::new(work.left + 200, work.top + 150, work.left + 900, work.top + 650);
        let _ = windows_list::place_async(id, start);
        let _ = wait_for(Duration::from_secs(1), || {
            frame_now().is_some_and(|f| f.left != 0 && (f.width() - start.width()).abs() < 40)
        });
        std::thread::sleep(Duration::from_millis(100));
        let f0 = frame_now().unwrap_or(start);
        let caps_before = session::caps_lock_on();
        let drag = |right: bool, from: (i32, i32), by: (i32, i32)| {
            // Solo si bajo el cursor está el títere: nunca arrastrar una
            // ventana del usuario.
            // (durante el vuelo de Z tiene cloak y ahí se ve la de atrás).
            if wait_for(Duration::from_millis(600), || {
                windows_list::root_at(from.0, from.1) == Some(id)
            })
            .is_none()
            {
                return 0;
            }
            session::set_cursor(from.0, from.1);
            session::send_keys(&[(CAPS, true)]);
            std::thread::sleep(Duration::from_millis(40));
            let sent = session::send_mouse_button(right, true);
            for i in 1..=12 {
                std::thread::sleep(Duration::from_millis(8));
                session::set_cursor(from.0 + by.0 * i / 12, from.1 + by.1 * i / 12);
            }
            std::thread::sleep(Duration::from_millis(60));
            session::send_mouse_button(right, false);
            std::thread::sleep(Duration::from_millis(20));
            session::send_keys(&[(CAPS, false)]);
            sent
        };
        let undo = || {
            session::send_keys(&[
                (CAPS, true),
                (ALT, true),
                (b'Z', true),
                (b'Z', false),
                (ALT, false),
                (CAPS, false),
            ]);
        };
        let near = |a: wm_model::Rect, b: wm_model::Rect| {
            (a.left - b.left).abs() <= 1
                && (a.top - b.top).abs() <= 1
                && (a.right - b.right).abs() <= 1
                && (a.bottom - b.bottom).abs() <= 1
        };

        // Mover (120, 84) desde el centro.
        let center = ((f0.left + f0.right) / 2, (f0.top + f0.bottom) / 2);
        let t0 = Instant::now();
        let sent = drag(false, center, (120, 84));
        let want = wm_model::Rect::new(f0.left + 120, f0.top + 84, f0.right + 120, f0.bottom + 84);
        let moved = wait_for(Duration::from_secs(1), || frame_now().is_some_and(|f| near(f, want)));
        if moved.is_none() && sent == 0 {
            ran = false;
        }
        ok &= moved.is_some();
        parts.push(format!(
            "Caps + arrastrar: {}",
            moved.map_or_else(
                || format!("NO (quedó en {:?})", frame_now()),
                |_| format!("movida (120, 84) en {}", ms(t0.elapsed()))
            )
        ));
        let caps_same = session::caps_lock_on() == caps_before;
        ok &= caps_same;
        parts.push(format!("Bloq Mayús igual: {caps_same}"));
        undo();
        let back = wait_for(Duration::from_secs(1), || frame_now().is_some_and(|f| near(f, f0)));
        ok &= back.is_some();
        parts.push(format!("Z la devuelve: {}", back.is_some()));

        // Cambiar el tamaño desde la esquina de abajo a la derecha.
        let corner = (f0.right - 30, f0.bottom - 30);
        drag(true, corner, (60, 40));
        let want = wm_model::Rect::new(f0.left, f0.top, f0.right + 60, f0.bottom + 40);
        let resized =
            wait_for(Duration::from_secs(1), || frame_now().is_some_and(|f| near(f, want)));
        ok &= resized.is_some();
        parts.push(format!(
            "Caps + clic derecho: {}",
            if resized.is_some() {
                "crece (60, 40) con la otra esquina fija".to_owned()
            } else {
                format!("NO (quedó en {:?})", frame_now())
            }
        ));
        undo();
        let _ = wait_for(Duration::from_secs(1), || frame_now().is_some_and(|f| near(f, f0)));

        // Con dos monitores: maximizada en el principal, arrastrada al otro,
        // Z la vuelve a maximizar en el principal (prueba del usuario).
        if let Some(other) = monitors.iter().find(|m| !m.primary) {
            std::thread::sleep(Duration::from_millis(300));
            hide::set_maximized_for_test(id, true);
            let _ = wait_for(Duration::from_secs(1), || hide::is_maximized(id));
            std::thread::sleep(Duration::from_millis(200));
            let fm = frame_now().unwrap_or(f0);
            let from = ((fm.left + fm.right) / 2, fm.top + 200);
            let to =
                ((other.work_area.left + other.work_area.right) / 2, other.work_area.top + 200);
            drag(false, from, (to.0 - from.0, to.1 - from.1));
            let center_in = |f: wm_model::Rect, r: wm_model::Rect| {
                let (cx, cy) = ((f.left + f.right) / 2, (f.top + f.bottom) / 2);
                cx >= r.left && cx < r.right && cy >= r.top && cy < r.bottom
            };
            let crossed = wait_for(Duration::from_secs(1), || {
                !hide::is_maximized(id) && frame_now().is_some_and(|f| center_in(f, other.bounds))
            })
            .is_some();
            std::thread::sleep(Duration::from_millis(100));
            undo();
            let primary = work;
            let back = wait_for(Duration::from_secs(1), || {
                hide::is_maximized(id) && frame_now().is_some_and(|f| center_in(f, primary))
            })
            .is_some();
            ok &= crossed && back;
            parts.push(format!(
                "maximizada → al otro monitor: {crossed}; Z la maximiza en el principal: {back}"
            ));
            hide::set_maximized_for_test(id, false);
            let _ = wait_for(Duration::from_secs(1), || !hide::is_maximized(id));
        }
    } else {
        ran = false;
        parts.push("sin monitor o sin títere".into());
    }
    move |r: &mut Report, hooks: Option<(u32, u32)>| {
        match hooks {
            Some((a, b)) => {
                ok &= a == b && a > 0;
                parts.push(format!("hook de mouse puesto {a} veces y quitado {b}"));
            }
            None => parts.push("sin el resumen del hook".into()),
        }
        let status = if ok {
            Status::Ok
        } else if !ran {
            // El mouse inyectado puede no llegar en el runner de CI (§13.11).
            Status::Skip
        } else {
            Status::Fail
        };
        r.check(NAME, status, parts.join("; "));
    }
}

/// Hito 4c (D4): una ventana en el escritorio 2, el core muere, el guardián
/// muestra todo y lo relanza: el core nuevo la vuelve a esconder por su
/// `wm.desk`. Después, "Salir" deja todo visible y sin `wm.desk`.
fn recovery_check(r: &mut Report, puppets: &[WindowId]) {
    const NAME: &str = "recuperación";
    const CAPS: u8 = 0x14;
    const ALT: u8 = 0xA4;
    let hidden = |id: WindowId| hide::visibility(id).cloaked == hide::DWM_CLOAKED_SHELL;
    let shown = |id: WindowId| hide::fully_visible(hide::visibility(id));
    let (a, others) = (puppets[0], &puppets[1..]);
    let (mut child, _lines) = match spawn_core() {
        Ok(x) => x,
        Err(e) => return r.check(NAME, Status::Fail, e),
    };
    let _ = windows_list::activate(a);
    std::thread::sleep(Duration::from_millis(100));
    session::send_keys(&[
        (CAPS, true),
        (ALT, true),
        (b'2', true),
        (b'2', false),
        (ALT, false),
        (CAPS, false),
    ]);
    let carried =
        wait_for(Duration::from_secs(1), || hidden(a) && hide::desk_of(a) == Some(2)).is_some();
    let _ = child.kill();
    let _ = child.wait();
    // Lo que hace el guardián al morir el core.
    let _ = hide::restore_all();
    let all_visible = puppets.iter().all(|&p| shown(p));
    let kept = hide::desk_of(a) == Some(2);
    let start = Instant::now();
    let (mut child, _lines) = match spawn_core_relaunched(true) {
        Ok(x) => x,
        Err(e) => return r.check(NAME, Status::Fail, e),
    };
    let rehidden =
        wait_for(Duration::from_secs(2), || hidden(a) && others.iter().all(|&o| shown(o)))
            .map(|_| start.elapsed());
    let _ = wm_platform::tray::send_order_in("ci-probe", wm_platform::tray::Order::Quit);
    let exited = wait_for(Duration::from_secs(3), || child.try_wait().ok().flatten().is_some());
    if exited.is_none() {
        let _ = child.kill();
    }
    let clean = wait_for(Duration::from_millis(500), || {
        puppets.iter().all(|&p| shown(p) && hide::desk_of(p).is_none())
    })
    .is_some();
    let ok = carried && all_visible && kept && rehidden.is_some() && clean;
    r.check(
        NAME,
        if ok { Status::Ok } else { Status::Fail },
        format!(
            "llevada al 2: {carried}; core matado → restore_all la muestra: {all_visible} (wm.desk queda: {kept}); relanzado → la vuelve a esconder: {}; Salir → todo visible y sin wm.desk: {clean}",
            rehidden.map_or_else(|| "NO".into(), |d| format!("a los {} desde el lanzamiento", ms(d)))
        ),
    );
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

/// Hito 4a.1: esconder y mostrar en lote, `wm.desk`, y re-esconder una
/// marcada que Windows destapó al activarla (como el clic en la barra).
fn batch_check(r: &mut Report, ids: &[WindowId]) {
    let hider = match Hider::new(HideMethod::ShellCloak) {
        Ok(h) => h,
        Err(e) => return r.check("lotes", Status::Skip, format!("sin cloak: {e}")),
    };
    // La tercera queda visible con el foco: el core nunca esconde la del
    // frente (mueve el foco antes), y esconder la recién activada a veces no
    // toma el cloak (plan-hito4.md §6).
    let (holder, ids) = (ids[2], &ids[..2]);
    let wins: Vec<(WindowId, u32)> =
        ids.iter().map(|&id| (id, windows_list::window_pid(id))).collect();
    let mut problems = Vec::new();
    for &(id, _) in &wins {
        if let Err(e) = hide::set_desk(id, 2) {
            problems.push(format!("set_desk: {e}"));
        }
    }
    // El foco en otra: activar la que ya está al frente no la destapa.
    let _ = windows_list::activate(holder);
    let start = Instant::now();
    let hid = hider.hide_many(&wins);
    let hide_time = start.elapsed();
    if hid.iter().any(|(_, r)| r.is_err()) {
        let late: Vec<String> = hid
            .iter()
            .filter(|(_, r)| r.is_err())
            .map(|(id, _)| {
                let t = wait_for(Duration::from_secs(1), || {
                    hide::visibility(*id).cloaked == hide::DWM_CLOAKED_SHELL
                });
                format!(
                    "{}: {:?}, al frente: {}, {:?}",
                    ids.iter().position(|x| x == id).unwrap_or(9),
                    t,
                    windows_list::foreground() == Some(*id),
                    hide::visibility(*id)
                )
            })
            .collect();
        problems.push(format!("esconder: {hid:?}; después: {late:?}"));
    }
    // Windows destapa una escondida al activarla; la marca queda.
    let _ = windows_list::activate(ids[0]);
    let uncloaked =
        wait_for(Duration::from_millis(300), || hide::visibility(ids[0]).cloaked == 0).is_some();
    let again = hider.hide_many(&wins);
    let recloaked = again.iter().all(|(_, r)| r.is_ok())
        && hide::visibility(ids[0]).cloaked == hide::DWM_CLOAKED_SHELL;
    let desks_ok = ids.iter().all(|&id| hide::desk_of(id) == Some(2));
    let start = Instant::now();
    let shown = hider.show_many(&wins);
    let show_time = start.elapsed();
    let visible = ids.iter().all(|&id| hide::fully_visible(hide::visibility(id)));
    let unmarked = ids.iter().all(|&id| hide::hidden_with(id).is_none());
    if shown.iter().any(|(_, r)| r.is_err()) {
        problems.push(format!("mostrar: {shown:?}"));
    }
    hide::forget_desks();
    let forgotten = ids.iter().all(|&id| hide::desk_of(id).is_none());
    let ok = problems.is_empty() && recloaked && desks_ok && visible && unmarked && forgotten;
    r.check(
        "lotes",
        if ok { Status::Ok } else { Status::Fail },
        format!(
            "2 ventanas: esconder {}, mostrar {}; destapada al activarla: {uncloaked}, re-escondida: {recloaked}; wm.desk: {desks_ok}, visibles y sin marca: {}, wm.desk borrado: {forgotten}{}",
            ms(hide_time),
            ms(show_time),
            visible && unmarked,
            if problems.is_empty() { String::new() } else { format!("; {}", problems.join("; ")) }
        ),
    );
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
    // Recién creadas, las ventanas overlay y el anfitrión del fantasma tienen
    // que dar "vigentes": si no, cada mapa, aviso o lista las rearma (fue el
    // parpadeo de la lista de ventanas, 17/11/2026).
    let current = std::thread::spawn(|| {
        let overlays = wm_platform::overlay::Overlays::new().map(|o| o.is_current());
        let ghost = wm_platform::ghost::GhostHost::new().map(|g| g.is_current());
        (overlays, ghost)
    })
    .join();
    match current {
        Ok((Ok(true), Ok(true))) => {
            r.check("overlay vigente", Status::Ok, "overlays y fantasma recién creados: vigentes".into())
        }
        Ok((o, g)) => r.check(
            "overlay vigente",
            Status::Fail,
            format!("recién creados dan no vigentes (overlays {o:?}, fantasma {g:?}): se rearmarían siempre"),
        ),
        Err(_) => r.check("overlay vigente", Status::Fail, "pánico al crear".into()),
    }
}
