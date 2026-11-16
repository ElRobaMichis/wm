//! `wm core` — Caps = Hyper e ir a una app por su inicial
//! (keyboard-design.md §6.1).
//!
//! Hilos (architecture.md §4.2 y §13.6):
//! - **brain** (este): único dueño del estado. Nunca llama a nada que pueda
//!   bloquearse: espera eventos con [`input::wait`] y pide el trabajo a los
//!   carriles de [`Ops`];
//! - **input** y **inyector**: el hook de teclado (spike 1);
//! - **ops** `focus`, `win`, `shell`: llamadas a ventanas ajenas y a Explorer;
//! - **log**: escribe en la consola. `println!` puede trabarse (por ejemplo,
//!   si el usuario selecciona texto en la terminal), así que el brain nunca
//!   escribe directo.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use wm_model::apps::{AppBinding, Target, bindings_path, parse_bindings, resolve};
use wm_model::keyboard::HyperAction;
use wm_platform::clock;
use wm_platform::input::{self, InputEvent, LatencyStats, ReinstallReason, Wake};
use wm_platform::ops::{self, Op, Ops, OpsEvent, Output, Ticket};
use wm_platform::windows_list::{self, WindowId};

/// Teclas de prueba (solo con `WM_TEST=1`).
const VK_F11: u8 = 0x7A;
const VK_F12: u8 = 0x7B;

/// Antigüedad máxima de una acción para ejecutarla.
const MAX_ACTION_AGE_MS: u32 = 1000;

/// Cuánto esperar a que termine la restauración al salir.
const EXIT_RESTORE_WAIT: Duration = Duration::from_millis(1500);

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

/// Consola en un hilo aparte: el brain solo manda líneas.
struct Log(Sender<String>);

impl Log {
    fn start() -> (Self, std::thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<String>();
        let handle = std::thread::Builder::new()
            .name("wm-log".into())
            .spawn(move || {
                for line in rx {
                    println!("{line}");
                }
            })
            .expect("no se pudo crear el hilo de log");
        (Self(tx), handle)
    }

    fn line(&self, s: impl Into<String>) {
        let _ = self.0.send(s.into());
    }
}

/// Lo que el brain espera de cada pedido en vuelo.
enum Pending {
    /// Enumerar ventanas para resolver `Caps` + letra.
    Scan {
        letter: u8,
        key_qpc: i64,
    },
    /// Traer al frente.
    Focus {
        letter: u8,
        app: String,
        title: String,
        key_qpc: i64,
    },
    Launch {
        letter: u8,
        app: String,
        path: String,
    },
    RestoreAtStart,
    RestoreAtExit,
}

struct Core {
    ops: Ops,
    log: Log,
    bindings: Vec<AppBinding>,
    test_mode: bool,
    acts: Activations,
    pending: HashMap<Ticket, Pending>,
}

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
    let brain = input::current_thread_id();
    let ops = match Ops::start(brain) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("No se pudieron crear los hilos de trabajo: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut core = Core {
        ops,
        log,
        bindings,
        test_mode,
        acts: Activations::default(),
        pending: HashMap::new(),
    };

    // Arranque = recuperación (architecture.md §9.5), en el carril de Explorer.
    let t = core.ops.submit(Op::RestoreAll);
    core.pending.insert(t, Pending::RestoreAtStart);

    input::quit_on_console_close(brain);
    if let Err(e) = input::start_keyboard_hook(brain, test_mode) {
        eprintln!("No se pudo instalar el hook de teclado: {e}");
        return ExitCode::FAILURE;
    }
    core.banner(config.as_ref());

    loop {
        let wake = input::wait(core.ops.next_deadline());
        match wake {
            Wake::Quit => break,
            Wake::Input(InputEvent::Action { action: HyperAction::Escape, .. }) => break,
            Wake::Input(event) => {
                core.on_input(event);
                if let Some(ms) = input::acknowledge() {
                    let secs = f64::from(ms) / 1000.0;
                    core.log.line(format!(
                        "El programa volvió a responder (modo seguro durante {secs:.1} s)."
                    ));
                }
            }
            Wake::Ops | Wake::Timeout | Wake::Ping(_) => {}
        }
        core.collect();
    }

    // Salida limpia (§9.7): siempre mostrar todo, con plazo.
    let t = core.ops.submit(Op::RestoreAll);
    core.pending.insert(t, Pending::RestoreAtExit);
    let until = Instant::now() + EXIT_RESTORE_WAIT;
    while core.pending.values().any(|p| matches!(p, Pending::RestoreAtExit)) {
        let deadline = core.ops.next_deadline().map_or(until, |d| d.min(until));
        if matches!(input::wait(Some(deadline)), Wake::Quit) || Instant::now() >= until {
            break;
        }
        core.collect();
    }

    core.print_summary();
    drop(core);
    let _ = log_thread.join();
    ExitCode::SUCCESS
}

