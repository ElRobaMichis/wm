//! `wm display-watch [restore]` — spike 7 (architecture.md §8 y §13.9): se
//! deja corriendo mientras el usuario conecta y desconecta monitores, cambia
//! la escala, suspende o bloquea la PC. Es el prototipo del motor de
//! persistencia elegido: **una instantánea por topología y reconciliar
//! después de la tormenta**.
//!
//! - Registra cada aviso de Windows con la hora.
//! - Cuando los avisos se calman (800 ms sin ninguno y la sesión
//!   desbloqueada), revisa qué monitores hay (identidad: EDID + serie +
//!   conector, emparejados por niveles) y si alguna ventana quedó fuera de
//!   pantalla (regla pura `screens::reachable`); si quedó, la rescata con el
//!   desplazamiento mínimo.
//! - Con `restore`: guarda el layout de cada combinación de monitores
//!   (posiciones relativas al área de trabajo, cada 2 s en calma) y, cuando
//!   vuelve una combinación conocida, devuelve cada ventana a su lugar.
//! - Imprime "ciclo N ok" o qué falló. Ctrl+C termina con el resumen.

use std::collections::{BTreeMap, HashMap};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wm_model::Rect;
use wm_model::screens::{MinVisible, MonitorId, RelRect, match_monitors, reachable, rescue};
use wm_platform::display_watch::{self, DisplayEvent};
use wm_platform::displays::{self, Display};
use wm_platform::windows_list::{self, WindowId};
use wm_platform::{clock, console, geometry, ghost};

const QUIET: Duration = Duration::from_millis(800);
const SNAPSHOT_EVERY: Duration = Duration::from_secs(2);

/// Layout guardado de una topología: por ventana, monitor e índice relativo.
struct Layout {
    monitors: Vec<MonitorId>,
    windows: HashMap<isize, (MonitorId, RelRect)>,
}

fn label(id: &MonitorId) -> String {
    let serial = id.serial.map_or("?".to_owned(), |s| s.to_string());
    format!("{:04x}:{:04x}/{serial}/{}", id.manufacturer, id.product, id.connector)
}

/// Ventanas de usuario visibles y no minimizadas, con su marco.
fn frames() -> Vec<(WindowId, String, Rect)> {
    windows_list::user_windows()
        .into_iter()
        .filter(|w| !wm_platform::hide::visibility(w.id).minimized)
        .filter_map(|w| Some((w.id, w.title, ghost::frame_bounds(w.id)?)))
        .collect()
}

fn snapshot(list: &[Display]) -> Layout {
    let mut windows = HashMap::new();
    for (id, _, frame) in frames() {
        let (cx, cy) = ((frame.left + frame.right) / 2, (frame.top + frame.bottom) / 2);
        let home = list.iter().find(|d| {
            let b = d.bounds;
            cx >= b.left && cx < b.right && cy >= b.top && cy < b.bottom
        });
        if let Some(d) = home {
            windows.insert(id.0, (d.id.clone(), RelRect::from_abs(frame, d.work_area)));
        }
    }
    Layout { monitors: list.iter().map(|d| d.id.clone()).collect(), windows }
}

/// ¿Es la misma combinación de monitores (todos emparejados, en los dos lados)?
fn same_topology(a: &[MonitorId], b: &[MonitorId]) -> bool {
    a.len() == b.len() && match_monitors(a, b).len() == a.len()
}

