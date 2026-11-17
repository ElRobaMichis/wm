//! Pruebas de la capa de ventana de [`World`] (hitos 1a y 1b, semántica por
//! estado de plan-core.md §3.1) con una plataforma simulada: ventanas con
//! bordes invisibles que crecen con la escala, tamaño mínimo, maximizadas,
//! apps que tardan en moverse y que se reescalan al cruzar a un monitor con
//! otro DPI. La simulación entrega los eventos en orden de llegada, y puede
//! recibir varias teclas antes de que Windows responda a la primera.
//!
//! También simula el carril `anim` (hito 1e): un vuelo mueve la ventana real
//! al despegar y responde `Flown` cuando termina; un vuelo nuevo sobre la
//! misma ventana la redirige. La simulación verifica que nunca llegue otro
//! pedido sobre una ventana en vuelo sin `Land` antes.

use std::collections::VecDeque;

use crate::keyboard::{HELD_A, HELD_D, HELD_S, HELD_W, HyperAction};
use crate::place::{MonitorObs, monitor_of};
use crate::world::{Effect, Event, FlyFail, FlyReport, NoFly, Obs, WinKey, World};
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
    /// Tamaño fijo (la app no se deja redimensionar).
    fixed: Option<(i32, i32)>,
    /// Marco al que vuelve al restaurarla (Windows lo recuerda).
    restore: Rect,
    min_width: i32,
    /// Milisegundos que tarda la app en aplicar un pedido.
    lag_ms: u64,
    no_fly: Option<NoFly>,
}

/// Un vuelo en curso en el carril de mentira.
struct Flight {
    due: u64,
    req: u64,
    to: Rect,
    ms: u32,
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
    /// Avisos en pantalla (hito de opciones).
    notices: Vec<String>,
    places: usize,
    normalized: bool,
    flight: Option<Flight>,
    flights: usize,
    lands: usize,
    /// El carril responde así a todo vuelo (sin animación).
    fly_fail: Option<FlyFail>,
    /// El `Scan` dice que la ventana no está al frente (apareció detrás).
    behind: bool,
    /// Tantos `Scan` más no ven la ventana (todavía sin mostrarse).
    unlisted: usize,
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
            fixed: None,
            restore: Rect::new(300, 200, 1100, 800),
            min_width: 0,
            lag_ms: 0,
            no_fly: None,
        };
        Self {
            now: 1_000 * MS,
            win,
            monitors: vec![monitor(0), monitor(1920)],
            queued: Vec::new(),
            dpi_queue: Vec::new(),
            logs: Vec::new(),
            notices: Vec::new(),
            places: 0,
            normalized: false,
            flight: None,
            flights: 0,
            lands: 0,
            fly_fail: None,
            behind: false,
            unlisted: 0,
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
            no_fly: self.win.no_fly,
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
            if let Some((fw, fh)) = self.win.fixed {
                frame.right = frame.left + fw;
                frame.bottom = frame.top + fh;
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
        self.run_until(w, events, u64::MAX);
    }

    /// Como `run_many`, pero sin pasar de `until` (µs): lo que vence después
    /// queda pendiente (por ejemplo, la espera de una segunda tecla).
    fn run_until(&mut self, w: &mut World, events: Vec<Event>, until: u64) {
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
                            assert!(self.flight.is_none(), "Place sobre una ventana en vuelo");
                            self.places += 1;
                            self.queued.push((self.now + self.win.lag_ms * MS, rect));
                            queue.push_back(Event::Placed { req, result: Ok(()) });
                        }
                        Effect::Fly { req, win, rect, to, ms, .. } => {
                            assert_eq!(win, self.win.key);
                            assert!(!self.win.maximized, "vuelo de una maximizada");
                            if let Some(f) = self.fly_fail.clone() {
                                queue.push_back(Event::Flown { req, result: Err(f) });
                                continue;
                            }
                            self.flights += 1;
                            // Redirigir: el pedido anterior responde enseguida.
                            if let Some(old) = self.flight.take() {
                                let report = FlyReport { arrived: false, flight_ms: 0 };
                                queue.push_back(Event::Flown { req: old.req, result: Ok(report) });
                            }
                            // La real va en un solo pedido, al despegar.
                            self.queued.push((self.now + self.win.lag_ms * MS, rect));
                            let due = self.now + u64::from(ms) * MS;
                            self.flight = Some(Flight { due, req, to, ms });
                        }
                        Effect::Land { win } => {
                            assert_eq!(win, self.win.key);
                            self.lands += 1;
                            if let Some(old) = self.flight.take() {
                                let report = FlyReport { arrived: false, flight_ms: 0 };
                                queue.push_back(Event::Flown { req: old.req, result: Ok(report) });
                            }
                        }
                        Effect::Normalize { win } => {
                            assert_eq!(win, self.win.key);
                            assert!(self.flight.is_none(), "Normalize sobre una ventana en vuelo");
                            self.normalized = true;
                            if self.win.maximized {
                                self.win.maximized = false;
                                self.win.frame = self.win.restore;
                            }
                        }
                        Effect::Maximize { win } => {
                            assert_eq!(win, self.win.key);
                            assert!(self.flight.is_none(), "Maximize sobre una ventana en vuelo");
                            if !self.win.maximized {
                                let m = monitor_of(&self.monitors, self.win.frame).unwrap();
                                self.win.restore = self.win.frame;
                                self.win.maximized = true;
                                self.win.frame = self.monitors[m].work;
                            }
                        }
                        Effect::Log(line) => self.logs.push(line),
                        Effect::Notice(text) => self.notices.push(text),
                        Effect::MovePointer { .. } => {}
                        // Hito 4d: pantallas.
                        Effect::Survey { req } => {
                            let o = self.obs();
                            let info = crate::world::WinInfo {
                                key: o.key,
                                app: o.app.clone(),
                                title: o.title.clone(),
                                desk: Some(1),
                                ..Default::default()
                            };
                            let survey = crate::world::Survey {
                                monitors: self.monitors.clone(),
                                wins: vec![crate::world::SurveyWin {
                                    info,
                                    frame: o.frame,
                                    outer: o.outer,
                                    maximized: o.maximized,
                                    parked: false,
                                }],
                            };
                            queue.push_back(Event::Surveyed { req, result: Ok(survey) });
                        }
                        Effect::Rescue { req, .. } => {
                            queue.push_back(Event::Rescued { req, failed: Vec::new() })
                        }
                        Effect::ScreensChanged => {}
                        Effect::Scan { req } => {
                            let info = crate::world::WinInfo {
                                key: self.win.key,
                                app: "brave".into(),
                                title: "Brave".into(),
                                desk: Some(1),
                                main: true,
                                ..Default::default()
                            };
                            let fg = (!self.behind).then_some(self.win.key);
                            let list = if self.unlisted > 0 {
                                self.unlisted -= 1;
                                Vec::new()
                            } else {
                                vec![info]
                            };
                            queue.push_back(Event::Windows { req, result: Ok((list, fg)) });
                        }
                        Effect::SetDesks { req, .. } => {
                            queue.push_back(Event::DesksSet { req, failed: Vec::new() })
                        }
                        other => panic!("efecto inesperado: {other:?}"),
                    }
                }
            }
            // Sin nada en cola: avanzar el reloj al próximo plazo (del modelo
            // o del vuelo), si hay.
            let flight_due = self.flight.as_ref().map(|f| f.due);
            let next = [w.deadline(), flight_due].into_iter().flatten().min();
            if next.is_none_or(|t| t > until) {
                self.now = self.now.max(until.min(self.now.max(next.unwrap_or(0))));
                break;
            }
            match (w.deadline(), flight_due) {
                (d, Some(f)) if d.is_none_or(|d| f <= d) => {
                    self.now = self.now.max(f);
                    self.settle();
                    let fl = self.flight.take().expect("recién visto");
                    let report = FlyReport { arrived: self.win.frame == fl.to, flight_ms: fl.ms };
                    queue.push_back(Event::Flown { req: fl.req, result: Ok(report) });
                }
                (Some(t), _) => {
                    self.now = self.now.max(t);
                    queue.push_back(Event::Tick);
                }
                (None, _) => break,
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
fn single_keys_give_halves_and_corners_need_two_keys_together() {
    // Esquema directo: W y después A, sueltas, dan dos mitades; W + A juntas,
    // la esquina. Sin plazos de por medio.
    for gap_ms in [100, 5_000] {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new();
        sim.key(&mut w, b'W');
        assert!(sim.is_at(0, P::TopHalf));
        sim.now += gap_ms * MS;
        sim.key(&mut w, b'A');
        assert!(sim.is_at(0, P::LeftHalf), "a los {gap_ms} ms: {:?}", sim.logs);
        sim.chord(&mut w, b"WA", true);
        assert!(sim.is_at(0, P::TopLeft), "{:?}", sim.logs);
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
fn a_hung_app_gets_one_request_and_no_retries() {
    // La app no atiende mensajes por 2,5 s: el pedido queda en su cola. No se
    // reintenta ni se ancla con el tamaño viejo (eso pisaría el pedido bueno
    // cuando la app despierte).
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.lag_ms = 2_500;
    sim.key(&mut w, b'D');
    assert_eq!(sim.places, 1, "un solo pedido: {:?}", sim.logs);
    assert!(sim.logs.last().unwrap().contains("no responde"), "{:?}", sim.logs);
    assert_eq!(w.deadline(), None);
    sim.now += 3_000 * MS;
    sim.settle(); // la app despierta y aplica lo que tenía en cola
    assert!(sim.is_at(0, P::RightHalf));
}

#[test]
fn minimum_size_is_accepted_after_a_second_pass() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.min_width = 1200; // más ancha que media pantalla
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.places, 2);
    assert_eq!(sim.win.frame.width(), 1200);
    assert!(sim.logs[0].contains("no se achica"), "{:?}", sim.logs);
    assert_eq!(w.deadline(), None);
}

#[test]
fn a_minimum_size_on_the_right_half_sticks_to_the_right_edge() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.min_width = 1200;
    let start = sim.now;
    sim.key(&mut w, b'D');
    assert!(sim.now - start < 300 * MS, "tardó {} ms", (sim.now - start) / MS);
    let work = work();
    assert_eq!(sim.win.frame.right, work.right, "no se sale por la derecha: {:?}", sim.logs);
    assert_eq!(sim.win.frame.width(), 1200);
    assert!(sim.logs.last().unwrap().contains("quedó pegada al borde"), "{:?}", sim.logs);
    assert_eq!(w.deadline(), None);
}

