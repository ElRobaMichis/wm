//! `wm core` — prototipo del spike 1 (architecture.md §13.2):
//! Caps = Hyper e ir a una app por su inicial (keyboard-design.md §6.1).
//!
//! Hilo principal = "brain": recibe las acciones del hilo del hook y decide.
//! Todavía no hay guardián, overlays ni capa de ventana.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wm_model::apps::{AppBinding, Target, parse_bindings, resolve};
use wm_model::keyboard::HyperAction;
use wm_platform::input::{self, InputEvent, LatencyStats, ReinstallReason};
use wm_platform::windows_list::{self, WindowId};

/// Teclas de prueba (solo con `WM_TEST=1`).
const VK_F11: u8 = 0x7A;
const VK_F12: u8 = 0x7B;

/// Antigüedad máxima de una acción para ejecutarla.
const MAX_ACTION_AGE_MS: u32 = 1000;

fn describe(action: HyperAction) -> String {
    match action {
        HyperAction::Letter { letter, thumb: false } => format!("Caps + {}", letter as char),
        HyperAction::Letter { letter, thumb: true } => format!("Caps + Alt + {}", letter as char),
        HyperAction::Digit { digit, .. } => format!("Caps + {digit}"),
        HyperAction::Escape => "Caps + Esc".to_owned(),
        HyperAction::Other { vk, .. } => format!("Caps + tecla {vk:#04x}"),
    }
}

/// Cuántas veces se intentó traer una app al frente y cómo salió.
#[derive(Default)]
struct Activations {
    total: u32,
    ok: u32,
    neutral_key: u32,
}

pub fn run() -> ExitCode {
    let config = config_path();
    let bindings = load_bindings(config.as_ref());
    let test_mode = std::env::var_os("WM_TEST").is_some_and(|v| v == "1");

    input::ensure_message_queue();
    let brain = input::current_thread_id();
    input::quit_on_console_close(brain);
    if let Err(e) = input::start_keyboard_hook(brain, test_mode) {
        eprintln!("No se pudo instalar el hook de teclado: {e}");
        return ExitCode::FAILURE;
    }

    println!("wm core (spike 1) activo.");
    println!("  Caps + letra    ir a la app con esa inicial (repetir = siguiente ventana)");
    println!("  Caps (toque)    Bloq Mayús normal");
    println!("  Caps + Esc      salir (también Ctrl+C)");
    if test_mode {
        println!("  MODO PRUEBA     Caps + F11 = trabar el programa 6 s (prueba el modo seguro)");
        println!("                  Caps + F12 = trabar el hook 1,5 s (prueba el vigía)");
    }
    match &config {
        Some(path) => {
            println!("  Asignaciones:   {} ({} cargadas)", path.display(), bindings.len());
        }
        None => println!("  Asignaciones:   sin %APPDATA%; solo modo automático"),
    }
    for b in &bindings {
        let launch = b.launch.as_deref().map(|p| format!("  (abre {p})")).unwrap_or_default();
        println!("    {} = {}{launch}", b.letter as char, b.app);
    }
    println!();

    let mut activations = Activations::default();
    while let Some(event) = input::next_event() {
        match event {
            InputEvent::Action { action: HyperAction::Escape, .. } => break,
            InputEvent::Action { action, time_ms } => {
                // Una acción que llega tarde (el programa estuvo trabado) ya no
                // es lo que el usuario quiere ahora: se descarta.
                let age = input::tick_ms().wrapping_sub(time_ms);
                if age > MAX_ACTION_AGE_MS {
                    println!("Acción descartada: llegó {age} ms tarde ({}).", describe(action));
                } else {
                    handle(action, &bindings, test_mode, &mut activations);
                }
            }
            InputEvent::HookReinstalled(reason) => {
                let why = match reason {
                    ReinstallReason::Watchdog => "el vigía detectó que Windows lo había quitado",
                    ReinstallReason::Resume => "la PC volvió del reposo",
                    ReinstallReason::Unlock => "se desbloqueó la sesión",
                };
                println!("Hook reinstalado: {why}.");
            }
            InputEvent::HookStalled { ms } => {
                println!("El hook tardó {ms} ms: estado del teclado reiniciado.");
            }
        }
        if let Some(ms) = input::acknowledge() {
            let secs = f64::from(ms) / 1000.0;
            println!("El programa volvió a responder (modo seguro durante {secs:.1} s).");
        }
    }

    print_summary(&activations);
    ExitCode::SUCCESS
}