impl Core {
    fn banner(&self, config: Option<&PathBuf>) {
        let log = &self.log;
        log.line("wm core activo.");
        log.line("  Caps + letra    ir a la app con esa inicial (repetir = siguiente ventana)");
        log.line("  Caps (toque)    Bloq Mayús normal");
        log.line("  Caps + Esc      salir (también Ctrl+C)");
        if self.test_mode {
            log.line("  MODO PRUEBA     Caps + F11 = trabar el programa 6 s");
            log.line("                  (prueba el modo seguro)");
            log.line("                  Caps + F12 = trabar el hook 1,5 s (prueba el vigía)");
        }
        match config {
            Some(path) => log.line(format!(
                "  Asignaciones:   {} ({} cargadas)",
                path.display(),
                self.bindings.len()
            )),
            None => log.line("  Asignaciones:   sin %APPDATA%; solo modo automático"),
        }
        for b in &self.bindings {
            let launch = b.launch.as_deref().map(|p| format!("  (abre {p})")).unwrap_or_default();
            log.line(format!("    {} = {}{launch}", b.letter as char, b.app));
        }
        log.line("");
    }

    fn on_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::Action { action, time_ms, qpc } => {
                // Una acción que llega tarde (el programa estuvo trabado) ya no
                // es lo que el usuario quiere ahora: se descarta.
                let age = input::tick_ms().wrapping_sub(time_ms);
                if age > MAX_ACTION_AGE_MS {
                    self.log.line(format!(
                        "Acción descartada: llegó {age} ms tarde ({}).",
                        describe(action)
                    ));
                } else {
                    self.handle(action, qpc);
                }
            }
            InputEvent::HookReinstalled(reason) => {
                let why = match reason {
                    ReinstallReason::Watchdog => "el vigía detectó que Windows lo había quitado",
                    ReinstallReason::Resume => "la PC volvió del reposo",
                    ReinstallReason::Unlock => "se desbloqueó la sesión",
                };
                self.log.line(format!("Hook reinstalado: {why}."));
            }
            InputEvent::HookStalled { ms } => {
                self.log.line(format!("El hook tardó {ms} ms: estado del teclado reiniciado."));
            }
        }
    }

    fn handle(&mut self, action: HyperAction, key_qpc: i64) {
        match action {
            HyperAction::Letter { letter, thumb: false } => {
                let t = self.ops.submit(Op::Scan);
                self.pending.insert(t, Pending::Scan { letter, key_qpc });
            }
            HyperAction::Letter { letter, thumb: true } => {
                let key = letter as char;
                self.log.line(format!("Caps + Alt + {key}: capa de ventana, pendiente."));
            }
            HyperAction::Digit { digit, thumb } => {
                let what = if thumb { "llevar a" } else { "ir a" };
                self.log.line(format!("Caps + {digit}: {what} escritorio {digit}, pendiente."));
            }
            HyperAction::Other { vk: VK_F11, .. } if self.test_mode => {
                self.log.line("PRUEBA: el programa se traba 6 s. Pasados 2 s, Caps + B tiene que");
                self.log.line("        escribir una B en vez de ir a Brave (modo seguro).");
                std::thread::sleep(Duration::from_secs(6));
            }
            HyperAction::Other { vk: VK_F12, .. } if self.test_mode => {
                self.log.line("PRUEBA: el hook se trabó 1,5 s. Escribí unas letras:");
                self.log.line("        tienen que salir normales (Caps no puede quedar trabado).");
            }
            HyperAction::Escape | HyperAction::Other { .. } => {}
        }
    }

    /// Resultados de los carriles (y plazos vencidos).
    fn collect(&mut self) {
        let mut events = self.ops.expire(Instant::now());
        events.extend(self.ops.drain());
        for event in events {
            match event {
                OpsEvent::Done { ticket, result, .. } => {
                    if let Some(p) = self.pending.remove(&ticket) {
                        self.on_done(p, result);
                    }
                }
                OpsEvent::Late { lane, call_ms, .. } => {
                    self.log.line(format!(
                        "Aviso: una llamada del carril {} volvió tras {call_ms} ms.",
                        lane.name()
                    ));
                }
            }
        }
    }

    fn on_done(&mut self, pending: Pending, result: Result<Output, String>) {
        match (pending, result) {
            (Pending::Scan { letter, key_qpc }, Ok(Output::Windows(windows))) => {
                self.go_to_app(letter, key_qpc, &windows);
            }
            (Pending::Focus { letter, app, title, key_qpc }, Ok(Output::Activation(a))) => {
                self.acts.total += 1;
                self.acts.ok += u32::from(a.ok);
                self.acts.neutral_key += u32::from(a.used_neutral_key);
                let status = if a.ok { "ok" } else { "FALLÓ" };
                let trick = if a.used_neutral_key { ", con tecla neutra" } else { "" };
                self.log.line(format!(
                    "Caps + {} → {app} «{}»: {status} en {:.1} ms{trick} (desde la tecla {:.1} ms)",
                    letter as char,
                    shorten(&title, 50),
                    a.elapsed.as_secs_f64() * 1000.0,
                    clock::micros_since(key_qpc) as f64 / 1000.0,
                ));
            }
            (Pending::Launch { letter, app, path }, Ok(_)) => {
                self.log.line(format!("Caps + {} → abriendo {app} ({path})", letter as char));
            }
            (Pending::Launch { letter, app, path }, Err(e)) => {
                let key = letter as char;
                self.log.line(format!("Caps + {key} → no se pudo abrir {app} ({path}): {e}"));
            }
            (Pending::RestoreAtStart, Ok(Output::Restored(r))) if !r.restored.is_empty() => {
                let n = r.restored.len();
                self.log.line(format!("Restauradas {n} ventanas de una sesión anterior."));
            }
            (Pending::RestoreAtStart | Pending::RestoreAtExit, _) => {}
            (Pending::Scan { letter, .. } | Pending::Focus { letter, .. }, Err(e)) => {
                self.log.line(format!("Caps + {}: {e}", letter as char));
            }
            (Pending::Scan { .. } | Pending::Focus { .. }, Ok(_)) => {}
        }
    }

    fn go_to_app(&mut self, letter: u8, key_qpc: i64, windows: &[windows_list::WindowInfo]) {
        let list: Vec<(WindowId, &str)> = windows.iter().map(|w| (w.id, w.app.as_str())).collect();
        let key = letter as char;
        match resolve(letter, &self.bindings, &list, windows_list::foreground()) {
            Target::Focus { window, app } => {
                let title =
                    windows.iter().find(|w| w.id == window).map_or("", |w| w.title.as_str());
                let (app, title) = (app.to_owned(), title.to_owned());
                let pending = Pending::Focus { letter, app, title, key_qpc };
                let t = self.ops.submit(Op::Focus(window));
                self.pending.insert(t, pending);
            }
            Target::Launch { app, path } => {
                let (app, path_s) = (app.to_owned(), path.to_owned());
                let pending = Pending::Launch { letter, app, path: path_s };
                let t = self.ops.submit(Op::Launch(path.to_owned()));
                self.pending.insert(t, pending);
            }
            Target::Nothing => {
                self.log.line(format!("Caps + {key}: no hay ninguna app abierta con la {key}."));
            }
        }
    }

    fn print_summary(&self) {
        let log = &self.log;
        let stats = input::hook_stats();
        let o = self.ops.stats();
        log.line("");
        log.line(format!("Traer al frente: {}/{} ok", self.acts.ok, self.acts.total));
        log.line(format!("  con tecla neutra: {}", self.acts.neutral_key));
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
