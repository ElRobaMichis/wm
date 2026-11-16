//! `wm guardian [comando...]` — el guardián (architecture.md §4.1 y §9).
//!
//! Lanza este mismo ejecutable con el comando indicado (por defecto `core`),
//! lo espera sin gastar CPU y, si muere mal, muestra todas las ventanas que
//! tengan nuestra marca y lo vuelve a lanzar: como mucho 3 veces por minuto.
//!
//! - Salida limpia del hijo (código 0): restaurar y terminar.
//! - Ctrl+C o cerrar la consola: el hijo termina, el guardián restaura y
//!   termina sin relanzar.
//! - Error de uso (código 2): restaurar y terminar.
//! - Cualquier otra salida (matado, pánico, error): restaurar, esperar un
//!   poco (250 ms, 500 ms, 1 s) y relanzar (con `WM_RELAUNCH=n`, el gancho
//!   de D4 para el hito 4).
//! - Cierre de sesión o apagado: no se relanza ni se avisa.
//!
//! **Instancia única** (hito 3a): si vigila al core, toma el mutex
//! `Local\wm.guardian` **antes** de restaurar nada; un segundo guardián
//! sale sin tocar las ventanas del primero. Sin consola, todo va a
//! `%LOCALAPPDATA%\wm\wm.log`, y si se rinde lo dice en un cuadro.

use std::collections::VecDeque;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

use wm_platform::console;
use wm_platform::hide::{self, RestoreReport};

/// Relanzamientos permitidos por ventana de tiempo.
const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(60);
const FIRST_BACKOFF: Duration = Duration::from_millis(250);

/// Código de salida de un proceso cortado con Ctrl+C (STATUS_CONTROL_C_EXIT).
const CONTROL_C_EXIT: u32 = 0xC000_013A;
/// Código con que `wm` sale ante un error de uso: relanzar no lo arregla.
const USAGE_ERROR: u32 = 2;
/// Hito U: cuánto tiene que andar una versión recién colocada para darla
/// por buena.
const TRIAL: Duration = Duration::from_secs(60);

/// Una versión recién colocada, a prueba.
struct Trial {
    dir: std::path::PathBuf,
    from: wm_model::update::Version,
    to: wm_model::update::Version,
}