#[test]
fn a_fixed_size_window_goes_into_the_corner_it_was_sent_to() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.fixed = Some((800, 400));
    sim.chord(&mut w, b"SD", true);
    let work = work();
    assert_eq!(
        sim.win.frame,
        Rect::new(work.right - 800, work.bottom - 400, work.right, work.bottom)
    );
    sim.key(&mut w, b'D'); // cuenta como la esquina: D da la mitad derecha (fija igual)
    assert_eq!(sim.win.frame.right, work.right, "{:?}", sim.logs);
}

#[test]
fn a_fixed_window_already_in_its_anchored_spot_is_not_called_hung() {
    // Prueba real de 1d: W la llevó arriba (pegada al borde) y para cuando
    // llegó A, la esquina de arriba a la izquierda (pegada) era donde ya
    // estaba. No se mueve, pero la app no está colgada.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.fixed = Some((800, 400));
    let work = work();
    sim.win.frame = Rect::new(work.left, work.top, work.left + 800, work.top + 400);
    sim.chord(&mut w, b"WA", true);
    let last = sim.logs.last().unwrap();
    assert!(!last.contains("no responde"), "{:?}", sim.logs);
    assert!(last.contains("ya estaba pegada al borde"), "{:?}", sim.logs);
    assert_eq!(sim.win.frame, Rect::new(work.left, work.top, work.left + 800, work.top + 400));
    assert_eq!(w.deadline(), None);
}

#[test]
fn a_window_that_could_not_shrink_still_counts_as_where_we_left_it() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.min_width = 1200;
    sim.key(&mut w, b'A'); // "mitad izquierda", aunque quedó más ancha
    sim.win.min_width = 0;
    sim.key(&mut w, b'D'); // desde la mitad izquierda: mitad derecha, no "libre + D"
    assert!(sim.is_at(0, P::RightHalf), "{:?}", sim.logs);
}

