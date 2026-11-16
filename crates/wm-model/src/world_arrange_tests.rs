//! Pruebas de la capa de ventana de [`World`] (hitos 1a y 1b, semántica por
//! estado de plan-core.md §3.1) con una plataforma simulada: ventanas con
//! bordes invisibles que crecen con la escala, tamaño mínimo, maximizadas,
//! apps que tardan en moverse y que se reescalan al cruzar a un monitor con
//! otro DPI. La simulación entrega los eventos en orden de llegada, y puede
//! recibir varias teclas antes de que Windows responda a la primera.

use std::collections::VecDeque;

use crate::keyboard::{HELD_A, HELD_D, HELD_S, HELD_W, HyperAction};
use crate::place::{MonitorObs, monitor_of};
use crate::world::{Effect, Event, Obs, WinKey, World};
use crate::{Placement, Rect, place};

const MS: u64 = 1000;

/// Una ventana simulada.
#[derive(Clone)]
struct FakeWin {
    key: WinKey,
    frame: Rect,
    /// Bordes invisibles a 96 DPI (izquierda, arriba, derecha, abajo).
    margins: (i32, i32, i32, i32),
    /// DPI con el que se dibuja ahora (los bordes crecen con la escala).
    dpi: u32,
    maximized: bool,
    min_width: i32,
    /// Milisegundos que tarda la app en aplicar un pedido.
    lag_ms: u64,
}

impl FakeWin {
    fn margins_now(&self) -> (i32, i32, i32, i32) {
        let s = |m: i32| m * self.dpi as i32 / 96;
        let (l, t, r, b) = self.margins;
        (s(l), s(t), s(r), s(b))
    }