pub fn run(args: &[String]) -> ExitCode {
    let restore = args.iter().any(|a| a == "restore");
    let rx = match display_watch::start() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("No se pudo escuchar los avisos de Windows: {e}");
            return ExitCode::FAILURE;
        }
    };
    console::trap_stop_requests();
    let t0 = clock::now();
    let stamp = |q: i64| clock::micros(t0, q) as f64 / 1e6;

    let Ok(list) = displays::displays() else {
        eprintln!("No se pudo leer la configuración de pantallas.");
        return ExitCode::FAILURE;
    };
    println!(
        "wm display-watch{} — Ctrl+C para terminar.",
        if restore { " (recuerda y restaura el layout)" } else { "" }
    );
    for d in &list {
        println!("Monitor {}: {} a {} DPI", label(&d.id), d.name, d.dpi);
    }
    let mut layouts: Vec<Layout> = vec![snapshot(&list)];
    println!("Ventanas: {}", layouts[0].windows.len());
    println!();

    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut burst: Vec<&'static str> = Vec::new();
    let mut last_event: Option<Instant> = None;
    let mut last_snapshot = Instant::now();
    let mut locked = false;
    let (mut cycles, mut ok_cycles, mut rescued, mut restored, mut misplaced) = (0u32, 0, 0, 0, 0);
    let mut new_topologies = 0u32;
    while !console::stop_requested() {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((q, e)) => {
                *counts.entry(e.name()).or_default() += 1;
                if !burst.contains(&e.name()) {
                    burst.push(e.name());
                }
                match e {
                    DisplayEvent::Lock => locked = true,
                    DisplayEvent::Unlock => locked = false,
                    _ => {}
                }
                let extra = match e {
                    DisplayEvent::DisplayChange { width, height } => format!(" ({width}×{height})"),
                    _ => String::new(),
                };
                println!("[{:>8.2} s] {}{extra}", stamp(q), e.name());
                last_event = Some(Instant::now());
            }
            Err(_) => {
                let Some(t) = last_event else {
                    // En calma: actualizar el layout de la topología actual.
                    if restore && last_snapshot.elapsed() >= SNAPSHOT_EVERY {
                        if let Ok(list) = displays::displays() {
                            let snap = snapshot(&list);
                            let same = |l: &&mut Layout| same_topology(&l.monitors, &snap.monitors);
                            match layouts.iter_mut().find(same) {
                                Some(l) => *l = snap,
                                None => layouts.push(snap),
                            }
                        }
                        last_snapshot = Instant::now();
                    }
                    continue;
                };
                if t.elapsed() < QUIET {
                    continue;
                }
                if locked {
                    // Con la sesión bloqueada no se toca nada (Fable).
                    continue;
                }
                last_event = None;
                cycles += 1;
                // Dos lecturas iguales a 250 ms: la topología está quieta.
                let (Ok(a), b) = (displays::displays(), {
                    std::thread::sleep(Duration::from_millis(250));
                    displays::displays()
                }) else {
                    println!("  → calma, pero la configuración de pantallas no se pudo leer");
                    last_event = Some(Instant::now());
                    continue;
                };
                let Ok(list) = b else {
                    last_event = Some(Instant::now());
                    continue;
                };
                let ids: Vec<MonitorId> = list.iter().map(|d| d.id.clone()).collect();
                let ids_a: Vec<MonitorId> = a.iter().map(|d| d.id.clone()).collect();
                if !same_topology(&ids, &ids_a) {
                    println!("  → la configuración siguió cambiando; espero otra calma");
                    last_event = Some(Instant::now());
                    continue;
                }
                let known = layouts.iter().position(|l| same_topology(&l.monitors, &ids));
                new_topologies += u32::from(known.is_none());
                let names: Vec<String> = ids.iter().map(label).collect();
                println!(
                    "  → calma tras [{}]: monitores {} ({})",
                    burst.join(", "),
                    names.join(" + "),
                    if known.is_some() {
                        "configuración conocida"
                    } else {
                        "configuración NUEVA"
                    }
                );
                burst.clear();
                let mut problems = Vec::new();

                // Restaurar el layout de una configuración conocida.
                if restore && let Some(k) = known {
                    let saved = &layouts[k];
                    let pairs = match_monitors(&saved.monitors, &ids);
                    for (hwnd, (mon, rel)) in &saved.windows {
                        let Some(si) = saved.monitors.iter().position(|m| m == mon) else {
                            continue;
                        };
                        let Some(&(_, li)) = pairs.iter().find(|(s, _)| *s == si) else { continue };
                        let id = WindowId(*hwnd);
                        let Some(now) = ghost::frame_bounds(id) else { continue }; // ya no existe
                        if wm_platform::hide::visibility(id).minimized {
                            continue;
                        }
                        let want = rel.to_abs(list[li].work_area);
                        if geometry::edge_error(now, want) <= geometry::TOLERANCE_PX {
                            continue;
                        }
                        match geometry::place_frame(id, want) {
                            Some(r) if r.error <= geometry::TOLERANCE_PX => {
                                restored += 1;
                                println!(
                                    "     devuelta «{}» a su lugar ({:?})",
                                    windows_list::window_title(id),
                                    r.frame
                                );
                            }
                            Some(r) => {
                                misplaced += 1;
                                problems.push(format!(
                                    "«{}» quedó a {} px de su lugar",
                                    windows_list::window_title(id),
                                    r.error
                                ));
                            }
                            None => {}
                        }
                    }
                }

                // Nunca fuera de pantalla.
                let areas: Vec<Rect> = list.iter().map(|d| d.work_area).collect();
                let min = MinVisible::at_dpi(list.iter().map(|d| d.dpi).max().unwrap_or(96));
                for (id, title, frame) in frames() {
                    if reachable(frame, &areas, min) {
                        continue;
                    }
                    problems.push(format!("«{title}» estaba fuera de pantalla en {frame:?}"));
                    if let Some(to) = rescue(frame, &areas, min)
                        && geometry::place_frame(id, to).is_some()
                    {
                        rescued += 1;
                    }
                }

                if problems.is_empty() {
                    ok_cycles += 1;
                    println!("  ciclo {cycles} ok");
                } else {
                    println!("  ciclo {cycles} con problemas:");
                    for p in &problems {
                        println!("     {p}");
                    }
                }
                if known.is_none() {
                    layouts.push(snapshot(&list));
                }
                last_snapshot = Instant::now();
            }
        }
    }

    println!();
    println!("== Resumen");
    for (name, n) in &counts {
        println!("  {name}: {n} avisos");
    }
    println!("  ciclos: {cycles} ({ok_cycles} ok)");
    println!("  configuraciones de monitores nuevas: {new_topologies}");
    println!("  ventanas rescatadas de fuera de pantalla: {rescued}");
    if restore {
        println!("  ventanas devueltas a su lugar: {restored} (quedaron corridas: {misplaced})");
    }
    console::mark_finished();
    ExitCode::SUCCESS
}