#[test]
fn full_screen_windows_are_left_alone() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = monitor(0).bounds;
    sim.run(&mut w, arrange(b'A', HELD_A));
    assert_eq!(sim.places, 0);
    assert!(sim.logs[0].contains("pantalla completa"));
    assert_eq!(sim.notices.last().map(String::as_str), Some("Brave está en pantalla completa"));
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
    sim.run(&mut w, arrange(b'D', HELD_D));
    assert_eq!(w.placing(), 0);
    assert!(sim.is_at(0, P::RightHalf), "A va a la izquierda; D desde ahí = mitad derecha");
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
fn user_case_1_a_corner_moves_to_the_next_monitor_with_its_chord() {
    // Una esquina se mueve con su acorde; una tecla sola da la mitad.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopRight);
    sim.chord(&mut w, b"WD", true); // ya estaba ahí: empuja
    assert!(sim.is_at(1, P::TopLeft), "{:?}", sim.logs);
    sim.chord(&mut w, b"WD", true); // nombra ↗ del monitor 1
    assert!(sim.is_at(1, P::TopRight), "{:?}", sim.logs);
    sim.chord(&mut w, b"WD", true); // sin monitor a la derecha ni arriba
    assert!(sim.is_at(1, P::TopRight));
    sim.key(&mut w, b'D'); // una tecla sola: la mitad derecha
    assert!(sim.is_at(1, P::RightHalf), "{:?}", sim.logs);
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
fn user_case_3_a_pushed_half_plus_w_is_the_top_half_of_that_monitor() {
    // Primero se definió como "esquina de ese lado"; el usuario prefirió la
    // mitad de arriba de todo el ancho, en una sola tecla.
    for gap_ms in [100, 5_000] {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new();
        sim.at(0, P::RightHalf);
        sim.key(&mut w, b'D'); // mitad izquierda del monitor 1
        sim.now += gap_ms * MS;
        sim.key(&mut w, b'W');
        assert!(sim.is_at(1, P::TopHalf), "a los {gap_ms} ms: {:?}", sim.logs);
        assert!(!sim.logs.iter().any(|l| l.contains("no hay otro monitor")), "{:?}", sim.logs);
    }
}

#[test]
fn a_single_w_against_the_top_says_so_but_not_inside_a_chord() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(1, P::TopHalf);
    sim.key(&mut w, b'W');
    assert!(sim.logs.last().unwrap().contains("no hay otro monitor arriba"), "{:?}", sim.logs);
    let before = sim.logs.len();
    sim.at(1, P::TopLeft);
    sim.chord(&mut w, b"WA", true); // W primero dentro de un acorde
    assert!(sim.is_at(0, P::TopRight), "{:?}", sim.logs);
    assert!(!sim.logs[before..].iter().any(|l| l.contains("arriba.")), "{:?}", sim.logs);
}

#[test]
fn releasing_alt_before_the_second_key_is_resolved_does_not_break_the_chord() {
    // La carrera que encontró la CI: en una máquina lenta, soltar Alt llega
    // entre la respuesta de la primera tecla y la de la segunda.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopLeft);
    let mut inspects = Vec::new();
    for (letter, held) in [(b'D', HELD_D), (b'S', HELD_D | HELD_S)] {
        let effects = w.step(arrange(letter, held), sim.now);
        let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
        inspects.push((req, sim.obs()));
    }
    let (first, second) = (inspects[0].clone(), inspects[1].clone());
    let mut queue = w.step(Event::Inspected { req: first.0, result: Ok(first.1) }, sim.now);
    queue.extend(w.step(Event::Layer(Layer::Apps), sim.now)); // se soltó Alt
    queue.extend(w.step(Event::Inspected { req: second.0, result: Ok(second.1) }, sim.now));
    // Ejecutar lo pedido, como lo haría la plataforma, y dejar terminar.
    let mut events = Vec::new();
    for effect in queue {
        if let Effect::Place { req, rect, .. } = effect {
            sim.queued.push((sim.now, rect));
            events.push(Event::Placed { req, result: Ok(()) });
        }
    }
    sim.run_many(&mut w, events);
    assert!(sim.is_at(0, P::BottomRight), "D + S = esquina de abajo a la derecha: {:?}", sim.logs);
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
                    for (order, fly) in
                        permutations(keys).into_iter().flat_map(|o| [(o.clone(), false), (o, true)])
                    {
                        for together in [true, false] {
                            let mut w = World::new(Vec::new());
                            w.step(Event::FlyEnabled(fly), 0);
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
                            assert!(sim.flight.is_none(), "quedó un vuelo sin terminar");
                            assert_eq!(w.placing(), 0);
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

// ---- F, C y Z (hito 1c) ----

#[test]
fn f_maximizes_and_f_again_goes_back() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let start = sim.win.frame;
    sim.key(&mut w, b'F');
    assert!(sim.win.maximized);
    sim.key(&mut w, b'F');
    assert!(!sim.win.maximized);
    assert_eq!(sim.win.frame, start, "vuelve al tamaño anterior");
}

#[test]
fn c_centers_then_cycles_60_80_and_back_to_the_original_size() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let (ww, hh) = (sim.win.frame.width(), sim.win.frame.height());
    sim.key(&mut w, b'C');
    assert_eq!(sim.win.frame, crate::place::centered(work(), ww, hh), "centrada, mismo tamaño");
    sim.key(&mut w, b'C');
    assert_eq!(sim.win.frame, crate::place::centered_percent(work(), 60));
    sim.key(&mut w, b'C');
    assert_eq!(sim.win.frame, crate::place::centered_percent(work(), 80));
    sim.key(&mut w, b'C');
    assert_eq!(sim.win.frame, crate::place::centered(work(), ww, hh), "vuelve al original");
}

#[test]
fn c_on_an_already_centered_window_starts_at_60() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = crate::place::centered(work(), 900, 700);
    sim.key(&mut w, b'C');
    assert_eq!(sim.win.frame, crate::place::centered_percent(work(), 60));
}

#[test]
fn starting_maximized_c_alternates_60_and_80_with_those_names() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.key(&mut w, b'F');
    for _ in 0..4 {
        sim.key(&mut w, b'C');
    }
    let names: Vec<&str> =
        sim.logs.iter().filter(|l| l.contains("centrada")).map(String::as_str).collect();
    assert_eq!(names.len(), 4, "{:?}", sim.logs);
    assert!(names.iter().all(|l| l.contains("60 %") || l.contains("80 %")), "{names:?}");
}

#[test]
fn every_c_moves_the_window_even_starting_maximized() {
    // Error que vio el usuario: desde maximizada, "original" y "60 %" eran el
    // mismo rectángulo y una de cada tres C no hacía nada.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.key(&mut w, b'F');
    let mut last = sim.win.frame;
    for i in 0..6 {
        sim.key(&mut w, b'C');
        assert_ne!(
            sim.win.frame,
            last,
            "la C número {} no movió la ventana: {:?}",
            i + 1,
            sim.logs
        );
        last = sim.win.frame;
    }
}

#[test]
fn z_undoes_every_kind_of_move_in_order() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let start = sim.win.frame;
    sim.key(&mut w, b'A'); // mitad izquierda
    sim.key(&mut w, b'W'); // mitad de arriba
    sim.chord(&mut w, b"SD", true); // ↘: un acorde, una sola entrada
    sim.key(&mut w, b'D'); // mitad derecha
    sim.key(&mut w, b'D'); // empuja: mitad izquierda del monitor 1
    sim.key(&mut w, b'C'); // centrada
    sim.key(&mut w, b'F'); // maximizada
    sim.key(&mut w, b'Z'); // deshace F
    assert!(!sim.win.maximized);
    sim.key(&mut w, b'Z'); // deshace C
    assert!(sim.is_at(1, P::LeftHalf), "{:?}", sim.logs);
    sim.key(&mut w, b'Z'); // deshace el empuje
    assert!(sim.is_at(0, P::RightHalf), "{:?}", sim.logs);
    sim.key(&mut w, b'Z'); // deshace D
    assert!(sim.is_at(0, P::BottomRight), "{:?}", sim.logs);
    sim.key(&mut w, b'Z'); // deshace el acorde S + D con una sola Z
    assert!(sim.is_at(0, P::TopHalf), "{:?}", sim.logs);
    sim.key(&mut w, b'Z');
    assert!(sim.is_at(0, P::LeftHalf));
    sim.key(&mut w, b'Z');
    assert_eq!(sim.win.frame, start, "volvió al principio");
    sim.key(&mut w, b'Z');
    assert!(sim.logs.last().unwrap().contains("no hay nada para deshacer"));
}

