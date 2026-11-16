//! `wm-bench ghost <puppet|explorer|brave|terminal|rayo> [dx] [ms] [vueltas]` — spike 6
//! (architecture.md §7.4): mide la animación con fantasma.
//!
//! `wm ghost-demo … curtain` pone un telón magenta del tamaño del monitor
//! justo debajo de la ventana animada; Desktop Duplication mira una fila a
//! 3/4 de la altura de la ventana y cuenta los píxeles que **no** son telón:
//! - en reposo, ese conteo es el ancho de la ventana en la fila (base);
//! - **defecto** = cualquier cuadro de la secuencia con el conteo a más de
//!   3 px de la base: menos es un hueco ("sin ventana"), más es "doble
//!   imagen" (real y fantasma a la vez, aunque sea un poco corridas);
//! - **cuadros distintos**: dentro del vuelo, tomado de las muestras (desde
//!   el primer cuadro que se movió hasta el primero en el destino), cuántos
//!   refrescos mostraron otra posición.
//!
//! Para `explorer`, `brave` y `terminal` abre una ventana nueva y la cierra al final.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use wm_platform::clock;

use crate::meter::{self, ColorKey, Sample};

const SLACK_PX: u32 = 3;

/// Abre la ventana de prueba. Devuelve el proceso (si hay que matarlo) y
/// los argumentos de destino para `ghost-demo`.
fn open_target(kind: &str, exe: &std::path::Path) -> Option<(Option<Child>, Vec<String>)> {
    match kind {
        "puppet" => {
            let p = Command::new(exe).args(["puppet", "10"]).stdout(Stdio::null()).spawn().ok()?;
            std::thread::sleep(Duration::from_millis(1500));
            Some((Some(p), vec!["puppet".into(), "1".into()]))
        }
        "explorer" => {
            let dir = exe.parent()?.to_string_lossy().to_string();
            Command::new("explorer.exe").arg(dir).spawn().ok()?;
            std::thread::sleep(Duration::from_millis(2500));
            Some((None, vec!["app".into(), "explorer".into()]))
        }
        "brave" => {
            let brave = r"C:\Program Files\BraveSoftware\Brave-Origin\Application\brave.exe";
            Command::new(brave).args(["--new-window", "https://example.com"]).spawn().ok()?;
            std::thread::sleep(Duration::from_millis(3000));
            Some((None, vec!["app".into(), "brave".into()]))
        }
        // La ventana de Rayo (el administrador de archivos del usuario) que ya
        // está abierta: no se abre ni se cierra nada; la demo la deja igual.
        "rayo" => Some((None, vec!["app".into(), "rayo".into()])),
        "terminal" => {
            Command::new("wt.exe").args(["-w", "new"]).spawn().ok()?;
            std::thread::sleep(Duration::from_millis(3000));
            Some((None, vec!["app".into(), "windowsterminal".into()]))
        }
        _ => None,
    }
}

fn close_window(hwnd: isize) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    // SAFETY: PostMessageW no bloquea; si la ventana ya no existe, falla.
    let _ = unsafe {
        PostMessageW(Some(HWND(hwnd as *mut core::ffi::c_void)), WM_CLOSE, WPARAM(0), LPARAM(0))
    };
}

struct Flight {
    t0: i64,
    t1: i64,
    arrived: bool,
    /// Con cambio de tamaño (mini-spike 6b): espera de aterrizaje y anchos
    /// del marco al salir y al llegar.
    resize: Option<(u32, i32, i32)>,
}

