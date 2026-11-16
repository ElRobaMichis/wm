//! Pruebas de la capa de ventana de [`World`] (hito 1a) con una plataforma
//! simulada: ventanas con bordes invisibles, tamaño mínimo, maximizadas y
//! apps que tardan en moverse.

use crate::keyboard::{HELD_A, HELD_W, HyperAction};
use crate::place::MonitorObs;
use crate::world::{Effect, Event, Obs, WinKey, World};
use crate::{Placement, Rect, place};

const MS: u64 = 1000;

/// Una ventana simulada.
#[derive(Clone)]
struct FakeWin {
    key: WinKey,
    frame: Rect,
    /// Bordes invisibles (izquierda, arriba, derecha, abajo).
    margins: (i32, i32, i32, i32),
    maximized: bool,
    min_width: i32,
    /// Milisegundos que tarda la app en aplicar un pedido.
    lag_ms: u64,
}

impl FakeWin {
    fn outer(&self) -> Rect {
        let (l, t, r, b) = self.margins;
        Rect::new(
            self.frame.left - l,
            self.frame.top - t,
            self.frame.right + r,
            self.frame.bottom + b,
        )
    }
}

struct Sim {
    now: u64,
    win: FakeWin,
    monitors: Vec<MonitorObs>,
    /// Pedidos que la app todavía no aplicó: (cuándo, rect exterior).
    queued: Vec<(u64, Rect)>,
    logs: Vec<String>,
    places: usize,
    normalized: bool,
}

fn monitor(left: i32) -> MonitorObs {
    MonitorObs {
        bounds: Rect::new(left, 0, left + 1920, 1080),
        work: Rect::new(left, 0, left + 1920, 1032),
        dpi: 96,
    }
}

impl Sim {
    fn new() -> Self {
        let win = FakeWin {
            key: WinKey { hwnd: 7, pid: 70 },
            frame: Rect::new(300, 200, 1100, 800),
            margins: (7, 0, 7, 7),
            maximized: false,
            min_width: 0,
            lag_ms: 0,
        };
        Self {
            now: 1_000 * MS,
            win,
            monitors: vec![monitor(0), monitor(1920)],
            queued: Vec::new(),
            logs: Vec::new(),
            places: 0,
            normalized: false,
        }
    }

    fn obs(&self) -> Obs {
        Obs {
            key: self.win.key,
            app: "brave".into(),
            title: "Brave".into(),
            frame: self.win.frame,
            outer: self.win.outer(),
            dpi: 96,
            maximized: self.win.maximized,
            minimized: false,
            monitors: self.monitors.clone(),
        }
    }

    /// La app aplica los pedidos que ya le llegaron.
    fn settle(&mut self) {
        let now = self.now;
        let (ready, later): (Vec<_>, Vec<_>) = self.queued.drain(..).partition(|(t, _)| *t <= now);
        self.queued = later;
        for (_, outer) in ready {
            let (l, t, r, b) = self.win.margins;
            let mut frame =
                Rect::new(outer.left + l, outer.top + t, outer.right - r, outer.bottom - b);
            if frame.width() < self.win.min_width {
                frame.right = frame.left + self.win.min_width;
            }
            self.win.frame = frame;
            self.win.maximized = false;
        }
    }

    fn run(&mut self, w: &mut World, event: Event) {
        let mut queue = vec![event];
        loop {
            while let Some(e) = queue.pop() {
                self.settle();
                for effect in w.step(e, self.now) {
                    match effect {
                        Effect::Inspect { req, win } => {
                            assert!(win.is_none() || win == Some(self.win.key));
                            queue.push(Event::Inspected { req, result: Ok(self.obs()) });
                        }
                        Effect::Place { req, win, rect } => {
                            assert_eq!(win, self.win.key);
                            self.places += 1;
                            self.queued.push((self.now + self.win.lag_ms * MS, rect));
                            queue.push(Event::Placed { req, result: Ok(()) });
                        }
                        Effect::Normalize { win } => {
                            assert_eq!(win, self.win.key);
                            self.normalized = true;
                        }
                        Effect::Log(line) => self.logs.push(line),
                        other => panic!("efecto inesperado: {other:?}"),
                    }
                }
            }
            // Sin nada en cola: avanzar el reloj al próximo plazo, si hay.
            match w.deadline() {
                Some(t) => {
                    self.now = self.now.max(t);
                    queue.push(Event::Tick);
                }
                None => break,
            }
        }
    }
}

fn arrange(letter: u8, held: u8) -> Event {
    Event::Action { action: HyperAction::Letter { letter, thumb: true }, held, age_ms: 3 }
}

fn work() -> Rect {
    monitor(0).work
}

