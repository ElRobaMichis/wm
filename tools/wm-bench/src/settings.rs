//! `wm-bench settings [arranques]` — spike 8 (architecture.md §13.10): mide
//! las dos ventanas de ajustes (Slint: `wm-settings.exe`; nativa:
//! `wm.exe settings`) con la misma pantalla.
//!
//! - **Arranque:** desde antes de lanzar el proceso hasta el primer cuadro en
//!   que se ve la marca magenta (Desktop Duplication, como el frío del spike
//!   5). Las dos se abren en (100, 100) con `--bench`.
//! - **Memoria:** working set privado y commit 1 s después de mostrarse.
//! - **CPU en reposo:** durante 3 s sin tocar nada.

use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use wm_platform::{clock, probe};

use crate::meter::{self, ColorKey};
use crate::sysinfo;

/// Fila (pantalla) que cruza la marca: ventana en y = 100, barra de título
/// ~31 px, marca de 64 px.
const ROW: u32 = 100 + 31 + 32;

struct Variant {
    name: &'static str,
    exe: std::path::PathBuf,
    args: Vec<&'static str>,
}

fn pct(v: &mut [f64]) -> String {
    if v.is_empty() {
        return "sin datos".into();
    }
    v.sort_by(f64::total_cmp);
    let p = |q: f64| v[((v.len() as f64 - 1.0) * q).round() as usize];
    format!("p50 {:.0} ms | p95 {:.0} ms | máx {:.0} ms", p(0.5), p(0.95), p(1.0))
}

pub fn run(args: &[String], dir: &Path) -> ExitCode {
    let runs: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(20).max(2);
    let variants = [
        Variant {
            name: "Slint (wm-settings.exe)",
            exe: dir.join("wm-settings.exe"),
            args: vec!["--bench", "100,100"],
        },
        Variant {
            name: "nativa (wm.exe settings)",
            exe: dir.join("wm.exe"),
            args: vec!["settings", "--bench", "100,100"],
        },
    ];
    for v in &variants {
        let size = std::fs::metadata(&v.exe).map(|m| m.len()).unwrap_or(0);
        println!("{}: {:.1} MB en disco", v.name, size as f64 / 1048576.0);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let meters = match meter::start(ROW, ColorKey::MAGENTA, Arc::clone(&stop)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Desktop Duplication: {e}");
            return ExitCode::FAILURE;
        }
    };
    std::thread::sleep(Duration::from_millis(300));

    struct Measured {
        starts: Vec<i64>,
        mem: Option<sysinfo::Memory>,
        idle_cpu_ms: f64,
    }
    let mut results: Vec<Measured> = Vec::new();
    for v in &variants {
        let mut r = Measured { starts: Vec::new(), mem: None, idle_cpu_ms: f64::NAN };
        for i in 0..runs {
            let t0 = clock::now();
            let Ok(mut child) = Command::new(&v.exe).args(&v.args).stdout(Stdio::null()).spawn()
            else {
                continue;
            };
            r.starts.push(t0);
            std::thread::sleep(Duration::from_millis(1000));
            if i == runs - 1 {
                r.mem = sysinfo::of_pid(child.id());
                let h = child.as_raw_handle() as isize;
                let c0 = probe::process_cpu_ms(h);
                std::thread::sleep(Duration::from_secs(3));
                r.idle_cpu_ms = probe::process_cpu_ms(h) - c0;
            }
            let _ = child.kill();
            let _ = child.wait();
            std::thread::sleep(Duration::from_millis(400));
        }
        results.push(r);
    }
    let end = clock::now();
    stop.store(true, Ordering::Relaxed);

    let primary = meters.into_iter().find(|m| m.desktop.left == 0 && m.desktop.top == 0);
    let samples = primary.and_then(|m| m.finish().ok()).map(|m| m.samples).unwrap_or_default();
    println!();
    for (v, r) in variants.iter().zip(&results) {
        let mut times: Vec<f64> = r
            .starts
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let until = r.starts.get(i + 1).copied().unwrap_or(end);
                meter::first_visible(&samples, *t, until)
            })
            .collect();
        let first = times.first().copied();
        println!("== {}", v.name);
        println!(
            "  arranque hasta el primer cuadro ({} de {} vistos): {}",
            times.len(),
            r.starts.len(),
            pct(&mut times)
        );
        if let Some(f) = first {
            println!("    (el primero de la serie: {f:.0} ms)");
        }
        match r.mem {
            Some(m) => println!(
                "  memoria: {:.1} MB WS privado / {:.1} MB commit",
                m.private_ws as f64 / 1048576.0,
                m.commit as f64 / 1048576.0
            ),
            None => println!("  memoria: no se pudo leer"),
        }
        println!("  CPU en reposo: {:.1} ms en 3 s", r.idle_cpu_ms);
    }
    ExitCode::SUCCESS
}
