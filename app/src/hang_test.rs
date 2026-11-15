//! Spike 4 (architecture.md §13.2): ¿qué llamadas se traban con una app
//! colgada, y las operaciones sobre las demás ventanas siguen rápidas?
//!
//! - `wm hang-puppet`: abre el títere colgable (en su propio proceso).
//! - `wm hang-probe [sleep|busy] [segundos]`: cuelga el títere y corre cada
//!   llamada que usamos en un hilo aparte con 2 s de plazo, antes y durante
//!   el cuelgue (y después de que aparezca la ventana fantasma). Con títeres
//!   comunes abiertos, mide también esconder/mostrar los demás mientras tanto.

use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use wm_platform::hide::{self, HideMethod, Hider};
use wm_platform::probe;
use wm_platform::puppet;
use wm_platform::windows_list::{self, WindowId};

const PLAZO: Duration = Duration::from_secs(2);

pub fn run_hang_puppet() -> ExitCode {
    println!("Títere colgable abierto. `wm hang-probe` lo cuelga a pedido. Cerralo para salir.");
    match puppet::run_hang_puppet() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("No se pudo abrir el títere colgable: {e}");
            ExitCode::FAILURE
        }
    }
}

type Probe = (&'static str, Box<dyn FnOnce() + Send>);

fn probes(id: WindowId) -> Vec<Probe> {
    fn hider_op(method: HideMethod, id: WindowId, hide_it: bool) -> Box<dyn FnOnce() + Send> {
        Box::new(move || {
            if let Ok(h) = Hider::new(method) {
                let _ = if hide_it { h.hide(id) } else { h.show(id) };
            }
        })
    }
    vec![
        ("GetWindowTextW", Box::new(move || drop(windows_list::window_title(id)))),
        ("visibility (IsWindowVisible, IsIconic, GetWindowRect, DwmGetWindowAttribute)",
            Box::new(move || { let _ = hide::visibility(id); })),
        ("IsZoomed", Box::new(move || { let _ = hide::is_maximized(id); })),
        ("user_windows (EnumWindows + títulos + nombre de proceso)",
            Box::new(|| drop(windows_list::user_windows()))),
        ("marked_windows (EnumWindows + GetPropW)", Box::new(|| drop(hide::marked_windows()))),
        ("cloak: esconder (SetPropW + SetCloak vía Explorer)",
            hider_op(HideMethod::ShellCloak, id, true)),
        ("cloak: mostrar", hider_op(HideMethod::ShellCloak, id, false)),
        ("minimize: esconder (DwmSetWindowAttribute + ShowWindowAsync)",
            hider_op(HideMethod::Minimize, id, true)),
        ("minimize: mostrar", hider_op(HideMethod::Minimize, id, false)),
        ("offscreen: esconder (SetWindowPos ASYNC)", hider_op(HideMethod::Offscreen, id, true)),
        ("offscreen: mostrar", hider_op(HideMethod::Offscreen, id, false)),
        ("activate (SetForegroundWindow + tecla neutra)",
            Box::new(move || { let _ = windows_list::activate(id); })),
        ("restore_all", Box::new(|| drop(hide::restore_all()))),
        ("[control] SendMessageTimeout 100 ms", Box::new(move || {
            let _ = probe::responds(id, 100);
        })),
        ("[control] SetWindowPos síncrono", Box::new(move || probe::set_window_pos_sync(id))),
    ]
}

/// Corre cada sonda en su propio hilo, con plazo.
fn run_phase(label: &str, id: WindowId) {
    println!();
    println!("== {label}");
    for (name, f) in probes(id) {
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        std::thread::spawn(move || {
            f();
            let _ = tx.send(started.elapsed());
        });
        match rx.recv_timeout(PLAZO) {
            Ok(d) => println!("  {:>9.2} ms  {name}", d.as_secs_f64() * 1000.0),
            Err(_) => println!("  TRABADA     {name}  (no volvió en {} s)", PLAZO.as_secs()),
        }
    }
}

/// Esconde y muestra con cloak los títeres comunes `rounds` veces, en un hilo
/// aparte, y devuelve la duración de cada operación individual.
fn others(puppets: Vec<WindowId>, rounds: usize) -> mpsc::Receiver<Vec<Duration>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok(hider) = Hider::new(HideMethod::ShellCloak) else { return };
        let mut times = Vec::new();
        for _ in 0..rounds {
            for &id in &puppets {
                let t = Instant::now();
                let _ = hider.hide(id);
                times.push(t.elapsed());
            }
            for &id in &puppets {
                let t = Instant::now();
                let _ = hider.show(id);
                times.push(t.elapsed());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let _ = tx.send(times);
    });
    rx
}

