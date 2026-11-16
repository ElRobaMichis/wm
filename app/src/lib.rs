//! Ejecutable con roles (architecture.md §4.1). Lo comparten `wm.exe` (de
//! consola: CLI, CI y pruebas) y `wmw.exe` (sin consola: el que se lanza al
//! iniciar sesión, plan-core.md D5).
//!
//! Implementados: `doctor`, `version`, `core` (prototipo del spike 1),
//! `puppet`, `hide-test` y `restore` (spike 2), `guardian`, `hidden`,
//! `hide-hold` y `crash-test` (spike 3), `hang-puppet` y `hang-probe`
//! (spike 4), `ci-probe` (spike 9) y `settings` (spike 8).

use std::process::ExitCode;

#[cfg(windows)]
mod brain;
#[cfg(windows)]
mod ci_probe;
#[cfg(windows)]
mod core;
#[cfg(windows)]
mod display_test;
#[cfg(windows)]
mod ghost_test;
#[cfg(windows)]
mod guardian;
#[cfg(windows)]
mod hang_test;
#[cfg(windows)]
mod hide_test;
#[cfg(windows)]
mod ops_test;
#[cfg(windows)]
mod overlay_test;
#[cfg(windows)]
mod place_test;

/// Lo que hacen los dos ejecutables (`wm.exe` de consola y `wmw.exe` sin
/// consola, plan-core.md D5): elegir el rol por el primer argumento.
pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let command = std::env::args().nth(1);
    match command.as_deref() {
        #[cfg(windows)]
        Some("ci-probe") => ci_probe::run(&args),
        #[cfg(windows)]
        Some("puppet") => hide_test::run_puppets(&args),
        #[cfg(windows)]
        Some("hide-test") => hide_test::run(&args),
        #[cfg(windows)]
        Some("restore") => hide_test::restore(),
        #[cfg(windows)]
        Some("hidden") => hide_test::hidden(),
        #[cfg(windows)]
        Some("hide-hold") => hide_test::hold(&args),
        #[cfg(windows)]
        Some("crash-test") => hide_test::crash_test(&args),
        #[cfg(windows)]
        Some("max-test") => hide_test::max_test(),
        #[cfg(windows)]
        Some("puppet-hard") => {
            use wm_platform::puppet::{HardKind, run_hard_puppet};
            match args.first().and_then(|k| HardKind::from_name(k)) {
                Some(kind) => match run_hard_puppet(kind) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("No se pudo abrir el títere: {e}");
                        ExitCode::FAILURE
                    }
                },
                None => {
                    let names: Vec<&str> = HardKind::ALL.iter().map(|k| k.name()).collect();
                    eprintln!("Uso: wm puppet-hard <{}>", names.join("|"));
                    ExitCode::from(2)
                }
            }
        }
        #[cfg(windows)]
        Some("hang-puppet") => hang_test::run_hang_puppet(),
        #[cfg(windows)]
        Some("hang-probe") => hang_test::probe(&args),
        #[cfg(windows)]
        Some("ops-test") => ops_test::run(&args),
        #[cfg(windows)]
        Some("overlay-demo") => overlay_test::demo(&args),
        #[cfg(windows)]
        Some("ghost-demo") => ghost_test::demo(&args),
        #[cfg(windows)]
        Some("displays") => displays(),
        #[cfg(windows)]
        Some("display-watch") => display_test::run(&args),
        #[cfg(windows)]
        Some("place-test") => place_test::run(&args),
        #[cfg(windows)]
        Some("guardian") => guardian::run(&args),
        #[cfg(windows)]
        Some("ctrl-c") => {
            // Herramienta de prueba del guardián: Ctrl+C a la consola de otro proceso.
            let Some(pid) = args.first().and_then(|p| p.parse().ok()) else {
                eprintln!("Uso: wm ctrl-c <pid> [break | close]");
                return ExitCode::from(2);
            };
            let mode = args.get(1).map(String::as_str);
            let result = match mode {
                Some("close") => wm_platform::console::close_console(pid),
                _ => wm_platform::console::send_ctrl_c(pid, mode == Some("break")),
            };
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
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
        #[cfg(windows)]
        Some("settings") => {
            // Versión nativa (spike 8). La de Slint es `wm-settings.exe`.
            let flag = |name: &str| {
                args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
            };
            let bench = flag("--bench").and_then(|s| {
                let (x, y) = s.split_once(',')?;
                Some((x.parse().ok()?, y.parse().ok()?))
            });
            let lang = flag("--lang").unwrap_or_else(|| "es".to_owned());
            match wm_platform::native_settings::run(wm_platform::native_settings::Options {
                lang,
                bench,
            }) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("No se pudo abrir la ventana de ajustes: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(other) => {
            eprintln!("Comando desconocido: `{other}`.");
            eprintln!("Uso: wm [doctor | core | apps | burn [s] | puppet [n] | version");
            eprintln!("          | guardian [comando...] | restore | hidden");
            eprintln!("          | hide-test <método> [rondas] [apps...]");
            eprintln!("          | hide-hold <método> [cycle] | crash-test [método] [n]]");
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

/// `wm displays`: monitores activos con su identidad estable (spike 7).
#[cfg(windows)]
fn displays() -> ExitCode {
    match wm_platform::displays::displays() {
        Ok(list) => {
            for d in list {
                let hz = d.refresh_hz.map_or("?".to_owned(), |h| h.to_string());
                println!(
                    "{}{}: «{}» EDID {:04x}:{:04x}, conector {}, salida {}",
                    d.gdi_name,
                    if d.primary { " (principal)" } else { "" },
                    d.name,
                    d.edid_manufacturer,
                    d.edid_product,
                    d.connector,
                    d.output_technology
                );
                println!(
                    "    {}x{} en ({}, {}), {} Hz, {} DPI, área de trabajo {:?}",
                    d.bounds.width(),
                    d.bounds.height(),
                    d.bounds.left,
                    d.bounds.top,
                    hz,
                    d.dpi,
                    d.work_area
                );
                let serial = d.id.serial.map_or("sin serie".to_owned(), |s| format!("serie {s}"));
                println!(
                    "    identidad: EDID {:04x}:{:04x}, {serial}, conector {}{}",
                    d.id.manufacturer,
                    d.id.product,
                    d.id.connector,
                    if d.edid_valid { "" } else { " (EDID NO confiable)" }
                );
                println!("    ruta: {}", d.key);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("QueryDisplayConfig: {e}");
            ExitCode::FAILURE
        }
    }
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