#[test]
fn after_undo_the_next_key_counts_from_the_restored_place() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::RightHalf);
    sim.key(&mut w, b'C');
    sim.key(&mut w, b'Z'); // vuelve a la mitad derecha
    assert!(sim.is_at(0, P::RightHalf));
    sim.key(&mut w, b'D'); // ya está en la mitad derecha: empuja
    assert!(sim.is_at(1, P::LeftHalf), "{:?}", sim.logs);
}

#[test]
fn z_restores_a_maximized_window() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.key(&mut w, b'F');
    sim.key(&mut w, b'A'); // sale de maximizada a la mitad izquierda
    assert!(sim.is_at(0, P::LeftHalf));
    sim.key(&mut w, b'Z');
    assert!(sim.win.maximized, "vuelve a estar maximizada");
}

#[test]
fn z_skips_windows_that_no_longer_exist() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.key(&mut w, b'A');
    sim.key(&mut w, b'D');
    let effects = w.step(arrange(b'Z', 0), sim.now); // pide la ventana de la última entrada
    let Effect::Inspect { req, win } = effects[0] else { panic!("{effects:?}") };
    assert_eq!(win, Some(sim.win.key));
    // La ventana "ya no existe": salta a la anterior en la misma tecla.
    let effects =
        w.step(Event::Inspected { req, result: Err("la ventana ya no existe".into()) }, sim.now);
    assert!(matches!(effects[0], Effect::Inspect { .. }), "pide la entrada anterior: {effects:?}");
}

#[test]
fn opposite_keys_leave_nothing_to_undo() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopLeft);
    sim.chord(&mut w, b"AD", false); // A mueve, D anula: vuelve a ↖
    assert!(sim.is_at(0, P::TopLeft));
    sim.key(&mut w, b'Z');
    assert!(sim.logs.last().unwrap().contains("no hay nada para deshacer"), "{:?}", sim.logs);
}

// ---- Animación (hito 1e) ----

fn flying_world() -> World {
    let mut w = World::new(Vec::new());
    w.step(Event::FlyEnabled(true), 0);
    w
}

#[test]
fn half_to_opposite_half_flies() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(0, P::RightHalf), "{:?}", sim.logs);
    assert_eq!((sim.flights, sim.places), (1, 0), "un vuelo, ningún Place");
    let last = sim.logs.last().unwrap();
    assert!(last.contains("0 px en 1 pasada") && last.contains("(vuelo 160 ms)"), "{last}");
    assert_eq!(w.deadline(), None);
}

#[test]
fn changing_size_flies_too() {
    // Mini-spike 6b: el carril recorta la miniatura (architecture.md §13.8b).
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'W'); // mitad de arriba: otro tamaño
    assert!(sim.is_at(0, P::TopHalf));
    assert_eq!((sim.flights, sim.places), (1, 0));
}

#[test]
fn a_window_that_did_not_take_the_size_changes_size_without_flying() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.win.min_width = 1200; // no se achica a media pantalla
    sim.at(0, P::TopHalf);
    sim.key(&mut w, b'A'); // la primera vez vuela y no llega: se ancla
    assert_eq!(sim.flights, 1);
    assert!(sim.logs.last().unwrap().contains("no se achica"), "{:?}", sim.logs);
    let places = sim.places;
    sim.key(&mut w, b'D'); // de ahí en más, cambiar de tamaño es instantáneo
    assert_eq!(sim.flights, 1, "{:?}", sim.logs);
    assert!(sim.places > places);
    assert_eq!(sim.win.frame.right, work().right, "pegada a la derecha: {:?}", sim.logs);
}

#[test]
fn pushing_flies_only_between_monitors_with_the_same_scale() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::RightHalf);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(1, P::LeftHalf), "{:?}", sim.logs);
    assert_eq!(sim.flights, 1);

    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.monitors[1].dpi = 144;
    sim.at(0, P::RightHalf);
    sim.key(&mut w, b'D');
    assert_eq!(sim.flights, 0, "al cruzar de escala la app se reescala: sin vuelo");
}

#[test]
fn windows_that_cannot_fly_move_instantly() {
    for reason in [NoFly::NotFront, NoFly::Hung, NoFly::Protected, NoFly::ReducedMotion] {
        let mut w = flying_world();
        let mut sim = Sim::new();
        sim.win.no_fly = Some(reason);
        sim.at(0, P::LeftHalf);
        sim.key(&mut w, b'D');
        assert!(sim.is_at(0, P::RightHalf));
        assert_eq!(sim.flights, 0, "{reason:?}");
    }
    // Sin habilitar (el carril no está listo): instantáneo.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'D');
    assert_eq!(sim.flights, 0);
}

#[test]
fn a_short_move_is_not_animated() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    let half = place(work(), P::LeftHalf);
    sim.win.frame = Rect::new(half.left + 30, half.top, half.right + 30, half.bottom);
    sim.key(&mut w, b'A'); // 30 px: se ve igual que un salto
    assert!(sim.is_at(0, P::LeftHalf));
    assert_eq!(sim.flights, 0);
}

#[test]
fn a_key_during_a_flight_counts_from_its_destination_and_redirects() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    let left = place(work(), P::LeftHalf);
    let right = place(work(), P::RightHalf);
    // D: despega (la real ya va a la derecha, con cloak) y el vuelo sigue.
    let effects = w.step(arrange(b'D', HELD_D), sim.now);
    let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
    let effects = w.step(Event::Inspected { req, result: Ok(sim.obs()) }, sim.now);
    let Effect::Fly { req: first, rect, to, .. } = effects[0] else { panic!("{effects:?}") };
    assert_eq!(to, right);
    sim.queued.push((sim.now, rect));
    sim.settle();
    w.step(Event::Layer(Layer::Apps), sim.now);
    w.step(Event::Layer(Layer::Window), sim.now);
    // A a mitad de vuelo (otro gesto): cuenta desde la derecha → izquierda.
    sim.now += 40 * MS;
    let effects = w.step(arrange(b'A', HELD_A), sim.now);
    let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
    let effects = w.step(Event::Inspected { req, result: Ok(sim.obs()) }, sim.now);
    assert!(!effects.iter().any(|e| matches!(e, Effect::Land { .. })), "mismo tamaño: se redirige");
    let Some(Effect::Fly { to, .. }) = effects.first() else { panic!("{effects:?}") };
    assert_eq!(*to, left);
    // La respuesta del primer vuelo (redirigido) no cambia nada.
    let report = FlyReport { arrived: false, flight_ms: 40 };
    assert!(w.step(Event::Flown { req: first, result: Ok(report) }, sim.now).is_empty());
    assert!(w.flying(sim.win.key));
}