fn report_others(label: &str, rx: mpsc::Receiver<Vec<Duration>>) {
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(mut t) if !t.is_empty() => {
            t.sort();
            let pct = |p: f64| t[((t.len() as f64 - 1.0) * p).round() as usize];
            println!(
                "  {label}: {} operaciones, p50 {:.2} ms | p99 {:.2} ms | máx {:.2} ms",
                t.len(),
                pct(0.5).as_secs_f64() * 1000.0,
                pct(0.99).as_secs_f64() * 1000.0,
                t.last().unwrap_or(&Duration::ZERO).as_secs_f64() * 1000.0
            );
        }
        _ => println!("  {label}: TRABADAS (no terminaron en 10 s)"),
    }
}

pub fn probe(args: &[String]) -> ExitCode {
    let busy = args.first().is_some_and(|a| a == "busy");
    let secs: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(25).clamp(10, 120);
    let Some(id) = puppet::find_hang_puppet() else {
        eprintln!("Falta el títere colgable: `wm hang-puppet` en otra terminal.");
        return ExitCode::FAILURE;
    };
    let commons = puppet::find_puppets();
    let _ = hide::restore_all();

    run_phase("Títere respondiendo (referencia)", id);
    if !commons.is_empty() {
        report_others("demás títeres, sin cuelgue", others(commons.clone(), 10));
    }

    let kind = if busy { "ocupado (CPU al 100 %)" } else { "dormido (no atiende mensajes)" };
    println!();
    println!("Colgando el títere {secs} s, {kind}...");
    if let Err(e) = puppet::hang(id, secs * 1000, busy) {
        eprintln!("No se pudo colgar el títere: {e}");
        return ExitCode::FAILURE;
    }
    let hung_at = Instant::now();
    std::thread::sleep(Duration::from_millis(300));
    let others_rx = (!commons.is_empty()).then(|| others(commons.clone(), 10));
    run_phase("Recién colgado (sin fantasma todavía)", id);
    if let Some(rx) = others_rx {
        report_others("demás títeres, con uno colgado", rx);
    }

    // Esperar a que Windows lo dé por colgado (IsHungAppWindow, ~5 s).
    let mut hung_after = None;
    while hung_at.elapsed() < Duration::from_secs(9) {
        if hung_after.is_none() && probe::is_hung(id) {
            hung_after = Some(hung_at.elapsed());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!();
    match hung_after {
        Some(d) => println!("IsHungAppWindow dio TRUE a los {:.1} s", d.as_secs_f64()),
        None => println!("IsHungAppWindow seguía en FALSE a los 9 s"),
    }
    let ghosts = probe::ghost_windows();
    println!("Ventanas fantasma (clase Ghost): {}", ghosts.len());
    for (g, pid) in &ghosts {
        let title = windows_list::window_title(*g);
        println!("  {:#x} del proceso {pid}: «{title}»", g.0);
    }
    let list = windows_list::user_windows();
    let ghosts_listed = list.iter().filter(|w| ghosts.iter().any(|(g, _)| *g == w.id)).count();
    let hung_listed = list.iter().any(|w| w.id == id);
    let v = hide::visibility(id);
    println!("  fantasmas en user_windows(): {ghosts_listed}");
    println!(
        "  la ventana colgada: en user_windows() {hung_listed}, visible {}, DWMWA_CLOAKED {}",
        v.visible, v.cloaked
    );
    run_phase("Colgado hace 9 s (con fantasma si Windows lo creó)", id);

    // Esperar a que se descuelgue y dejar todo en orden.
    let until = Duration::from_secs(u64::from(secs) + 3);
    println!();
    let left = until.saturating_sub(hung_at.elapsed()).as_secs();
    println!("Esperando a que se descuelgue (~{left} s)...");
    while hung_at.elapsed() < until {
        std::thread::sleep(Duration::from_millis(200));
    }
    let report = hide::restore_all();
    std::thread::sleep(Duration::from_millis(500));
    let v = hide::visibility(id);
    println!(
        "Al final: {} restauradas, títere colgable {} (responde: {})",
        report.restored.len(),
        if hide::fully_visible(v) { "visible" } else { "NO visible" },
        probe::responds(id, 500)
    );
    ExitCode::SUCCESS
}