fn handle(action: HyperAction, bindings: &[AppBinding], test_mode: bool, acts: &mut Activations) {
    match action {
        HyperAction::Letter { letter, thumb: false } => go_to_app(letter, bindings, acts),
        HyperAction::Letter { letter, thumb: true } => {
            println!("Caps + Alt + {}: capa de ventana, pendiente.", letter as char);
        }
        HyperAction::Digit { digit, thumb } => {
            let what = if thumb { "llevar a" } else { "ir a" };
            println!("Caps + {digit}: {what} escritorio {digit}, pendiente.");
        }
        HyperAction::Other { vk: VK_F11, .. } if test_mode => {
            println!("PRUEBA: el programa se traba 6 s. Pasados 2 s, Caps + B tiene que");
            println!("        escribir una B en vez de ir a Brave (modo seguro).");
            std::thread::sleep(Duration::from_secs(6));
        }
        HyperAction::Other { vk: VK_F12, .. } if test_mode => {
            println!("PRUEBA: el hook se trabó 1,5 s. Escribí unas letras: tienen que salir");
            println!("        normales (Caps no puede quedar trabado).");
        }
        HyperAction::Escape | HyperAction::Other { .. } => {}
    }
}

fn print_summary(acts: &Activations) {
    let stats = input::hook_stats();
    println!();
    println!("Traer al frente: {}/{} ok", acts.ok, acts.total);
    println!("  con tecla neutra: {}", acts.neutral_key);
    println!("Latencia del hook (resolución 10 µs; meta: p99.9 < 200 µs y máximo < 5 ms):");
    print_latency("  sin inyección", &stats.plain);
    print_latency("  con inyección", &stats.injecting);
    println!(
        "Hook reinstalado: {} por el vigía, {} al volver del reposo, {} al desbloquear",
        stats.reinstalls_watchdog, stats.reinstalls_resume, stats.reinstalls_unlock
    );
    println!("Veces en modo seguro: {}", stats.safe_mode_entries);
    println!("Callbacks lentos (> 100 ms) con estado reiniciado: {}", stats.stalls);
}

fn print_latency(label: &str, s: &LatencyStats) {
    if s.events == 0 {
        println!("{label}: sin eventos");
        return;
    }
    println!(
        "{label} ({} eventos): p50 ≤ {} µs | p99 ≤ {} µs | p99.9 ≤ {} µs | máx {:.1} µs",
        s.events, s.p50_us, s.p99_us, s.p999_us, s.max_us
    );
}

/// `wm apps`: muestra las ventanas abiertas y a qué apunta cada letra, sin
/// mover el foco ni instalar hooks.
pub fn list_apps() -> ExitCode {
    let config = config_path();
    let bindings = load_bindings(config.as_ref());
    let started = Instant::now();
    let windows = windows_list::user_windows();
    let elapsed = started.elapsed();
    println!("{} ventanas de usuario (enumeradas en {:.1} ms):", windows.len(), ms(elapsed));
    for w in &windows {
        println!("  {:<22} «{}»", w.app, shorten(&w.title, 60));
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

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn go_to_app(letter: u8, bindings: &[AppBinding], acts: &mut Activations) {
    let started = Instant::now();
    let windows = windows_list::user_windows();
    let list: Vec<(WindowId, &str)> = windows.iter().map(|w| (w.id, w.app.as_str())).collect();
    let foreground = windows_list::foreground();
    let key = letter as char;

    match resolve(letter, bindings, &list, foreground) {
        Target::Focus { window, app } => {
            let title = windows.iter().find(|w| w.id == window).map_or("", |w| w.title.as_str());
            let result = windows_list::activate(window);
            acts.total += 1;
            acts.ok += u32::from(result.ok);
            acts.neutral_key += u32::from(result.used_neutral_key);
            let status = if result.ok { "ok" } else { "FALLÓ" };
            let trick = if result.used_neutral_key { ", con tecla neutra" } else { "" };
            println!(
                "Caps + {key} → {app} «{}»: {status} en {:.1} ms{trick} (total {:.1} ms)",
                shorten(title, 50),
                result.elapsed.as_secs_f64() * 1000.0,
                started.elapsed().as_secs_f64() * 1000.0,
            );
        }
        Target::Launch { app, path } => match std::process::Command::new(&path).spawn() {
            Ok(_) => println!("Caps + {key} → abriendo {app} ({path})"),
            Err(e) => println!("Caps + {key} → no se pudo abrir {app} ({path}): {e}"),
        },
        Target::Nothing => println!("Caps + {key}: no hay ninguna app abierta con la {key}."),
    }
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("wm").join("apps.txt"))
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

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{cut}…")
}