    fn outer(&self) -> Rect {
        let (l, t, r, b) = self.margins_now();
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
    /// Cambios de DPI pendientes (cuándo, DPI nuevo): llegan después de
    /// cruzar a un monitor con otra escala.
    dpi_queue: Vec<(u64, u32)>,
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
            dpi: 96,
            maximized: false,
            min_width: 0,
            lag_ms: 0,
        };
        Self {
            now: 1_000 * MS,
            win,
            monitors: vec![monitor(0), monitor(1920)],
            queued: Vec::new(),
            dpi_queue: Vec::new(),
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
            dpi: self.win.dpi,
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
            let (l, t, r, b) = self.win.margins_now();
            let mut frame =
                Rect::new(outer.left + l, outer.top + t, outer.right - r, outer.bottom - b);
            if frame.width() < self.win.min_width {
                frame.right = frame.left + self.win.min_width;
            }
            self.win.frame = frame;
            self.win.maximized = false;
            // Cruzó a un monitor con otra escala: Windows le avisa un poco después.
            if let Some(m) = monitor_of(&self.monitors, frame)
                && self.monitors[m].dpi != self.win.dpi
            {
                self.dpi_queue.push((now + 10 * MS, self.monitors[m].dpi));
            }
        }
        let (ready, later): (Vec<_>, Vec<_>) =
            self.dpi_queue.drain(..).partition(|(t, _)| *t <= now);
        self.dpi_queue = later;
        for (_, dpi) in ready {
            // La app se reescala: el rectángulo exterior queda y los bordes
            // invisibles crecen, así que el marco se ajusta.
            let outer = self.win.outer();
            self.win.dpi = dpi;
            let (l, t, r, b) = self.win.margins_now();
            self.win.frame =
                Rect::new(outer.left + l, outer.top + t, outer.right - r, outer.bottom - b);
        }
    }

    /// Corre `event` y todo lo que desencadena, hasta que no quede nada.
    fn run(&mut self, w: &mut World, event: Event) {
        self.run_many(w, vec![event]);
    }

    /// Como `run`, pero con varios eventos que llegan a la vez (antes de que
    /// Windows responda al primero): teclas "juntas".
    fn run_many(&mut self, w: &mut World, events: Vec<Event>) {
        let mut queue: VecDeque<Event> = events.into();
        loop {
            while let Some(e) = queue.pop_front() {
                self.settle();
                for effect in w.step(e, self.now) {
                    match effect {
                        Effect::Inspect { req, win } => {
                            assert!(win.is_none() || win == Some(self.win.key));
                            queue.push_back(Event::Inspected { req, result: Ok(self.obs()) });
                        }
                        Effect::Place { req, win, rect } => {
                            assert_eq!(win, self.win.key);
                            self.places += 1;
                            self.queued.push((self.now + self.win.lag_ms * MS, rect));
                            queue.push_back(Event::Placed { req, result: Ok(()) });
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
                    queue.push_back(Event::Tick);
                }
                None => break,
            }
        }
    }

    /// Una tecla sola y soltar el pulgar (el gesto termina).
    fn key(&mut self, w: &mut World, letter: u8) {
        self.run(w, arrange(letter, bit(letter)));
        self.run(w, Event::Layer(Layer::Apps));
        self.run(w, Event::Layer(Layer::Window));
    }

    /// Un acorde: las teclas en ese orden, cada una con las anteriores todavía
    /// apretadas. `together`: llegan todas antes de que Windows responda.
    fn chord(&mut self, w: &mut World, letters: &[u8], together: bool) {
        let mut held = 0;
        let events: Vec<Event> = letters
            .iter()
            .map(|&l| {
                held |= bit(l);
                arrange(l, held)
            })
            .collect();
        if together {
            self.run_many(w, events);
        } else {
            for e in events {
                self.run(w, e);
                self.now += 30 * MS;
            }
        }
        self.run(w, Event::Layer(Layer::Apps));
        self.run(w, Event::Layer(Layer::Window));
    }

    fn at(&mut self, m: usize, p: Placement) {
        self.win.frame = place(self.monitors[m].work, p);
    }

    fn is_at(&self, m: usize, p: Placement) -> bool {
        self.win.frame == place(self.monitors[m].work, p)
    }
}

fn bit(letter: u8) -> u8 {
    match letter {
        b'W' => HELD_W,
        b'A' => HELD_A,
        b'S' => HELD_S,
        b'D' => HELD_D,
        _ => 0,
    }
}

fn arrange(letter: u8, held: u8) -> Event {
    Event::Action { action: HyperAction::Letter { letter, thumb: true }, held, age_ms: 3 }
}

fn work() -> Rect {
    monitor(0).work
}

use crate::keyboard::Layer;
use Placement as P;

// ---- Una tecla sola ----

#[test]
fn left_half_lands_exactly_and_the_brain_goes_back_to_sleep() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.win.frame, place(work(), P::LeftHalf));
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
    sim.key(&mut w, b'D');
    assert!(sim.is_at(1, P::RightHalf));
}

#[test]
fn a_corner_by_state_no_matter_how_long_after() {
    // Sin plazo: "W y, cuando sea, A" da la esquina por dónde está la ventana.
    for gap_ms in [100, 5_000] {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new();
        sim.key(&mut w, b'W');
        assert!(sim.is_at(0, P::TopHalf));
        sim.now += gap_ms * MS;
        sim.key(&mut w, b'A');
        assert!(sim.is_at(0, P::TopLeft), "a los {gap_ms} ms: {:?}", sim.logs);
    }
}

#[test]
fn if_the_window_was_moved_by_hand_it_counts_from_where_it_is() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.key(&mut w, b'W');
    sim.win.frame = Rect::new(500, 300, 900, 700); // el usuario la movió con el mouse
    sim.key(&mut w, b'A');
    assert!(sim.is_at(0, P::LeftHalf), "libre + A = mitad izquierda");
}

#[test]
fn maximized_windows_are_restored_first() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.maximized = true;
    sim.win.frame = work();
    sim.key(&mut w, b'D');
    assert!(sim.normalized);
    assert!(sim.is_at(0, P::RightHalf));
}