#[test]
fn the_model_has_no_deadline_while_flying() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    let effects = w.step(arrange(b'D', HELD_D), sim.now);
    let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
    let effects = w.step(Event::Inspected { req, result: Ok(sim.obs()) }, sim.now);
    let Effect::Fly { req, from, to, ms, .. } = effects[0] else { panic!("{effects:?}") };
    assert_eq!(from, place(work(), P::LeftHalf));
    assert_eq!(to, place(work(), P::RightHalf));
    assert!(w.flying(sim.win.key));
    assert_eq!(w.deadline(), None);
    // Un Flown viejo (de otro pedido) no cambia nada.
    let stray =
        Event::Flown { req: req + 100, result: Ok(FlyReport { arrived: true, flight_ms: ms }) };
    assert!(w.step(stray, sim.now).is_empty());
    assert!(w.flying(sim.win.key));
}

#[test]
fn a_chord_during_a_flight_redirects_it() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.chord(&mut w, b"DS", true); // D vuela; con S, la esquina (otro tamaño)
    assert!(sim.is_at(0, P::BottomRight), "{:?}", sim.logs);
    assert_eq!((sim.flights, sim.lands, sim.places), (2, 0, 0));
}

#[test]
fn f_during_a_flight_lands_it_first() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.run_many(&mut w, vec![arrange(b'D', HELD_D), arrange(b'F', 0)]);
    assert!(sim.win.maximized);
    assert_eq!(sim.lands, 1);
}

#[test]
fn undo_flies_back() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'D');
    sim.key(&mut w, b'Z');
    assert!(sim.is_at(0, P::LeftHalf), "{:?}", sim.logs);
    assert_eq!(sim.flights, 2);
}

#[test]
fn a_failed_flight_places_without_animation() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.fly_fail = Some(FlyFail { fatal: false, why: "cloak no confirmado".into() });
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(0, P::RightHalf));
    assert_eq!(sim.places, 1);
    assert!(!sim.logs.iter().any(|l| l.contains("apagada")), "{:?}", sim.logs);
}

#[test]
fn a_fatal_failure_turns_animation_off_for_the_session() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.fly_fail = Some(FlyFail { fatal: true, why: "el carril se trabó".into() });
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(0, P::RightHalf));
    assert!(sim.logs.iter().any(|l| l.contains("Animación apagada")), "{:?}", sim.logs);
    sim.fly_fail = None;
    sim.key(&mut w, b'A');
    assert!(sim.is_at(0, P::LeftHalf));
    assert_eq!(sim.flights, 0, "ya no se anima");
}

#[test]
fn a_slow_app_that_misses_the_landing_is_checked_as_usual() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.win.lag_ms = 300; // llega después del vuelo
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'D');
    assert!(sim.is_at(0, P::RightHalf), "{:?}", sim.logs);
    assert_eq!(sim.places, 0, "sin segundo pedido");
    assert_eq!(w.placing(), 0);
}

#[test]
fn flight_time_grows_with_distance() {
    use crate::world::fly_ms;
    let r = |x: i32| Rect::new(x, 0, x + 800, 600);
    assert_eq!(fly_ms(r(0), r(64)), 110);
    assert_eq!(fly_ms(r(0), r(960)), 160);
    assert_eq!(fly_ms(r(0), r(1920)), 180);
}

// ---- Acordes de esquina a esquina con las teclas a destiempo (prueba del
// usuario del 1e): la primera tecla llega sola y Windows ya respondió
// cuando llega la segunda. Antes, la primera mandaba la ventana a una mitad
// (otro tamaño) y el acorde ya no se animaba.

/// Las teclas de un acorde con `gap_ms` entre una y otra (menos que la
/// espera de 80 ms), y Windows respondiendo en el medio.
fn chord_spaced(sim: &mut Sim, w: &mut World, letters: &[u8], gap_ms: u64) {
    let mut held = 0;
    for &l in letters {
        held |= bit(l);
        let until = sim.now + gap_ms * MS;
        sim.run_until(w, vec![arrange(l, held)], until);
        sim.now = until;
    }
    sim.run(w, Event::Layer(Layer::Apps));
    sim.run(w, Event::Layer(Layer::Window));
}

#[test]
fn corner_to_corner_flies_even_if_the_keys_arrive_apart() {
    // (origen, teclas, destino) en el monitor 0, como los probó el usuario.
    let cases: [(P, &[u8], P); 6] = [
        (P::TopLeft, b"WD", P::TopRight),
        (P::TopLeft, b"DW", P::TopRight),
        (P::TopRight, b"WA", P::TopLeft),
        (P::BottomRight, b"AS", P::BottomLeft),
        (P::BottomLeft, b"SD", P::BottomRight),
        (P::BottomLeft, b"DS", P::BottomRight),
    ];
    for (from, keys, to) in cases {
        let mut w = flying_world();
        let mut sim = Sim::new();
        sim.at(0, from);
        chord_spaced(&mut sim, &mut w, keys, 20);
        let k = String::from_utf8_lossy(keys);
        assert!(sim.is_at(0, to), "{from:?} + {k}: {:?}", sim.logs);
        assert_eq!(
            (sim.flights, sim.places, sim.lands),
            (1, 0, 0),
            "{from:?} + {k}: {:?}",
            sim.logs
        );
    }
}

#[test]
fn a_corner_pushed_to_the_neighbor_flies_even_if_the_keys_arrive_apart() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(1, P::TopLeft); // monitor de la derecha
    chord_spaced(&mut sim, &mut w, b"AW", 20);
    assert!(sim.is_at(0, P::TopRight), "{:?}", sim.logs);
    assert_eq!((sim.flights, sim.places), (1, 0), "{:?}", sim.logs);
}

#[test]
fn a_single_key_from_a_corner_waits_for_a_second_one_then_goes() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::TopLeft);
    let start = sim.now;
    sim.key(&mut w, b'A'); // sola: la mitad izquierda
    assert!(sim.is_at(0, P::LeftHalf), "{:?}", sim.logs);
    assert_eq!(sim.flights, 1, "una sola tecla: un vuelo, sin paso intermedio");
    assert!(sim.now - start >= 50 * MS, "esperó la segunda tecla");
    // Desde una mitad no se espera: el vuelo sale con la respuesta del Inspect.
    let effects = w.step(arrange(b'W', HELD_W), sim.now);
    let Effect::Inspect { req, .. } = effects[0] else { panic!("{effects:?}") };
    let effects = w.step(Event::Inspected { req, result: Ok(sim.obs()) }, sim.now);
    assert!(matches!(effects.first(), Some(Effect::Fly { .. })), "{effects:?}");
}

