//! `Caps` + arrastrar (hito 5): la parte del modelo, con respuestas a mano.

use crate::Rect;
use crate::drag::Mode;
use crate::mouse::Button;
use crate::place::MonitorObs;
use crate::world::{Command, DragReport, Effect, Event, Grab, Obs, WinKey, World};

const WIN: WinKey = WinKey { hwnd: 7, pid: 70 };

fn obs(frame: Rect, maximized: bool) -> Obs {
    let m = Rect::new(0, 0, 1920, 1080);
    Obs {
        key: WIN,
        app: "brave".into(),
        title: "x".into(),
        frame,
        outer: frame,
        dpi: 96,
        maximized,
        minimized: false,
        monitors: vec![MonitorObs { bounds: m, work: Rect::new(0, 0, 1920, 1040), dpi: 96 }],
        no_fly: None,
        pointer: None,
    }
}

fn grab(frame: Rect, maximized: bool) -> Grab {
    Grab { obs: obs(frame, maximized), restore: maximized.then_some((800, 600)), resizable: true }
}

fn press(w: &mut World, button: Button, at: (i32, i32), now: u64) -> Vec<Effect> {
    w.step(Event::Pointer { button, down: true, x: at.0, y: at.1 }, now)
}

fn release(w: &mut World, button: Button, now: u64) -> Vec<Effect> {
    w.step(Event::Pointer { button, down: false, x: 0, y: 0 }, now)
}

fn inspect_req(e: &[Effect]) -> u64 {
    e.iter()
        .find_map(|e| match e {
            Effect::InspectAt { req, .. } => Some(*req),
            _ => None,
        })
        .expect("InspectAt")
}

#[test]
fn drag_to_move_and_undo() {
    let mut w = World::new(Vec::new());
    let frame = Rect::new(100, 100, 900, 700);
    let req = inspect_req(&press(&mut w, Button::Left, (500, 120), 1_000_000));
    let e = w.step(Event::Grabbed { req, result: Ok(grab(frame, false)) }, 1_002_000);
    assert!(e.contains(&Effect::Raise { win: WIN }));
    assert!(e.contains(&Effect::DragStart {
        win: WIN,
        mode: Mode::Move,
        grab: (500, 120),
        restore: None
    }));
    let e = release(&mut w, Button::Left, 1_500_000);
    assert_eq!(e, vec![Effect::DragStop { win: WIN }]);
    let moved = Rect::new(300, 200, 1100, 800);
    let report = DragReport { moved: true, frame: Some(moved), ms: 500 };
    w.step(Event::DragDone { win: WIN, result: Ok(report) }, 1_510_000);
    assert!(w.can_undo(), "el arrastre entra al diario");
    assert_eq!(w.deadline(), None);
    // Z: pide la ventana para devolverla.
    let e = w.step(Event::Command(Command::Undo), 2_000_000);
    assert!(e.iter().any(|e| matches!(e, Effect::Inspect { win: Some(WIN), .. })), "{e:?}");
}

#[test]
fn z_right_after_releasing_waits_for_the_drag_to_finish() {
    let mut w = World::new(Vec::new());
    let frame = Rect::new(100, 100, 900, 700);
    let req = inspect_req(&press(&mut w, Button::Left, (500, 120), 1_000_000));
    w.step(Event::Grabbed { req, result: Ok(grab(frame, false)) }, 1_002_000);
    release(&mut w, Button::Left, 1_500_000);
    // Z antes de que llegue el fin del arrastre: espera.
    let e = w.step(Event::Command(Command::Undo), 1_501_000);
    assert!(e.is_empty(), "{e:?}");
    let report = DragReport { moved: true, frame: Some(Rect::new(300, 200, 1100, 800)), ms: 500 };
    let e = w.step(Event::DragDone { win: WIN, result: Ok(report) }, 1_510_000);
    assert!(e.iter().any(|e| matches!(e, Effect::Inspect { win: Some(WIN), .. })), "{e:?}");
}

