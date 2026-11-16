//! `wm core` — Caps = Hyper e ir a una app por su inicial
//! (keyboard-design.md §6.1). Las decisiones están en `wm_model::world`; este
//! archivo arranca los hilos, corre el bucle del brain ([`Brain`]) y sale
//! siempre mostrando todo (plan-core.md §2).
//!
//! Hilos (architecture.md §4.2 y §13.6):
//! - **brain** (este): único dueño del estado. Nunca llama a nada que pueda
//!   bloquearse: espera eventos con [`input::wait`] y pide el trabajo a los
//!   carriles de [`Ops`];
//! - **input** y **inyector**: el hook de teclado (spike 1);
//! - **ops** `focus`, `win`, `shell`: llamadas a ventanas ajenas y a Explorer;
//! - **log**: consola y `%LOCALAPPDATA%\wm\wm.log`.
//!
//! Si el brain entra en pánico, se muestra todo y se sale con error: el
//! guardián lo relanza (architecture.md §9.3).

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wm_model::apps::{AppBinding, Target, bindings_path, parse_bindings, resolve};
use wm_model::world::{World, shorten};
use wm_platform::input::{self, LatencyStats, Wake};
use wm_platform::ops::{self, Ops};
use wm_platform::windows_list::{self, WindowId};

use crate::brain::{Brain, Log, log_path};

/// Cuánto esperar a que termine la restauración al salir.
const EXIT_RESTORE_WAIT: Duration = Duration::from_millis(1500);

/// Código de salida tras un pánico (el guardián relanza).
const EXIT_PANIC: u8 = 3;

pub fn run() -> ExitCode {
    let config = bindings_path();
    let bindings = load_bindings(config.as_ref());
    let test_mode = std::env::var_os("WM_TEST").is_some_and(|v| v == "1");
    let (log, log_thread) = Log::start();

    ops::boost_process();
    // Que un hilo nuestro colgado nunca se convierta en una ventana fantasma
    // (con overlays, una fantasma taparía la pantalla).
    wm_platform::overlay::disable_ghosting();
    input::ensure_message_queue();
    ops::boost_brain_thread();
    let brain_thread = input::current_thread_id();
    let ops = match Ops::start(brain_thread) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("No se pudieron crear los hilos de trabajo: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut brain = Brain::new(World::new(bindings), ops, log.clone(), test_mode);
    brain.restore_at_start();

    input::quit_on_console_close(brain_thread);
    if let Err(e) = input::start_keyboard_hook(brain_thread, test_mode) {
        eprintln!("No se pudo instalar el hook de teclado: {e}");
        return ExitCode::FAILURE;
    }
    banner(&log, brain.world.bindings(), config.as_ref(), test_mode);

    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| main_loop(&mut brain)));
    let code = match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "sin detalle".to_owned());
            log.line(format!("ERROR INTERNO (pánico): {what}. Muestro todo y salgo."));
            ExitCode::from(EXIT_PANIC)
        }
    };

    // Salida (§9.7), limpia o tras un pánico: siempre mostrar todo, con plazo.
    brain.restore_at_exit(EXIT_RESTORE_WAIT);
    print_summary(&brain);
    drop(brain);
    drop(log);
    let _ = log_thread.join();
    code
}

fn main_loop(brain: &mut Brain) {
    loop {
        match input::wait(brain.ops.next_deadline()) {
            Wake::Quit => break,
            Wake::Input(event) => {
                brain.on_input(event);
                if let Some(ms) = input::acknowledge() {
                    let secs = f64::from(ms) / 1000.0;
                    brain.log.line(format!(
                        "El programa volvió a responder (modo seguro durante {secs:.1} s)."
                    ));
                }
            }
            Wake::Ops | Wake::Timeout | Wake::Ping(_) => {}
        }
        brain.collect();
        if brain.quit {
            break;
        }
    }
}