#[test]
fn a_corner_chord_with_the_keys_60_ms_apart_still_flies_directly() {
    // Con la espera de 50 ms, ~1 de cada 10 acordes del usuario pasaba por
    // la mitad: ahora se espera 80 ms.
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::BottomLeft);
    chord_spaced(&mut sim, &mut w, b"SD", 60);
    assert!(sim.is_at(0, P::BottomRight), "{:?}", sim.logs);
    assert_eq!((sim.flights, sim.places, sim.lands), (1, 0, 0), "{:?}", sim.logs);
}

#[test]
fn a_second_key_later_than_the_wait_is_logged() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::BottomLeft);
    chord_spaced(&mut sim, &mut w, b"SD", 120);
    assert!(sim.is_at(0, P::BottomRight), "{:?}", sim.logs);
    let last = sim.logs.last().unwrap();
    assert!(last.contains("la segunda tecla llegó 120 ms después"), "{last}");
}

// ---- G/V: agrandar y achicar (hito 1f) ----

#[test]
fn g_widens_a_left_half_against_the_left_edge_and_v_narrows_it() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let work = work();
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'G');
    assert_eq!(sim.win.frame, Rect::new(0, 0, 960 + 192, work.bottom), "{:?}", sim.logs);
    assert!(sim.logs.last().unwrap().contains("más grande, 60 % de ancho"), "{:?}", sim.logs);
    sim.key(&mut w, b'G'); // sigue anclada a la izquierda aunque ya no es una mitad
    assert_eq!(sim.win.frame, Rect::new(0, 0, 960 + 2 * 192, work.bottom));
    sim.key(&mut w, b'V');
    sim.key(&mut w, b'V');
    assert!(sim.is_at(0, P::LeftHalf), "{:?}", sim.logs);
}

#[test]
fn g_on_a_right_half_grows_toward_the_left() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::RightHalf);
    sim.key(&mut w, b'G');
    assert_eq!(sim.win.frame, Rect::new(1920 - 1152, 0, 1920, work().bottom));
}

#[test]
fn g_on_a_top_half_changes_only_the_height() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopHalf);
    sim.key(&mut w, b'G');
    assert_eq!(sim.win.frame, Rect::new(0, 0, 1920, 516 + 103));
    assert!(sim.logs.last().unwrap().contains("% de alto"), "{:?}", sim.logs);
}

#[test]
fn g_on_a_corner_grows_both_ways_out_of_the_corner() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::BottomRight);
    sim.key(&mut w, b'G');
    let work = work();
    assert_eq!(
        sim.win.frame,
        Rect::new(work.right - 1152, work.bottom - (516 + 103), work.right, work.bottom)
    );
}

#[test]
fn g_on_a_free_window_grows_around_its_center_and_stays_on_screen() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = Rect::new(20, 300, 820, 700); // cerca del borde izquierdo
    sim.key(&mut w, b'G');
    let f = sim.win.frame;
    assert_eq!((f.width(), f.height()), (800 + 192, 400 + 103));
    assert_eq!(f.left, 0, "corrida para que entre: {f:?}");
    assert!((f.top + f.bottom - (300 + 700)).abs() <= 1, "centrada en alto: {f:?}");
}

#[test]
fn g_and_v_stop_at_the_limits() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    for _ in 0..5 {
        sim.key(&mut w, b'G');
    }
    assert_eq!(sim.win.frame.width(), 1920, "hasta el ancho completo");
    let places = sim.places;
    sim.key(&mut w, b'G');
    assert_eq!(sim.places, places);
    assert!(sim.logs.last().unwrap().contains("tamaño máximo"), "{:?}", sim.logs);
    for _ in 0..10 {
        sim.key(&mut w, b'V');
    }
    assert_eq!(sim.win.frame.width(), 1920 * 20 / 100, "no menos del 20 %");
    assert_eq!(sim.win.frame.left, 0, "sigue pegada a la izquierda");
    assert!(sim.logs.last().unwrap().contains("tamaño mínimo"), "{:?}", sim.logs);
}

#[test]
fn v_on_a_maximized_window_gives_90_percent_centered() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.key(&mut w, b'F');
    assert!(sim.win.maximized);
    sim.key(&mut w, b'V');
    assert!(!sim.win.maximized);
    let f = sim.win.frame;
    assert_eq!((f.width(), f.height()), (1920 - 192, 1032 - 103), "{:?}", sim.logs);
    assert_eq!(f.left, 96);
}

#[test]
fn after_g_a_goes_back_to_the_exact_half_and_z_undoes_g() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'G');
    sim.key(&mut w, b'A'); // no empuja al otro monitor: vuelve a la mitad
    assert!(sim.is_at(0, P::LeftHalf), "{:?}", sim.logs);
    sim.key(&mut w, b'G');
    sim.key(&mut w, b'Z');
    assert!(sim.is_at(0, P::LeftHalf), "{:?}", sim.logs);
}

#[test]
fn g_flies_like_any_other_size_change() {
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.at(0, P::LeftHalf);
    sim.key(&mut w, b'G');
    assert_eq!((sim.flights, sim.places), (1, 0));
}

#[test]
fn hitting_the_minimum_with_v_does_not_stop_other_animations() {
    // Prueba del usuario: Brave no baja de 502 px; después de V hasta el
    // límite, centrar, G, A y W + A dejaban de animarse.
    let mut w = flying_world();
    let mut sim = Sim::new();
    sim.win.min_width = 502;
    sim.at(0, P::RightHalf);
    for _ in 0..4 {
        sim.key(&mut w, b'V'); // 40 %, 30 %, 20 % (no llega: 502 px), otra vez
    }
    assert_eq!(sim.win.frame.width(), 502, "{:?}", sim.logs);
    let flights = sim.flights;
    sim.key(&mut w, b'G'); // 502 → más ancha: dentro de lo que acepta
    sim.key(&mut w, b'A');
    sim.chord(&mut w, b"WA", true);
    sim.key(&mut w, b'C');
    assert!(sim.flights >= flights + 4, "se siguen animando: {:?}", sim.logs);
    // La segunda vez, V ya no pide menos de 502 px: llega volando.
    let flights = sim.flights;
    sim.at(0, P::RightHalf);
    for _ in 0..3 {
        sim.key(&mut w, b'V');
    }
    assert_eq!(sim.win.frame.width(), 502, "{:?}", sim.logs);
    assert_eq!(sim.flights, flights + 3, "{:?}", sim.logs);
    assert_eq!(sim.win.frame.height(), work().height(), "solo el ancho: {:?}", sim.logs);
}

#[test]
fn v_at_the_app_minimum_does_not_ask_again() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.min_width = 502;
    sim.at(0, P::RightHalf);
    for _ in 0..3 {
        sim.key(&mut w, b'V'); // 40 %, 30 %, 20 % (queda en 502 px)
    }
    let places = sim.places;
    sim.key(&mut w, b'V');
    assert_eq!(sim.places, places, "no vuelve a pedir: {:?}", sim.logs);
    assert!(sim.logs.last().unwrap().contains("la app no se achica más"), "{:?}", sim.logs);
    sim.key(&mut w, b'G'); // agrandar sí
    assert!(sim.win.frame.width() > 502);
}