/// Prueba del usuario: maximizada en el monitor 1, arrastrada al 2; Z la
/// vuelve a maximizar en el 1, no en el 2.
#[test]
fn undo_maximizes_back_on_the_original_monitor() {
    let mut w = World::new(Vec::new());
    let max1 = Rect::new(0, 0, 1920, 1040);
    let req = inspect_req(&press(&mut w, Button::Left, (960, 10), 1_000_000));
    w.step(Event::Grabbed { req, result: Ok(grab(max1, true)) }, 1_001_000);
    release(&mut w, Button::Left, 1_500_000);
    let report = DragReport { moved: true, frame: None, ms: 500 };
    w.step(Event::DragDone { win: WIN, result: Ok(report) }, 1_510_000);
    let e = w.step(Event::Command(Command::Undo), 2_000_000);
    let Some(req) = e.iter().find_map(|e| match e {
        Effect::Inspect { req, win: Some(WIN) } => Some(*req),
        _ => None,
    }) else {
        panic!("{e:?}")
    };
    // Ahora está, normal, en el monitor 2.
    let mut o = obs(Rect::new(2200, 100, 3000, 700), false);
    o.monitors.push(MonitorObs {
        bounds: Rect::new(1920, 0, 3840, 1080),
        work: Rect::new(1920, 0, 3840, 1040),
        dpi: 96,
    });
    let e = w.step(Event::Inspected { req, result: Ok(o) }, 2_001_000);
    let placed =
        e.iter().position(|e| matches!(e, Effect::Place { rect, .. } if rect.right <= 1920));
    let maxed = e.iter().position(|e| matches!(e, Effect::Maximize { win: WIN }));
    assert!(placed.is_some() && maxed.is_some() && placed < maxed, "{e:?}");
}

#[test]
fn a_still_click_is_not_recorded() {
    let mut w = World::new(Vec::new());
    let req = inspect_req(&press(&mut w, Button::Left, (500, 120), 1_000_000));
    w.step(Event::Grabbed { req, result: Ok(grab(Rect::new(0, 0, 800, 600), false)) }, 1_001_000);
    release(&mut w, Button::Left, 1_100_000);
    let report = DragReport { moved: false, frame: None, ms: 100 };
    w.step(Event::DragDone { win: WIN, result: Ok(report) }, 1_101_000);
    assert!(!w.can_undo());
}

#[test]
fn releasing_before_the_window_is_known_starts_nothing() {
    let mut w = World::new(Vec::new());
    let req = inspect_req(&press(&mut w, Button::Left, (5, 5), 1_000_000));
    assert!(release(&mut w, Button::Left, 1_001_000).is_empty());
    let e = w.step(
        Event::Grabbed { req, result: Ok(grab(Rect::new(0, 0, 800, 600), false)) },
        1_002_000,
    );
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(w.deadline(), None);
}

#[test]
fn right_drag_resizes_from_the_nearest_corner_but_not_a_maximized_window() {
    let mut w = World::new(Vec::new());
    let frame = Rect::new(100, 100, 900, 700);
    let req = inspect_req(&press(&mut w, Button::Right, (850, 650), 1_000_000));
    let e = w.step(Event::Grabbed { req, result: Ok(grab(frame, false)) }, 1_001_000);
    assert!(e.iter().any(|e| matches!(
        e,
        Effect::DragStart { mode: Mode::Resize { left: false, top: false }, .. }
    )));
    release(&mut w, Button::Right, 1_100_000);
    w.step(
        Event::DragDone { win: WIN, result: Ok(DragReport { moved: false, frame: None, ms: 1 }) },
        1_101_000,
    );

    let req = inspect_req(&press(&mut w, Button::Right, (850, 650), 2_000_000));
    let e = w.step(
        Event::Grabbed { req, result: Ok(grab(Rect::new(0, 0, 1920, 1040), true)) },
        2_001_000,
    );
    assert!(!e.iter().any(|e| matches!(e, Effect::DragStart { .. })), "{e:?}");
    assert!(release(&mut w, Button::Right, 2_100_000).is_empty());
}

#[test]
fn a_maximized_window_is_restored_when_dragged() {
    let mut w = World::new(Vec::new());
    let req = inspect_req(&press(&mut w, Button::Left, (960, 10), 1_000_000));
    let e = w.step(
        Event::Grabbed { req, result: Ok(grab(Rect::new(0, 0, 1920, 1040), true)) },
        1_001_000,
    );
    assert!(e.iter().any(|e| matches!(e, Effect::DragStart { restore: Some((800, 600)), .. })));
}

