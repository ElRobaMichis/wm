//! `wm place-test <puppet N | app NOMBRE> [cross [n]] [close]` — spike 7
//! (architecture.md §8): coloca una ventana por su marco visible en mitades,
//! cuartos, tercios y "full" de cada monitor, y mide el error de cada borde,
//! las pasadas y si la app se movió sola después (+250 ms).
//!
//! Con `cross`, la pasa `n` veces (20 por defecto) de la mitad izquierda del
//! primer monitor a la derecha del segundo y vuelta: con escalas distintas,
//! mide el cruce de DPI. Al final la deja como estaba (o la cierra, con `close`).

use std::process::ExitCode;
use std::time::Duration;

use wm_model::{Placement, Rect, place};
use wm_platform::geometry::{self, TOLERANCE_PX};
use wm_platform::windows_list::{self, WindowId};

fn target(args: &[String]) -> Option<WindowId> {
    match args.first().map(String::as_str) {
        Some("puppet") => {
            let n: usize = args.get(1)?.parse().ok()?;
            wm_platform::puppet::find_puppets().get(n.checked_sub(1)?).copied()
        }
        Some("app") => {
            let name = args.get(1)?.to_lowercase();
            windows_list::user_windows().into_iter().find(|w| w.app == name).map(|w| w.id)
        }
        _ => None,
    }
}

fn thirds(area: Rect) -> [(&'static str, Rect); 3] {
    let (a, b) = (area.split_x(1, 3), area.split_x(2, 3));
    [
        ("tercio izq.", Rect { right: a, ..area }),
        ("tercio medio", Rect { left: a, right: b, ..area }),
        ("tercio der.", Rect { left: b, ..area }),
    ]
}

pub fn run(args: &[String]) -> ExitCode {
    let Some(id) = target(args) else {
        eprintln!("Uso: wm place-test <puppet N | app NOMBRE> [close]");
        return ExitCode::from(2);
    };
    let close = args.iter().any(|a| a == "close");
    let cross: Option<usize> = args.iter().position(|a| a == "cross").map(|i| {
        args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(20)
    });
    let was_max = wm_platform::hide::is_maximized(id);
    if was_max {
        wm_platform::hide::set_maximized_for_test(id, false);
        std::thread::sleep(Duration::from_millis(400));
    }
    let original = wm_platform::ghost::window_rect(id);
    let title = windows_list::window_title(id);
    println!("Ventana «{title}»");

    let placements = [
        ("full", Placement::Full),
        ("mitad izq.", Placement::LeftHalf),
        ("mitad der.", Placement::RightHalf),
        ("mitad arriba", Placement::TopHalf),
        ("mitad abajo", Placement::BottomHalf),
        ("cuarto ↖", Placement::TopLeft),
        ("cuarto ↘", Placement::BottomRight),
    ];
    let (mut total, mut ok, mut worst, mut two_pass) = (0, 0, 0, 0);
    let (mut crossings, mut drifted) = (0, 0);
    let mut times = Vec::new();
    let displays = wm_platform::displays::displays().unwrap_or_default();
    let mut record = |name: &str, r: geometry::Placed| {
        total += 1;
        ok += usize::from(r.error <= TOLERANCE_PX);
        worst = worst.max(r.error);
        two_pass += usize::from(r.passes > 1);
        crossings += usize::from(r.dpi_changed);
        drifted += usize::from(r.drift > TOLERANCE_PX);
        times.push(r.elapsed);
        let mark = if r.error <= TOLERANCE_PX { "ok" } else { "FUERA" };
        println!(
            "  {name:<13} error {} px en {} pasada(s), {:.0} ms{}{}  {mark}{}",
            r.error,
            r.passes,
            r.elapsed.as_secs_f64() * 1000.0,
            if r.dpi_changed { ", cambió de DPI" } else { "" },
            if r.drift > 0 { format!(", se movió sola {} px", r.drift) } else { String::new() },
            if r.error > TOLERANCE_PX { format!(" (quedó {:?})", r.frame) } else { String::new() }
        );
    };
    if let Some(n) = cross {
        let [a, b] = [0, 1].map(|i| displays.get(i).map(|d| (d.name.clone(), d.dpi, d.work_area)));
        let (Some(a), Some(b)) = (a, b) else {
            eprintln!("El modo cross necesita dos monitores.");
            return ExitCode::FAILURE;
        };
        println!("Cruce: {} ({} DPI) ↔ {} ({} DPI), {n} veces", a.0, a.1, b.0, b.1);
        for i in 0..n {
            let (name, rect) = if i % 2 == 0 {
                ("→ 2.º monitor", place(b.2, Placement::RightHalf))
            } else {
                ("→ 1.er monitor", place(a.2, Placement::LeftHalf))
            };
            if let Some(r) = geometry::place_frame(id, rect) {
                record(name, r);
            }
        }
    }
    for d in displays.iter().filter(|_| cross.is_none()) {
        println!("== {} ({} DPI), área {:?}", d.name, d.dpi, d.work_area);
        let mut targets: Vec<(&str, Rect)> =
            placements.iter().map(|(n, p)| (*n, place(d.work_area, *p))).collect();
        targets.extend(thirds(d.work_area));
        for (name, rect) in targets {
            match geometry::place_frame(id, rect) {
                Some(r) => record(name, r),
                None => println!("  {name:<13} no se pudo colocar"),
            }
        }
    }
    times.sort();
    let p95 = times.get(times.len().saturating_sub(1) * 95 / 100).copied().unwrap_or_default();
    println!();
    println!(
        "{ok}/{total} con error ≤ {TOLERANCE_PX} px; peor error {worst} px; \
         {two_pass} con 2 pasadas; p95 {:.0} ms",
        p95.as_secs_f64() * 1000.0
    );
    println!("Cruces de DPI: {crossings}; la app se movió sola después: {drifted}");

    if close {
        windows_list::close(id);
    } else if let Some(r) = original {
        let _ = windows_list::place_async(id, r);
        if was_max {
            std::thread::sleep(Duration::from_millis(300));
            wm_platform::hide::set_maximized_for_test(id, true);
        }
    }
    ExitCode::SUCCESS
}