#[test]
fn slow_apps_are_checked_until_they_arrive() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.lag_ms = 100; // llega entre el chequeo de 50 y el de 120 ms
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert!(sim.is_at(0, P::LeftHalf));
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
fn a_window_that_could_not_shrink_still_counts_as_where_we_left_it() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.min_width = 1200;
    sim.key(&mut w, b'A'); // "mitad izquierda", aunque quedó más ancha
    sim.win.min_width = 0;
    sim.key(&mut w, b'W'); // desde la mitad izquierda: esquina, no "libre + W"
    assert!(sim.is_at(0, P::TopLeft), "{:?}", sim.logs);
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
fn the_last_key_wins_and_counts_from_where_the_first_was_going() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.lag_ms = 30;
    // Dos teclas seguidas antes de que la app llegue a la primera.
    let effects = w.step(arrange(b'A', HELD_A), sim.now);
    let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
    let effects = w.step(Event::Inspected { req, result: Ok(sim.obs()) }, sim.now);
    assert!(matches!(effects[0], Effect::Place { .. }));
    sim.run(&mut w, arrange(b'W', HELD_W));
    assert_eq!(w.placing(), 0);
    assert!(sim.is_at(0, P::TopLeft), "A va a la izquierda; W desde ahí = esquina");
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

// ---- Empujar al monitor vecino ----

#[test]
fn pushing_past_the_last_monitor_does_nothing() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf); // monitor de más a la izquierda
    sim.key(&mut w, b'A');
    assert_eq!(sim.places, 0);
    assert!(sim.logs[0].contains("no hay otro monitor a la izquierda"), "{:?}", sim.logs);
}

#[test]
fn repeating_a_direction_pushes_to_the_next_monitor_and_back() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.key(&mut w, b'D'); // mitad derecha del monitor 0
    sim.key(&mut w, b'D'); // empuja: mitad izquierda del monitor 1
    assert!(sim.is_at(1, P::LeftHalf));
    assert!(sim.logs[1].contains("monitor de la derecha, mitad izquierda"), "{:?}", sim.logs);
    sim.key(&mut w, b'A'); // vuelve: mitad derecha del monitor 0
    assert!(sim.is_at(0, P::RightHalf));
    sim.key(&mut w, b'A'); // dentro del monitor 0
    assert!(sim.is_at(0, P::LeftHalf));
}

#[test]
fn crossing_to_a_higher_dpi_waits_for_the_rescale() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.monitors[1].dpi = 144; // el de la derecha a 150 %
    sim.at(0, P::RightHalf);
    sim.key(&mut w, b'D');
    assert_eq!(sim.win.dpi, 144);
    assert!(sim.is_at(1, P::LeftHalf));
    assert_eq!(sim.places, 1, "los bordes ya iban escalados: una sola pasada");
    assert!(sim.logs[0].contains("0 px en 1 pasada"), "{:?}", sim.logs);
}

// ---- Los casos que encontró el usuario ----

#[test]
fn user_case_1_a_corner_moves_as_a_corner() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopRight);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(1, P::TopLeft), "{:?}", sim.logs);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(1, P::TopRight), "una esquina sigue siendo esquina: {:?}", sim.logs);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(1, P::TopRight));
    assert!(sim.logs.last().unwrap().contains("no hay otro monitor a la derecha"));
    sim.key(&mut w, b'A');
    sim.key(&mut w, b'A');
    assert!(sim.is_at(0, P::TopRight), "{:?}", sim.logs);
}

#[test]
fn user_case_2_a_chord_does_not_depend_on_which_finger_lands_first() {
    for order in [*b"WA", *b"AW"] {
        for together in [true, false] {
            let mut w = World::new(Vec::new());
            let mut sim = Sim::new();
            sim.at(1, P::TopLeft);
            sim.chord(&mut w, &order, together);
            assert!(
                sim.is_at(0, P::TopRight),
                "orden {:?}, juntas {together}: {:?}",
                order.map(|c| c as char),
                sim.logs
            );
        }
    }
}