#[test]
fn a_keyboard_reset_ends_the_drag_once() {
    let mut w = World::new(Vec::new());
    let req = inspect_req(&press(&mut w, Button::Left, (500, 120), 1_000_000));
    w.step(Event::Grabbed { req, result: Ok(grab(Rect::new(0, 0, 800, 600), false)) }, 1_001_000);
    let e = w.step(Event::KeyboardReset, 1_200_000);
    assert!(e.contains(&Effect::DragStop { win: WIN }));
    assert!(release(&mut w, Button::Left, 1_300_000).is_empty(), "el arriba tardío no repite");
}

#[test]
fn keys_during_a_drag_are_ignored_and_the_cap_resets() {
    let mut w = World::new(Vec::new());
    let req = inspect_req(&press(&mut w, Button::Left, (500, 120), 1_000_000));
    w.step(Event::Grabbed { req, result: Ok(grab(Rect::new(0, 0, 800, 600), false)) }, 1_001_000);
    let action = crate::keyboard::HyperAction::Letter { letter: b'A', thumb: true };
    let e = w.step(Event::Action { action, held: 0, age_ms: 0 }, 1_100_000);
    assert!(!e.iter().any(|e| matches!(e, Effect::Inspect { .. })), "{e:?}");
    // Se perdió el "arriba": a los 60 s se corta y se reinicia el teclado.
    let due = w.deadline().expect("tope");
    let e = w.step(Event::Tick, due);
    assert!(e.contains(&Effect::DragStop { win: WIN }));
    assert!(e.contains(&Effect::ResetKeyboard));
}

/// 10 000 secuencias al azar: todo `DragStart` tiene exactamente un
/// `DragStop`, y al final no queda nada pendiente.
#[test]
fn every_drag_start_has_one_stop() {
    let mut seed: u64 = 0x0064_7261_6732;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..10_000 {
        let mut w = World::new(Vec::new());
        let mut now = 1_000_000u64;
        let mut open = 0i32; // DragStart sin DragStop
        let mut inspects: Vec<u64> = Vec::new();
        let mut running: Vec<WinKey> = Vec::new();
        let mut down: Option<Button> = None;
        let track =
            |e: Vec<Effect>, open: &mut i32, inspects: &mut Vec<u64>, running: &mut Vec<WinKey>| {
                for x in e {
                    match x {
                        Effect::DragStart { win, .. } => {
                            *open += 1;
                            running.push(win);
                        }
                        Effect::DragStop { .. } => *open -= 1,
                        Effect::InspectAt { req, .. } => inspects.push(req),
                        _ => {}
                    }
                    assert!((0..=1).contains(open), "arrastres abiertos: {open}");
                }
            };
        for _ in 0..12 {
            now += 10_000;
            let e = match next() % 5 {
                0 if down.is_none() => {
                    let b = if next() % 2 == 0 { Button::Left } else { Button::Right };
                    down = Some(b);
                    press(&mut w, b, (300, 300), now)
                }
                1 if down.is_some() => release(&mut w, down.take().unwrap(), now),
                2 if !inspects.is_empty() => {
                    let req = inspects.remove(0);
                    let r = if next() % 4 == 0 {
                        Err("nada".into())
                    } else {
                        Ok(grab(Rect::new(100, 100, 900, 700), next() % 5 == 0))
                    };
                    w.step(Event::Grabbed { req, result: r }, now)
                }
                3 if !running.is_empty() && open == 0 => {
                    let win = running.remove(0);
                    let report = DragReport { moved: next() % 2 == 0, frame: None, ms: 5 };
                    w.step(Event::DragDone { win, result: Ok(report) }, now)
                }
                4 => {
                    down = None; // el reinicio también suelta la máquina del hook
                    w.step(Event::KeyboardReset, now)
                }
                _ => Vec::new(),
            };
            track(e, &mut open, &mut inspects, &mut running);
        }
        // Soltar y terminar lo que quede.
        if let Some(b) = down.take() {
            track(release(&mut w, b, now + 1), &mut open, &mut inspects, &mut running);
        }
        for win in running.drain(..) {
            let report = DragReport { moved: false, frame: None, ms: 1 };
            w.step(Event::DragDone { win, result: Ok(report) }, now + 2);
        }
        assert_eq!(open, 0);
        assert_eq!(w.deadline(), None, "nada pendiente al final");
    }
}
