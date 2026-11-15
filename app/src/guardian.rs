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
//!   poco (250 ms, 500 ms, 1 s) y relanzar.

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
    console::trap_stop_requests();

    let pid = std::process::id();
    log(&format!("guardián {pid}: vigila `wm {}`", child_args.join(" ")));
    // Arranque = recuperación (§9.5): restos de una sesión anterior.
    report("al arrancar", &hide::restore_all());

    let mut restarts: VecDeque<Instant> = VecDeque::new();
    let code = loop {
        let started = Instant::now();
        let mut child = match Command::new(&exe).args(&child_args).spawn() {
            Ok(c) => c,
            Err(e) => {
                log(&format!("no se pudo lanzar el hijo: {e}"));
                break ExitCode::FAILURE;
            }
        };
        log(&format!("hijo {} lanzado", child.id()));
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
        if clean {
            log("salida limpia: no se relanza");
            break ExitCode::SUCCESS;
        }
        if raw == Some(USAGE_ERROR) {
            log("error de uso del comando: no se relanza");
            break ExitCode::from(2);
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

fn report(when: &str, r: &RestoreReport) {
    if r.restored.is_empty() && r.failed.is_empty() {
        return;
    }
    log(&format!(
        "{when}: {} ventanas restauradas, {} fallaron",
        r.restored.len(),
        r.failed.len()
    ));
    for (id, e) in &r.failed {
        log(&format!("  NO se pudo restaurar {:#x}: {e}", id.0));
    }
}

fn log(msg: &str) {
    // A stderr: el hijo usa la misma consola para lo suyo.
    eprintln!("[guardián] {msg}");
}