fn banner(log: &Log, bindings: &[AppBinding], config: Option<&PathBuf>, test_mode: bool) {
    log.line("wm core activo.");
    log.line("  Caps + letra    ir a la app con esa inicial (repetir = siguiente ventana)");
    log.line("  Caps (toque)    Bloq Mayús normal");
    log.line("  Caps + Esc      salir (también Ctrl+C)");
    if test_mode {
        log.line("  MODO PRUEBA     Caps + F10 = pánico a propósito (muestra todo y sale con 3)");
        log.line("                  Caps + F11 = trabar el programa 6 s");
        log.line("                  (prueba el modo seguro)");
        log.line("                  Caps + F12 = trabar el hook 1,5 s (prueba el vigía)");
    }
    match config {
        Some(path) => {
            log.line(format!("  Asignaciones:   {} ({} cargadas)", path.display(), bindings.len()))
        }
        None => log.line("  Asignaciones:   sin %APPDATA%; solo modo automático"),
    }
    for b in bindings {
        let launch = b.launch.as_deref().map(|p| format!("  (abre {p})")).unwrap_or_default();
        log.line(format!("    {} = {}{launch}", b.letter as char, b.app));
    }
    if let Some(path) = log_path() {
        log.line(format!("  Log:            {}", path.display()));
    }
    log.line("");
}

fn print_summary(brain: &Brain) {
    let log = &brain.log;
    let stats = input::hook_stats();
    let o = brain.ops.stats();
    let acts = brain.world.activations();
    log.line("");
    log.line(format!("Traer al frente: {}/{} ok", acts.ok, acts.total));
    log.line(format!("  con tecla neutra: {}", acts.neutral_key));
    log.line("Latencia del hook (resolución 10 µs; meta: p99.9 < 200 µs y máximo < 5 ms):");
    log.line(latency("  sin inyección", &stats.plain));
    log.line(latency("  con inyección", &stats.injecting));
    log.line(format!(
        "Hook reinstalado: {} por el vigía, {} al volver del reposo, {} al desbloquear",
        stats.reinstalls_watchdog, stats.reinstalls_resume, stats.reinstalls_unlock
    ));
    log.line(format!("Veces en modo seguro: {}", stats.safe_mode_entries));
    log.line(format!("Callbacks lentos (> 100 ms) con estado reiniciado: {}", stats.stalls));
    log.line(format!(
        "Hilos de trabajo abandonados: {} (volvieron tarde: {})",
        o.abandoned, o.late
    ));
}

fn latency(label: &str, s: &LatencyStats) -> String {
    if s.events == 0 {
        return format!("{label}: sin eventos");
    }
    format!(
        "{label} ({} eventos): p50 ≤ {} µs | p99 ≤ {} µs | p99.9 ≤ {} µs | máx {:.1} µs",
        s.events, s.p50_us, s.p99_us, s.p999_us, s.max_us
    )
}

/// `wm apps`: muestra las ventanas abiertas y a qué apunta cada letra, sin
/// mover el foco ni instalar hooks.
pub fn list_apps() -> ExitCode {
    let config = bindings_path();
    let bindings = load_bindings(config.as_ref());
    let started = Instant::now();
    let windows = windows_list::user_windows();
    let elapsed = started.elapsed();
    println!(
        "{} ventanas de usuario (enumeradas en {:.1} ms):",
        windows.len(),
        elapsed.as_secs_f64() * 1000.0
    );
    for w in &windows {
        let hidden = if w.hidden { "  (escondida por wm)" } else { "" };
        println!("  {:<22} «{}»{hidden}", w.app, shorten(&w.title, 60));
    }

    let list: Vec<(WindowId, &str)> = windows.iter().map(|w| (w.id, w.app.as_str())).collect();
    println!();
    println!("Caps + letra (con la ventana actual al frente):");
    for letter in b'A'..=b'Z' {
        match resolve(letter, &bindings, &list, windows_list::foreground()) {
            Target::Focus { app, .. } => println!("  {} → {app}", letter as char),
            Target::Launch { app, path } => {
                println!("  {} → abrir {app} ({path})", letter as char);
            }
            Target::Nothing => {}
        }
    }
    ExitCode::SUCCESS
}

fn load_bindings(path: Option<&PathBuf>) -> Vec<AppBinding> {
    let Some(path) = path else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let (bindings, errors) = parse_bindings(&text);
    for e in errors {
        eprintln!("{}: {e}", path.display());
    }
    bindings
}