pub fn run(args: &[String], exe: &std::path::Path) -> ExitCode {
    let kind = args.first().map_or("puppet", String::as_str);
    let dx: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(600);
    let ms: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    let reps: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);

    let Some((mut helper, target)) = open_target(kind, exe) else {
        eprintln!("Uso: wm-bench ghost <puppet|explorer|brave|terminal|rayo> [dx] [ms] [vueltas]");
        return ExitCode::FAILURE;
    };
    let mut demo_args: Vec<String> = vec!["ghost-demo".into()];
    demo_args.extend(target);
    demo_args.extend([dx.to_string(), ms.to_string(), reps.to_string(), "curtain".into()]);
    // `lane`: los vuelos los hace el carril `anim` del core (hito 1e);
    // `resize=…` y `fit=…`: vuelos con cambio de tamaño (mini-spike 6b).
    for a in args {
        if a == "lane" || a.starts_with("resize=") || a.starts_with("fit=") {
            demo_args.push(a.clone());
        }
    }
    let mut demo = match Command::new(exe).args(&demo_args).stdout(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("No se pudo lanzar ghost-demo: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut lines = BufReader::new(demo.stdout.take().expect("stdout")).lines();
    let ready: Vec<i64> = loop {
        match lines.next() {
            Some(Ok(l)) if l.starts_with("READY") => {
                break l.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            }
            Some(Ok(_)) => {}
            _ => {
                eprintln!("ghost-demo no arrancó (¿no encontró la ventana?).");
                if let Some(h) = helper.as_mut() {
                    let _ = h.kill();
                }
                return ExitCode::FAILURE;
            }
        }
    };
    let [left, top, right, bottom] = [0, 1, 2, 3].map(|i| ready[i] as i32);
    let target_hwnd = ready.get(4).copied().unwrap_or(0) as isize;
    // Con cambio de tamaño, la demo elige una fila dentro de las dos
    // ubicaciones.
    let row_y = ready.get(5).map_or(top + (bottom - top) * 3 / 4, |&y| y as i32);
    let monitor = wm_platform::monitors()
        .into_iter()
        .map(|m| m.bounds)
        .find(|b| left >= b.left && left < b.right && row_y >= b.top && row_y < b.bottom);
    let Some(monitor) = monitor else {
        eprintln!("La ventana no está en ningún monitor.");
        return ExitCode::FAILURE;
    };

    let stop = Arc::new(AtomicBool::new(false));
    let meters =
        match meter::start((row_y - monitor.top) as u32, ColorKey::NotCurtain, Arc::clone(&stop)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Desktop Duplication: {e}");
                return ExitCode::FAILURE;
            }
        };
    let measure_start = clock::now();

    let mut flights: Vec<Flight> = Vec::new();
    let until = Instant::now() + Duration::from_secs(300);
    for line in lines.map_while(Result::ok) {
        if line.starts_with("END") || Instant::now() > until {
            break;
        }
        if let Some(rest) = line.strip_prefix("ANIM ") {
            let v: Vec<i64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 5 {
                let resize =
                    (v.len() >= 8 && v[6] != v[7]).then(|| (v[5] as u32, v[6] as i32, v[7] as i32));
                flights.push(Flight { t0: v[0], t1: v[1], arrived: v[4] == 1, resize });
            }
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    let _ = demo.wait();
    match helper.as_mut() {
        Some(h) => {
            let _ = h.kill();
            let _ = h.wait();
        }
        None if kind != "rayo" => close_window(target_hwnd),
        None => {}
    }

    println!(
        "{kind}: marco visible {}×{} px, {dx} px en {ms} ms, {} vuelos (llegaron a tiempo: {})",
        right - left,
        bottom - top,
        flights.len(),
        flights.iter().filter(|f| f.arrived).count()
    );
    for m in meters {
        let (name, hz, desktop) = (m.name.clone(), m.refresh_hz, m.desktop);
        let Ok(measured) = m.finish() else { continue };
        if desktop.left != monitor.left || desktop.top != monitor.top {
            continue;
        }
        report(&name, hz, ms, &measured.samples, measure_start, &flights);
    }
    ExitCode::SUCCESS
}

/// Anchos aceptables en la fila para un cuadro presentado en `qpc`, en un
/// vuelo con cambio de tamaño: el ancho del fantasma según la misma curva
/// del carril (`anim::ease_out`), con ±2 cuadros de margen de tiempo (el
/// cuadro se calcula para el siguiente y se presenta en el próximo refresco).
fn expected_widths(f: &Flight, qpc: i64, ms: u64, hz: f64, offset: i64) -> Option<(u32, u32)> {
    let (_, w0, w1) = f.resize?;
    let us = |a: i64, b: i64| clock::micros(a.min(b), a.max(b)) as f64 * (b - a).signum() as f64;
    let period_us = 1e6 / hz;
    let dur_us = ms as f64 * 1000.0;
    let width = |dt_us: f64| {
        let u = wm_platform::anim::ease_out((dt_us / dur_us).clamp(0.0, 1.0));
        f64::from(w0) + f64::from(w1 - w0) * u
    };
    let dt = us(f.t0, qpc);
    let (a, b) = (width(dt - 2.0 * period_us), width(dt + 2.0 * period_us));
    let (lo, hi) = (a.min(b), a.max(b));
    // Después del vuelo, el ancho final.
    let (lo, hi) =
        if qpc > f.t1 { (lo.min(f64::from(w1)), hi.max(f64::from(w1))) } else { (lo, hi) };
    let shift = |w: f64| (w.round() as i64 + offset).max(0) as u32;
    Some((shift(lo), shift(hi)))
}

fn report(
    name: &str,
    hz: f64,
    ms: u64,
    samples: &[Sample],
    measure_start: i64,
    flights: &[Flight],
) {
    // Ancho en reposo: los últimos cuadros antes de la primera secuencia
    // (cuando el telón ya está pintado).
    let first_seq = flights.first().map_or(i64::MAX, |f| f.t0 - clock::plus_micros(0, 150_000));
    let before: Vec<u32> = samples
        .iter()
        .filter(|s| s.qpc >= measure_start && s.qpc < first_seq)
        .map(|s| s.covered)
        .collect();
    let mut idle: Vec<u32> = before.iter().rev().take(5).copied().collect();
    idle.sort_unstable();
    let Some(&base) = idle.get(idle.len() / 2) else {
        println!("{name}: sin cuadros antes del primer vuelo");
        return;
    };
    println!("== {name} ({hz:.2} Hz): ancho de la ventana en la fila {base} px");
    // Con cambio de tamaño: lo que la fila ve de más o de menos respecto del
    // marco (en reposo, antes del primer vuelo).
    let offset = flights.first().and_then(|f| f.resize).map_or(0, |(_, w0, _)| {
        let off = i64::from(base) - i64::from(w0);
        println!("  con cambio de tamaño: la fila ve {off:+} px respecto del marco");
        off
    });

    let (mut gaps, mut doubles, mut moved, mut refreshes, mut frames) = (0, 0, 0, 0.0, 0);
    let mut notes: Vec<String> = Vec::new();
    for (i, f) in flights.iter().enumerate() {
        let from = f.t0 - (clock::plus_micros(0, 150_000));
        let to = clock::plus_micros(f.t1, 200_000);
        let seq: Vec<&Sample> = samples.iter().filter(|s| s.qpc >= from && s.qpc < to).collect();
        let (mut g, mut d) = (0, 0);
        let mut odd: Vec<String> = Vec::new();
        for s in &seq {
            let (lo, hi) = expected_widths(f, s.qpc, ms, hz, offset).unwrap_or((base, base));
            if s.covered + SLACK_PX < lo {
                g += 1;
            } else if s.covered > hi + SLACK_PX {
                d += 1;
            } else {
                continue;
            }
            let ms = (s.qpc - f.t0) as f64 / clock::plus_micros(0, 1000) as f64;
            odd.push(format!("{ms:+.0} ms: {} px", s.covered));
        }
        gaps += g;
        doubles += d;
        if !odd.is_empty() && notes.len() < 6 {
            notes.push(format!("vuelo {}: {}", i + 1, odd.join(", ")));
        }
        // Vuelo según las muestras: del primer cuadro que se movió al primero
        // en la posición final.
        let xs: Vec<(i64, Option<u32>)> = seq.iter().map(|s| (s.qpc, s.x)).collect();
        let (Some(first), Some(last)) = (xs.first().and_then(|x| x.1), xs.last().and_then(|x| x.1))
        else {
            continue;
        };
        let Some(a) = xs.iter().position(|x| x.1.is_some_and(|v| v != first)) else { continue };
        let Some(b) = xs[a..].iter().position(|x| x.1 == Some(last)).map(|p| p + a) else {
            continue;
        };
        frames += b - a + 1;
        moved += (a..=b).filter(|&k| xs[k].1 != xs[k - 1].1).count();
        refreshes += clock::micros(xs[a - 1].0, xs[b].0) as f64 / 1e6 * hz;
    }
    let pct = if refreshes > 0.0 { 100.0 * moved as f64 / refreshes } else { 0.0 };
    println!(
        "  vuelo: {pct:.1} % de refrescos con el fantasma en otra posición \
         ({moved} de {refreshes:.0}; {frames} cuadros de DWM)"
    );
    println!(
        "  cuadros con defecto (±{SLACK_PX} px): {gaps} sin ventana, {doubles} con doble imagen"
    );
    let mut lands: Vec<u32> = flights.iter().filter_map(|f| f.resize.map(|r| r.0)).collect();
    if !lands.is_empty() {
        lands.sort_unstable();
        let p95 = lands[(lands.len() * 95 / 100).min(lands.len() - 1)];
        let max = lands[lands.len() - 1];
        println!("  espera de aterrizaje: p95 {p95} ms, máx {max} ms (meta p95 ≤ 50 ms)");
    }
    for n in &notes {
        println!("    {n}");
    }
}
