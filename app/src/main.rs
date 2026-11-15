//! Ejecutable único con roles (architecture.md §4.1).
//!
//! Implementados: `doctor`, `version`, `core` (prototipo del spike 1),
//! `puppet`, `hide-test` y `restore` (spike 2). `guardian` y `settings`
//! llegan en los spikes siguientes.

use std::process::ExitCode;

#[cfg(windows)]
mod core;
#[cfg(windows)]
mod hide_test;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let command = std::env::args().nth(1);
    match command.as_deref() {
        #[cfg(windows)]
        Some("puppet") => hide_test::run_puppets(&args),
        #[cfg(windows)]
        Some("hide-test") => hide_test::run(&args),
        #[cfg(windows)]
        Some("restore") => hide_test::restore(),
        None | Some("doctor") => doctor(),
        Some("version" | "--version" | "-V") => {
            println!("wm {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        #[cfg(windows)]
        Some("core") => core::run(),
        #[cfg(windows)]
        Some("apps") => core::list_apps(),
        Some("burn") => burn(std::env::args().nth(2)),
        Some(role @ ("guardian" | "settings")) => {
            eprintln!("`{role}` todavía no está implementado (ver architecture.md §13).");
            ExitCode::from(2)
        }
        Some(other) => {
            eprintln!("Comando desconocido: `{other}`.");
            eprintln!("Uso: wm [doctor | core | apps | burn [s] | puppet [n] | restore | version");
            eprintln!("          | hide-test <método> [rondas] [apps...]]");
            ExitCode::from(2)
        }
    }
}

/// `wm burn [segundos]`: ocupa todos los núcleos al 100 % (prioridad normal)
/// para medir el hook bajo carga (spike 1, architecture.md §13.2).
fn burn(seconds: Option<String>) -> ExitCode {
    use std::time::{Duration, Instant};

    let secs: u64 = seconds.and_then(|s| s.parse().ok()).unwrap_or(60);
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    println!("Ocupando {threads} núcleos al 100 % durante {secs} s (Ctrl+C para cortar)...");
    let deadline = Instant::now() + Duration::from_secs(secs);
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(move || {
                const MUL: u64 = 6_364_136_223_846_793_005;
                let mut x: u64 = 1;
                while Instant::now() < deadline {
                    for _ in 0..10_000 {
                        x = std::hint::black_box(x.wrapping_mul(MUL).wrapping_add(1));
                    }
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    println!("Listo.");
    ExitCode::SUCCESS
}

#[cfg(windows)]
fn doctor() -> ExitCode {
    use wm_model::{Placement, place};

    println!("wm {}", env!("CARGO_PKG_VERSION"));
    match wm_platform::os_build() {
        Some(build) => println!("Windows build: {build}"),
        None => println!("Windows build: desconocido"),
    }
    println!("DPI awareness: {}", wm_platform::dpi_awareness());

    let monitors = wm_platform::monitors();
    println!("Monitores: {}", monitors.len());
    for m in &monitors {
        let scale = m.dpi * 100 / 96;
        let hz = m.refresh_hz.map_or_else(|| "?".to_owned(), |hz| hz.to_string());
        println!(
            "  {}{}: {}x{} a {} Hz, escala {}% ({} DPI), área de trabajo {:?}",
            m.device,
            if m.primary { " (principal)" } else { "" },
            m.bounds.width(),
            m.bounds.height(),
            hz,
            scale,
            m.dpi,
            m.work_area,
        );
        let left = place(m.work_area, Placement::LeftHalf);
        let right = place(m.work_area, Placement::RightHalf);
        println!("    mitad izquierda {left:?} | mitad derecha {right:?}");
    }
    ExitCode::SUCCESS
}

#[cfg(not(windows))]
fn doctor() -> ExitCode {
    eprintln!("Este programa solo funciona en Windows.");
    ExitCode::FAILURE
}