#[test]
fn v_on_a_right_half_after_growing_a_corner_changes_only_the_width() {
    // Prueba del usuario: esquina → G hasta ~90 % → D (mitad derecha) → V
    // achicaba los dos ejes ("40 % × 90 %") en vez de solo el ancho.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::TopRight);
    for _ in 0..4 {
        sim.key(&mut w, b'G');
    }
    sim.key(&mut w, b'D');
    assert!(sim.is_at(0, P::RightHalf), "{:?}", sim.logs);
    sim.key(&mut w, b'V');
    assert!(sim.logs.last().unwrap().contains("40 % de ancho"), "{:?}", sim.logs);
    assert_eq!(sim.win.frame.height(), work().height());
    assert_eq!(sim.win.frame.right, work().right);
}

#[test]
fn v_on_a_half_the_app_cannot_fill_changes_only_the_width() {
    // Prueba del usuario con Steam (no baja de 1010 × 600): esquina de arriba
    // a la derecha → G hasta ~93 % → D (mitad derecha, queda de 1010 px) → V
    // achicaba los dos ejes ("53 % × 90 %").
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.min_width = 1010;
    sim.key(&mut w, b'D'); // aprende el mínimo de ancho
    sim.chord(&mut w, b"WD", true);
    for _ in 0..4 {
        sim.key(&mut w, b'G');
    }
    sim.key(&mut w, b'D');
    assert_eq!(sim.win.frame.width(), 1010, "{:?}", sim.logs);
    assert_eq!(sim.win.frame.height(), work().height());
    let places = sim.places;
    sim.key(&mut w, b'V');
    assert_eq!(sim.win.frame.height(), work().height(), "solo el ancho: {:?}", sim.logs);
    assert_eq!(sim.places, places, "ya está en su mínimo de ancho: {:?}", sim.logs);
}

#[test]
fn v_on_a_strip_restored_by_z_changes_only_the_width() {
    // Prueba del usuario con Terminal: franja del 40 % pegada a la derecha →
    // C → Z (vuelve a la franja) → V achicaba los dos ejes ("30 % × 90 %").
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(0, P::RightHalf);
    sim.key(&mut w, b'V'); // 40 % de ancho
    sim.key(&mut w, b'C');
    sim.key(&mut w, b'Z');
    assert_eq!(sim.win.frame.width(), 1920 * 40 / 100, "{:?}", sim.logs);
    sim.key(&mut w, b'V');
    let last = sim.logs.last().unwrap();
    assert!(last.contains("30 % de ancho"), "{last}");
    assert_eq!(sim.win.frame.right, work().right);
    assert_eq!(sim.win.frame.height(), work().height());
}

#[test]
fn g_and_v_follow_the_edges_the_window_touches() {
    let work = work();
    // Franja de arriba de 30 % de alto, a todo el ancho: solo el alto.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = Rect::new(work.left, work.top, work.right, work.top + 310);
    sim.key(&mut w, b'G');
    assert!(sim.logs.last().unwrap().contains("% de alto"), "{:?}", sim.logs);
    assert_eq!(sim.win.frame.top, work.top);
    // Pegada abajo a la izquierda sin ser una esquina exacta: contra esa esquina.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.win.frame = Rect::new(work.left, work.bottom - 400, work.left + 700, work.bottom);
    sim.key(&mut w, b'V');
    assert_eq!(
        (sim.win.frame.left, sim.win.frame.bottom),
        (work.left, work.bottom),
        "{:?}",
        sim.logs
    );
}

#[test]
fn z_to_a_monitor_that_is_gone_lands_on_the_nearest_one() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(1, P::LeftHalf);
    sim.key(&mut w, b'C'); // centrada en el monitor 1: la entrada guarda la mitad
    // Se desconecta el monitor 1 y Windows trae la ventana al 0.
    sim.monitors = vec![monitor(0)];
    sim.win.frame = Rect::new(400, 200, 1360, 800);
    sim.key(&mut w, b'Z');
    let f = sim.win.frame;
    assert_eq!((f.width(), f.height()), (960, 1032), "mismo tamaño: {f:?} {:?}", sim.logs);
    assert_eq!(f.right, 1920, "pegada al borde más cercano");
    assert!(
        sim.logs.iter().any(|l| l.contains("fuera de las pantallas actuales")),
        "{:?}",
        sim.logs
    );
}

// ---- Hito 4d (ajuste): reencaje tras un cambio de escala ----

/// Windows reescala la ventana al cambiar la escala de su monitor: el
/// rectángulo exterior crece ×k desde la esquina y los bordes invisibles
/// también.
fn rescale(sim: &mut Sim, dpi: u32, work_bottom: i32) {
    let k = f64::from(dpi) / f64::from(sim.win.dpi);
    sim.monitors[0].dpi = dpi;
    sim.monitors[0].work.bottom = work_bottom;
    let o = sim.win.outer();
    let (w, h) = ((f64::from(o.width()) * k) as i32, (f64::from(o.height()) * k) as i32);
    let outer = Rect::new(o.left, o.top, o.left + w, o.top + h);
    sim.win.dpi = dpi;
    let (l, t, r, b) = sim.win.margins_now();
    sim.win.frame = Rect::new(outer.left + l, outer.top + t, outer.right - r, outer.bottom - b);
}

#[test]
fn a_scale_change_snaps_a_half_back_exactly_without_z_or_flight() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.monitors = vec![monitor(0)];
    sim.key(&mut w, b'A');
    assert!(sim.is_at(0, P::LeftHalf));
    let flights = sim.flights;
    rescale(&mut sim, 120, 1020);
    assert!(!sim.is_at(0, P::LeftHalf), "Windows la despegó");
    sim.run(&mut w, Event::Screen(crate::screens::ScreenEvent::Changed));
    assert!(sim.is_at(0, P::LeftHalf), "reencajada: {:?} {:?}", sim.win.frame, sim.logs);
    assert_eq!(sim.flights, flights, "sin vuelo");
    assert!(sim.logs.iter().any(|l| l.contains("reencajadas")), "{:?}", sim.logs);
    // Z deshace la A, no el reencaje: vuelve a donde estaba antes de la A.
    let places = sim.places;
    sim.key(&mut w, b'A');
    assert_eq!(sim.places, places, "ya está exacta: A no la mueve");
    assert!(sim.logs.last().unwrap().contains("no hay otro monitor"), "{:?}", sim.logs);
}

#[test]
fn a_window_moved_by_hand_is_not_snapped_after_a_scale_change() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.monitors = vec![monitor(0)];
    sim.key(&mut w, b'A');
    sim.win.frame = Rect::new(400, 300, 1000, 900);
    let places = sim.places;
    rescale(&mut sim, 120, 1020);
    sim.run(&mut w, Event::Screen(crate::screens::ScreenEvent::Changed));
    assert_eq!(sim.places, places, "{:?}", sim.logs);
}