#[test]
fn user_case_3_a_pushed_half_plus_w_is_the_top_corner_of_that_side() {
    for gap_ms in [100, 5_000] {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new();
        sim.at(0, P::RightHalf);
        sim.key(&mut w, b'D'); // mitad izquierda del monitor 1
        sim.now += gap_ms * MS;
        sim.key(&mut w, b'W');
        assert!(sim.is_at(1, P::TopLeft), "a los {gap_ms} ms: {:?}", sim.logs);
        assert!(!sim.logs.iter().any(|l| l.contains("no hay otro monitor")), "{:?}", sim.logs);
    }
}

#[test]
fn a_single_w_against_the_top_says_so_but_not_inside_a_chord() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(1, P::TopLeft);
    sim.key(&mut w, b'W');
    assert!(sim.logs.last().unwrap().contains("no hay otro monitor arriba"), "{:?}", sim.logs);
    let before = sim.logs.len();
    sim.chord(&mut w, b"WA", true); // W primero dentro de un acorde
    assert!(sim.is_at(0, P::TopRight));
    assert!(!sim.logs[before..].iter().any(|l| l.contains("arriba.")), "{:?}", sim.logs);
}

#[test]
fn opposite_keys_together_cancel() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopLeft);
    sim.chord(&mut w, b"AD", true);
    assert!(sim.is_at(0, P::TopLeft), "A + D no hace nada: {:?}", sim.logs);
}

#[test]
fn a_chord_on_its_own_corner_without_neighbor_stays() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopLeft); // monitor de la izquierda, sin nada arriba
    sim.chord(&mut w, b"WA", true);
    assert!(sim.is_at(0, P::TopLeft));
}

// ---- Propiedad: el orden de los dedos no importa ----

#[test]
fn every_chord_gives_the_same_result_in_every_order() {
    let origins = [
        None, // libre
        Some(P::LeftHalf),
        Some(P::RightHalf),
        Some(P::TopHalf),
        Some(P::BottomHalf),
        Some(P::TopLeft),
        Some(P::TopRight),
        Some(P::BottomLeft),
        Some(P::BottomRight),
    ];
    let chords: [&[u8]; 10] =
        [b"WA", b"WD", b"SA", b"SD", b"AD", b"WS", b"WAD", b"SAD", b"AWS", b"DWS"];
    for m in 0..2 {
        for origin in origins {
            for keys in chords {
                for lag in [0, 100] {
                    let mut results = Vec::new();
                    for order in permutations(keys) {
                        for together in [true, false] {
                            let mut w = World::new(Vec::new());
                            let mut sim = Sim::new();
                            sim.win.lag_ms = lag;
                            match origin {
                                Some(p) => sim.at(m, p),
                                None => {
                                    let x = sim.monitors[m].work.left + 300;
                                    sim.win.frame = Rect::new(x, 200, x + 800, 800);
                                }
                            }
                            sim.chord(&mut w, &order, together);
                            results.push((order.clone(), together, sim.win.frame));
                        }
                    }
                    let first = results[0].2;
                    for (order, together, frame) in &results {
                        assert_eq!(
                            *frame,
                            first,
                            "monitor {m}, origen {origin:?}, acorde {:?}, demora {lag}: orden {:?} juntas {together}",
                            String::from_utf8_lossy(keys),
                            String::from_utf8_lossy(order),
                        );
                    }
                }
            }
        }
    }
}

fn permutations(keys: &[u8]) -> Vec<Vec<u8>> {
    if keys.len() <= 1 {
        return vec![keys.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..keys.len() {
        let mut rest = keys.to_vec();
        let k = rest.remove(i);
        for mut tail in permutations(&rest) {
            tail.insert(0, k);
            out.push(tail);
        }
    }
    out
}