pub fn run(args: &[String]) -> ExitCode {
    let child_args: Vec<String> =
        if args.is_empty() { vec!["core".to_owned()] } else { args.to_vec() };
    if child_args.first().is_some_and(|c| c == "guardian") {
        eprintln!("El guardián no puede vigilar a otro guardián.");
        return ExitCode::from(2);
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("No se encontró el propio ejecutable: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Instancia única: antes de restaurar nada.
    let _instance = if child_args.first().is_some_and(|c| c == "core") {
        match wm_platform::instance::acquire("guardian") {
            Some(i) => Some(i),
            None => {
                // Doble clic otra vez en el acceso directo: abrir los ajustes
                // del que ya está corriendo.
                let opened = wm_platform::tray::send_order(wm_platform::tray::Order::Settings);
                log(if opened {
                    "ya hay un wm corriendo: le pido que abra los ajustes"
                } else {
                    "ya hay un wm corriendo en esta sesión: no arranco otro"
                });
                return ExitCode::SUCCESS;
            }
        }
    } else {
        None
    };
    console::trap_stop_requests();

    let pid = std::process::id();
    log(&format!("guardián {pid}: vigila `wm {}`", child_args.join(" ")));
    // Arranque = recuperación (§9.5): restos de una sesión anterior.
    report("al arrancar", &hide::restore_all());

    // Hito U: con el core cerrado, terminar o deshacer un cambio a medias y
    // colocar una versión que quedó lista.
    let watching_core = child_args.first().is_some_and(|c| c == "core");
    let mut trial: Option<Trial> = None;
    if watching_core && crate::update::installed_here() {
        trial = start_trial_if_any();
    }
    let mut update_failed = false;
    // Relanzar ya, con los escritorios (código 10 de "Actualizar ahora").
    let mut relaunch_now = false;

    let mut restarts: VecDeque<Instant> = VecDeque::new();
    let code = loop {
        let started = Instant::now();
        let mut cmd = Command::new(&exe);
        let relaunch = if relaunch_now { 1 } else { restarts.len() };
        relaunch_now = false;
        cmd.args(&child_args).env("WM_RELAUNCH", relaunch.to_string());
        if let Some(t) = &trial {
            cmd.env("WM_UPDATED_FROM", t.from.to_string());
        }
        if std::mem::take(&mut update_failed) {
            cmd.env("WM_UPDATE_FAILED", "1");
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                log(&format!("no se pudo lanzar el hijo: {e}"));
                break ExitCode::FAILURE;
            }
        };
        log(&format!("hijo {} lanzado", child.id()));
        // En prueba: si pasa 60 s andando, la versión nueva queda.
        if let Some(t) = &trial {
            let alive = loop {
                match child.try_wait() {
                    Ok(None) if started.elapsed() < TRIAL => {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    Ok(None) => break true,
                    _ => break false,
                }
            };
            if alive {
                crate::update::commit(&t.dir, t.to);
                log(&format!("la versión {} anduvo {} s: queda", t.to, TRIAL.as_secs()));
                trial = None;
            }
        }
        let status = child.wait();
        let lived = started.elapsed();

        let t = Instant::now();
        let restored = hide::restore_all();
        let restore_ms = t.elapsed().as_secs_f64() * 1000.0;

        let raw = status.as_ref().ok().and_then(std::process::ExitStatus::code).map(|c| c as u32);
        let clean = matches!(raw, Some(0));
        let ctrl_c = raw == Some(CONTROL_C_EXIT) || console::stop_requested();
        let how = match raw {
            Some(c) => format!("código {c:#x}"),
            None => format!("{status:?}"),
        };
        log(&format!(
            "hijo terminó tras {:.1} s ({how}); restauración en {restore_ms:.1} ms",
            lived.as_secs_f64()
        ));
        report("tras la salida del hijo", &restored);

        if ctrl_c {
            log("parada pedida por el usuario: no se relanza");
            break ExitCode::SUCCESS;
        }
        // Hito U: "Actualizar ahora": colocar y relanzar ya, sin contarlo
        // como caída (los escritorios se recuperan como tras una caída).
        if raw == Some(crate::update::EXIT_APPLY_UPDATE) && trial.is_none() {
            log("el core pidió actualizar");
            trial = start_trial_if_any();
            relaunch_now = true;
            continue;
        }
        // La versión en prueba se cayó antes de 60 s: volver a la anterior.
        if let Some(t) = trial.take()
            && !clean
        {
            log(&format!("la versión {} se cayó a prueba: vuelvo a la {}", t.to, t.from));
            crate::update::rollback(&t.dir, Some(t.to));
            update_failed = true;
            restarts.clear();
            relaunch_now = true;
            continue;
        }
        if clean {
            log("salida limpia: no se relanza");
            break ExitCode::SUCCESS;
        }
        if raw == Some(USAGE_ERROR) {
            log("error de uso del comando: no se relanza");
            break ExitCode::from(2);
        }
        if shutting_down() {
            log("Windows se está apagando o cerrando la sesión: no se relanza");
            break ExitCode::SUCCESS;
        }

        let now = Instant::now();
        while restarts.front().is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW) {
            restarts.pop_front();
        }
        if restarts.len() >= MAX_RESTARTS {
            log(&format!(
                "el hijo murió {} veces en menos de {} s: me rindo (ventanas ya restauradas)",
                MAX_RESTARTS + 1,
                RESTART_WINDOW.as_secs()
            ));
            tell_gave_up();
            break ExitCode::FAILURE;
        }
        let backoff = FIRST_BACKOFF * (1u32 << restarts.len());
        restarts.push_back(now);
        log(&format!("relanzando en {} ms", backoff.as_millis()));
        std::thread::sleep(backoff);
        if console::stop_requested() {
            log("parada pedida por el usuario durante la espera");
            break ExitCode::SUCCESS;
        }
    };
    console::mark_finished();
    code
}

/// Coloca una versión lista (si hay) o retoma una prueba en curso.
fn start_trial_if_any() -> Option<Trial> {
    let dir = crate::update::install_dir()?;
    if let Some((from, to)) = crate::update::recover_at_start(&dir) {
        log(&format!("la versión {to} sigue a prueba"));
        return Some(Trial { dir, from, to });
    }
    match crate::update::apply_staged(&dir) {
        Ok(Some(to)) => {
            let from = crate::update::current();
            log(&format!("versión {to} colocada (antes {from}); a prueba {} s", TRIAL.as_secs()));
            Some(Trial { dir, from, to })
        }
        Ok(None) => None,
        Err(e) => {
            log(&format!("no se pudo colocar la actualización: {e}"));
            None
        }
    }
}

fn report(when: &str, r: &RestoreReport) {
    if r.restored.is_empty() && r.failed.is_empty() {
        return;
    }
    log(&format!("{when}: {} ventanas restauradas, {} fallaron", r.restored.len(), r.failed.len()));
    for (id, e) in &r.failed {
        log(&format!("  NO se pudo restaurar {:#x}: {e}", id.0));
    }
}

fn log(msg: &str) {
    // A stderr (el hijo usa la misma consola para lo suyo) y al archivo: sin
    // consola (wmw.exe) es lo único que queda.
    eprintln!("[guardián] {msg}");
    crate::brain::append_line(&format!("[guardián] {msg}"));
}

/// ¿Windows se está apagando o cerrando la sesión?
fn shutting_down() -> bool {
    wm_platform::session::shutting_down()
}

/// Sin consola, rendirse en silencio dejaría a la usuaria sin saber qué
/// pasó: un cuadro con lo esencial.
fn tell_gave_up() {
    if wm_platform::session::has_console() {
        return; // con consola, el mensaje ya se vio
    }
    let path = crate::brain::log_path().map(|p| p.display().to_string()).unwrap_or_default();
    wm_platform::session::warning_box(
        "wm",
        &format!(
            "wm se cerró por un error que se repitió.

Tus ventanas quedaron visibles.

Detalle: {path}"
        ),
    );
}