#[test]
fn one_pixel_off_without_a_neighbor_a_makes_it_exact_once() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.monitors = vec![monitor(0)];
    sim.at(0, P::LeftHalf);
    sim.win.frame.right -= 1;
    sim.key(&mut w, b'A');
    assert!(sim.is_at(0, P::LeftHalf), "{:?}", sim.logs);
    assert!(sim.logs.iter().any(|l| l.contains("no hay otro monitor")));
    let places = sim.places;
    sim.key(&mut w, b'A');
    assert_eq!(sim.places, places, "la segunda A no pide nada");
    sim.key(&mut w, b'Z');
    assert!(sim.is_at(0, P::LeftHalf), "el ajuste no deja entrada en Z");
}

#[test]
fn one_pixel_off_with_a_neighbor_a_still_pushes() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(1, P::LeftHalf);
    sim.win.frame.left += 1;
    sim.key(&mut w, b'A');
    assert!(sim.is_at(0, P::RightHalf), "{:?}", sim.logs);
}

#[test]
fn a_corner_one_pixel_off_without_a_neighbor_ends_exact() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.monitors = vec![monitor(0)];
    sim.at(0, P::TopLeft);
    sim.win.frame.bottom += 1;
    sim.chord(&mut w, b"WA", true);
    assert!(sim.is_at(0, P::TopLeft), "{:?} {:?}", sim.win.frame, sim.logs);
}

#[test]
fn a_window_already_in_a_corner_before_wm_counts_as_placed() {
    // Como Rayo: puesta en su esquina en otra sesión de wm.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.monitors = vec![monitor(0)];
    sim.at(0, P::TopRight);
    sim.run(&mut w, Event::Screen(crate::screens::ScreenEvent::WorkArea)); // como el arranque
    rescale(&mut sim, 120, 1020);
    sim.run(&mut w, Event::Screen(crate::screens::ScreenEvent::Changed));
    assert!(sim.is_at(0, P::TopRight), "{:?} {:?}", sim.win.frame, sim.logs);
}

#[test]
fn a_half_on_a_monitor_that_goes_away_snaps_back_when_it_returns() {
    // Como Claude: en el monitor 2; al desconectarlo Windows la lleva al 1 y
    // al reconectarlo la devuelve corrida unos px.
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.at(1, P::RightHalf);
    sim.key(&mut w, b'A');
    assert!(sim.is_at(1, P::LeftHalf));
    let both = sim.monitors.clone();
    sim.monitors = vec![monitor(0)];
    sim.win.frame = Rect::new(0, 0, 960, 1032);
    sim.run(&mut w, Event::Screen(crate::screens::ScreenEvent::Changed));
    sim.monitors = both;
    sim.win.frame = Rect::new(1923, 2, 2885, 1030);
    sim.run(&mut w, Event::Screen(crate::screens::ScreenEvent::Changed));
    assert!(sim.is_at(1, P::LeftHalf), "{:?} {:?}", sim.win.frame, sim.logs);
}

// ---- v1.1: regla "al abrir" ----

#[test]
fn a_new_window_with_an_open_rule_goes_to_its_place_once_and_z_undoes_it() {
    let (b, _) = crate::apps::parse_bindings("b = brave | | | al abrir: derecha");
    let mut w = World::new(b);
    let mut sim = Sim::new();
    let start = sim.win.frame;
    sim.run(&mut w, Event::Foreground(sim.win.key)); // apareció una ventana nueva
    assert!(sim.is_at(0, P::RightHalf), "{:?} {:?}", sim.win.frame, sim.logs);
    assert_eq!(sim.flights, 0, "sin animación");
    let places = sim.places;
    sim.run(&mut w, Event::Foreground(sim.win.key));
    assert_eq!(sim.places, places, "una sola vez");
    sim.key(&mut w, b'Z');
    assert_eq!(sim.win.frame, start, "Z la devuelve adonde la abrió la app");
}

#[test]
fn a_window_that_appears_behind_gets_its_rule_when_it_comes_to_the_front() {
    // Como IntelliJ: la ventana principal aparece mientras la pantalla de
    // carga sigue al frente, y pasa al frente un momento después.
    let (b, _) = crate::apps::parse_bindings("i = idea64, brave | | | al abrir: derecha");
    let mut w = World::new(b);
    let mut sim = Sim::new();
    sim.behind = true;
    // Dispara el `Scan` (la 99 no aparece: se la sigue buscando un rato).
    let first = Event::Foreground(WinKey { hwnd: 99, pid: 99 });
    sim.run_until(&mut w, vec![first], sim.now + 100 * MS);
    assert_eq!(sim.places, 0, "detrás no se toca todavía: {:?}", sim.logs);
    sim.behind = false;
    sim.now += 800 * MS;
    sim.run(&mut w, Event::Foreground(sim.win.key));
    assert!(sim.is_at(0, P::RightHalf), "{:?} {:?}", sim.win.frame, sim.logs);
}

#[test]
fn a_window_that_comes_to_the_front_before_it_is_listed_still_gets_its_rule() {
    // Como IntelliJ: Java la activa antes de terminar de mostrarla; el
    // primer `Scan` no la ve y Windows no vuelve a avisar.
    let (b, _) = crate::apps::parse_bindings("i = idea64, brave | | | al abrir: derecha");
    let mut w = World::new(b);
    let mut sim = Sim::new();
    sim.unlisted = 2;
    sim.run(&mut w, Event::Foreground(sim.win.key));
    assert!(sim.is_at(0, P::RightHalf), "{:?} {:?}", sim.win.frame, sim.logs);
    assert!(sim.logs.iter().any(|l| l.contains("después de pasar al frente")), "{:?}", sim.logs);
}

#[test]
fn without_rules_an_unlisted_window_is_not_searched_again() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.unlisted = 1;
    sim.run(&mut w, Event::Foreground(sim.win.key));
    assert_eq!(sim.unlisted, 0);
    sim.run(&mut w, Event::Tick);
    assert_eq!(w.deadline(), None, "ningún plazo pendiente");
}

#[test]
fn a_window_that_stays_behind_is_not_touched() {
    let (b, _) = crate::apps::parse_bindings("i = idea64, brave | | | al abrir: derecha");
    let mut w = World::new(b);
    let mut sim = Sim::new();
    let start = sim.win.frame;
    sim.behind = true;
    sim.run(&mut w, Event::Foreground(WinKey { hwnd: 99, pid: 99 }));
    sim.behind = false;
    sim.now += 10_000 * MS; // pasa al frente mucho después: ya es del usuario
    sim.run(&mut w, Event::Foreground(sim.win.key));
    assert_eq!(sim.win.frame, start, "{:?}", sim.logs);
}

#[test]
fn without_rules_nothing_moves() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let start = sim.win.frame;
    sim.run(&mut w, Event::Foreground(sim.win.key));
    assert_eq!(sim.win.frame, start);
    assert_eq!(sim.places, 0);
}