#[test]
fn left_half_lands_exactly_and_the_brain_goes_back_to_sleep() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.win.frame, place(work(), Placement::LeftHalf));
    assert_eq!(sim.places, 1);
    assert!(sim.logs[0].contains("mitad izquierda (brave): 0 px en 1 pasada"), "{:?}", sim.logs);
    assert_eq!(w.deadline(), None, "sin plazos pendientes: 0 % de CPU");
    assert_eq!(w.placing(), 0);
    assert_eq!(w.in_flight(), 0);
}

#[test]
fn the_window_stays_on_its_monitor() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = Rect::new(2200, 100, 3000, 700); // en el monitor de la derecha
    sim.run(&mut w, arrange(b'D', 0)); // D sola
    assert_eq!(sim.win.frame, place(monitor(1920).work, Placement::RightHalf));
}

#[test]
fn both_keys_held_make_a_corner() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.run(&mut w, arrange(b'A', HELD_W | HELD_A));
    assert_eq!(sim.win.frame, place(work(), Placement::TopLeft));
}

#[test]
fn a_perpendicular_key_within_a_second_makes_a_corner() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.run(&mut w, arrange(b'W', HELD_W));
    assert_eq!(sim.win.frame, place(work(), Placement::TopHalf));
    sim.now += 300 * MS;
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.win.frame, place(work(), Placement::TopLeft));
}

#[test]
fn after_a_second_it_is_a_new_half_not_a_corner() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.run(&mut w, arrange(b'W', HELD_W));
    sim.now += 1_500 * MS;
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.win.frame, place(work(), Placement::LeftHalf));
}

#[test]
fn if_the_window_was_moved_in_between_it_is_not_a_corner() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.run(&mut w, arrange(b'W', HELD_W));
    sim.win.frame = Rect::new(500, 300, 900, 700); // el usuario la movió con el mouse
    sim.now += 200 * MS;
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.win.frame, place(work(), Placement::LeftHalf));
}

#[test]
fn maximized_windows_are_restored_first() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.maximized = true;
    sim.win.frame = work();
    sim.run(&mut w, arrange(b'D', 0));
    assert!(sim.normalized);
    assert_eq!(sim.win.frame, place(work(), Placement::RightHalf));
}

#[test]
fn slow_apps_are_checked_until_they_arrive() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.lag_ms = 100; // llega entre el chequeo de 50 y el de 120 ms
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.win.frame, place(work(), Placement::LeftHalf));
    assert_eq!(sim.places, 1, "no hizo falta una segunda pasada");
    assert!(sim.logs[0].contains("0 px en 1 pasada"));
}

#[test]
fn minimum_size_is_accepted_after_a_second_pass() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.min_width = 1200; // más ancha que media pantalla
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.places, 2);
    assert_eq!(sim.win.frame.width(), 1200);
    assert!(sim.logs[0].contains("tamaño mínimo"), "{:?}", sim.logs);
    assert_eq!(w.deadline(), None);
}

#[test]
fn already_there_does_nothing() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = place(work(), Placement::LeftHalf);
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.places, 0);
    assert!(sim.logs[0].contains("ya estaba ahí"));
}

#[test]
fn full_screen_windows_are_left_alone() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = monitor(0).bounds;
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.places, 0);
    assert!(sim.logs[0].contains("pantalla completa"));
}

#[test]
fn the_last_key_wins() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.lag_ms = 30;
    // Dos teclas seguidas antes de que la app llegue a la primera.
    let effects = w.step(arrange(b'A', HELD_A), sim.now);
    let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
    let effects = w.step(Event::Inspected { req, result: Ok(sim.obs()) }, sim.now);
    assert!(matches!(effects[0], Effect::Place { .. }));
    sim.run(&mut w, arrange(b'D', 0));
    assert_eq!(w.placing(), 0);
    assert_eq!(sim.win.frame, place(work(), Placement::RightHalf));
    assert_eq!(sim.logs.len(), 1, "solo se informa la última: {:?}", sim.logs);
}

#[test]
fn a_window_that_disappears_ends_the_job() {
    let mut w = World::new(Vec::new());
    let sim = Sim::new();
    let effects = w.step(arrange(b'A', HELD_A), sim.now);
    let Effect::Inspect { req, .. } = effects[0] else { panic!() };
    w.step(Event::Inspected { req, result: Ok(sim.obs()) }, sim.now);
    let t = w.deadline().expect("hay un chequeo");
    let effects = w.step(Event::Tick, t);
    let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
    let effects =
        w.step(Event::Inspected { req, result: Err("la ventana ya no existe".into()) }, t);
    assert!(matches!(&effects[0], Effect::Log(l) if l.contains("ya no existe")));
    assert_eq!(w.deadline(), None);
}
