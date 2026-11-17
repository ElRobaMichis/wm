//! Hito 4d: tormentas de pantallas, bloqueo y rescate, contra una plataforma
//! simulada (plan-hito4d §6).

use std::collections::VecDeque;

use crate::Rect;
use crate::keyboard::HyperAction;
use crate::place::MonitorObs;
use crate::screens::{ScreenEvent, min_visible, reachable};
use crate::world::{Effect, Event, FocusReport, Survey, SurveyWin, WinInfo, WinKey, World};

const MS: u64 = 1000;

#[derive(Debug, Clone)]
struct Win {
    info: WinInfo,
    frame: Rect,
    maximized: bool,
    parked: bool,
}

struct Sim {
    now: u64,
    monitors: Vec<MonitorObs>,
    wins: Vec<Win>,
    logs: Vec<String>,
    surveys: usize,
    rescues: usize,
    screens_changed: usize,
    /// "Windows" corre esta ventana justo después de un rescate.
    wander: Option<WinKey>,
}

fn monitor(left: i32) -> MonitorObs {
    MonitorObs {
        bounds: Rect::new(left, 0, left + 1920, 1080),
        work: Rect::new(left, 0, left + 1920, 1032),
        dpi: 96,
    }
}

fn key(n: isize) -> WinKey {
    WinKey { hwnd: n, pid: n as u32 * 10 }
}

impl Sim {
    fn new() -> Self {
        Self {
            now: 1_000 * MS,
            monitors: vec![monitor(0), monitor(1920)],
            wins: Vec::new(),
            logs: Vec::new(),
            surveys: 0,
            rescues: 0,
            screens_changed: 0,
            wander: None,
        }
    }

    fn add(&mut self, n: isize, frame: Rect) -> WinKey {
        let info = WinInfo {
            key: key(n),
            app: format!("app{n}"),
            title: format!("ventana {n}"),
            desk: Some(1),
            ..WinInfo::default()
        };
        self.wins.push(Win { info, frame, maximized: false, parked: false });
        key(n)
    }

    fn get(&mut self, k: WinKey) -> &mut Win {
        self.wins.iter_mut().find(|w| w.info.key == k).expect("ventana")
    }

    fn frame(&self, k: WinKey) -> Rect {
        self.wins.iter().find(|w| w.info.key == k).expect("ventana").frame
    }

    fn survey(&self) -> Survey {
        Survey {
            monitors: self.monitors.clone(),
            wins: self
                .wins
                .iter()
                .map(|w| SurveyWin {
                    info: w.info.clone(),
                    frame: w.frame,
                    // Bordes invisibles de 7 px a los lados y abajo.
                    outer: Rect::new(
                        w.frame.left - 7,
                        w.frame.top,
                        w.frame.right + 7,
                        w.frame.bottom + 7,
                    ),
                    maximized: w.maximized,
                    parked: w.parked,
                })
                .collect(),
        }
    }

    fn run(&mut self, w: &mut World, event: Event) {
        let mut queue = VecDeque::from([event]);
        while let Some(e) = queue.pop_front() {
            for effect in w.step(e, self.now) {
                match effect {
                    Effect::Survey { req } => {
                        self.surveys += 1;
                        queue.push_back(Event::Surveyed { req, result: Ok(self.survey()) });
                    }
                    Effect::Rescue { req, moves } => {
                        self.rescues += 1;
                        for (k, x, y) in moves {
                            let win = self.get(k);
                            let (dx, dy) = (x + 7 - win.frame.left, y - win.frame.top);
                            let f = win.frame;
                            win.frame =
                                Rect::new(f.left + dx, f.top + dy, f.right + dx, f.bottom + dy);
                        }
                        if let Some(k) = self.wander {
                            self.get(k).frame = Rect::new(-6000, 100, -5200, 700);
                        }
                        queue.push_back(Event::Rescued { req, failed: Vec::new() });
                    }
                    Effect::ScreensChanged => self.screens_changed += 1,
                    Effect::Scan { req } => {
                        let list = self.wins.iter().map(|w| w.info.clone()).collect();
                        queue.push_back(Event::Windows { req, result: Ok((list, None)) });
                    }
                    Effect::SetDesks { req, .. } => {
                        queue.push_back(Event::DesksSet { req, failed: Vec::new() })
                    }
                    Effect::Show { req, .. } => {
                        queue.push_back(Event::Shown { req, result: Ok(Vec::new()) })
                    }
                    Effect::Hide { req, .. } => {
                        queue.push_back(Event::Hidden { req, result: Ok(Vec::new()) })
                    }
                    Effect::Focus { req, .. } | Effect::FocusShell { req } => {
                        let report = FocusReport { ok: true, neutral_key: false, elapsed_us: 500 };
                        queue.push_back(Event::Focused { req, result: Ok(report) });
                    }
                    Effect::Log(l) => self.logs.push(l),
                    _ => {}
                }
            }
        }
    }

    /// Avanza el reloj de plazo en plazo, hasta `ms` más.
    fn wait(&mut self, w: &mut World, ms: u64) {
        let end = self.now + ms * MS;
        while let Some(d) = w.deadline().filter(|&d| d <= end) {
            self.now = self.now.max(d);
            self.run(w, Event::Tick);
        }
        self.now = end;
    }

    fn screen(&mut self, w: &mut World, e: ScreenEvent) {
        self.run(w, Event::Screen(e));
    }

    fn calm_logs(&self) -> usize {
        self.logs.iter().filter(|l| l.starts_with("Pantallas: en calma")).count()
    }

    fn reachable(&self, k: WinKey) -> bool {
        let areas: Vec<Rect> = self.monitors.iter().map(|m| m.work).collect();
        reachable(self.frame(k), &areas, min_visible(&self.monitors))
    }
}

#[test]
fn a_storm_gives_one_reconcile_and_then_the_brain_sleeps() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.add(1, Rect::new(100, 100, 900, 700));
    for _ in 0..5 {
        sim.screen(&mut w, ScreenEvent::Changed);
        sim.wait(&mut w, 100);
    }
    assert_eq!(sim.surveys, 0, "nada mientras siguen llegando avisos");
    sim.wait(&mut w, 2_000);
    assert_eq!(sim.calm_logs(), 1, "{:?}", sim.logs);
    assert_eq!(sim.surveys, 2, "dos instantáneas iguales");
    assert!(sim.logs.iter().any(|l| l.contains("pantallas ×5")), "{:?}", sim.logs);
    sim.wait(&mut w, 5_000);
    assert_eq!(sim.surveys, 3, "más el control tardío");
    assert_eq!(w.deadline(), None, "nada pendiente: el brain duerme");
    assert_eq!(sim.rescues, 0, "todo estaba bien: no se mueve nada");
}

#[test]
fn a_window_left_outside_comes_back_with_its_size_and_one_notice() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let ok = sim.add(1, Rect::new(100, 100, 900, 700));
    let lost = sim.add(2, Rect::new(2500, 100, 3300, 700));
    // Se desconecta el monitor de la derecha y Windows no trajo la ventana.
    sim.monitors = vec![monitor(0)];
    sim.screen(&mut w, ScreenEvent::Changed);
    sim.wait(&mut w, 2_000);
    assert_eq!(sim.rescues, 1);
    assert!(sim.reachable(lost), "{:?}", sim.frame(lost));
    assert_eq!((sim.frame(lost).width(), sim.frame(lost).height()), (800, 600));
    assert_eq!(sim.frame(ok), Rect::new(100, 100, 900, 700), "la que estaba bien no se toca");
    let line = sim.logs.iter().find(|l| l.starts_with("Pantallas: en calma")).expect("log");
    assert!(line.contains("1 fuera de pantalla") && line.contains("app2"), "{line}");
    assert!(crate::notices::notice_for(line).is_some(), "aviso en pantalla: {line}");
}

#[test]
fn windows_that_belong_to_windows_are_left_alone() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let parked = sim.add(1, Rect::new(-32000, -32000, -31200, -31400));
    sim.get(parked).parked = true;
    let minimized = sim.add(2, Rect::new(5000, 100, 5800, 700));
    sim.get(minimized).info.minimized = true;
    let full = sim.add(3, monitor(0).bounds);
    sim.screen(&mut w, ScreenEvent::Changed);
    sim.wait(&mut w, 2_000);
    assert_eq!(sim.rescues, 0, "{:?}", sim.logs);
    assert_eq!(sim.frame(full), monitor(0).bounds);
}

#[test]
fn a_hidden_window_on_another_desk_is_rescued_too() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let hidden = sim.add(1, Rect::new(4000, 100, 4800, 700));
    sim.get(hidden).info.desk = Some(2);
    sim.get(hidden).info.marked = true;
    sim.get(hidden).info.concealed = true;
    sim.screen(&mut w, ScreenEvent::Changed);
    sim.wait(&mut w, 2_000);
    assert!(sim.reachable(hidden), "con cloak también se trae: {:?}", sim.frame(hidden));
}

#[test]
fn nothing_happens_while_locked_and_one_pass_after_unlocking() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let lost = sim.add(1, Rect::new(4000, 100, 4800, 700));
    sim.screen(&mut w, ScreenEvent::Lock);
    sim.screen(&mut w, ScreenEvent::Changed);
    assert_eq!(w.deadline(), None, "bloqueada: sin timers");
    sim.wait(&mut w, 5_000);
    assert_eq!(sim.surveys, 0);
    assert!(!sim.reachable(lost));
    sim.screen(&mut w, ScreenEvent::Unlock);
    sim.wait(&mut w, 2_000);
    assert!(sim.reachable(lost));
    assert_eq!(sim.calm_logs(), 1);
}

#[test]
fn a_key_unblocks_a_lost_unlock() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let lost = sim.add(1, Rect::new(4000, 100, 4800, 700));
    sim.screen(&mut w, ScreenEvent::Suspend);
    sim.screen(&mut w, ScreenEvent::Changed);
    sim.wait(&mut w, 2_000);
    assert!(!sim.reachable(lost));
    // Nunca llegó la vuelta del reposo; el usuario aprieta una tecla.
    sim.run(&mut w, Event::Action { action: HyperAction::Escape, held: 0, age_ms: 0 });
    sim.wait(&mut w, 2_000);
    assert!(sim.reachable(lost), "{:?}", sim.logs);
}

#[test]
fn a_late_wave_is_rescued_and_a_window_that_runs_away_again_is_left() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let runner = sim.add(1, Rect::new(4000, 100, 4800, 700));
    sim.wander = Some(runner);
    sim.screen(&mut w, ScreenEvent::Changed);
    sim.wait(&mut w, 2_000);
    assert_eq!(sim.rescues, 1);
    assert!(!sim.reachable(runner), "se volvió a ir");
    sim.wander = None;
    // Ola tardía: otra ventana queda afuera después de la calma.
    let late = sim.add(2, Rect::new(-5000, 200, -4200, 800));
    sim.wait(&mut w, 5_000);
    assert!(sim.reachable(late), "el control tardío la trae");
    assert!(!sim.reachable(runner), "no se insiste");
    assert!(sim.logs.iter().any(|l| l.contains("app1 sigue fuera de pantalla")), "{:?}", sim.logs);
}

#[test]
fn the_ui_hears_about_new_monitors_but_not_on_the_first_pass() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    sim.add(1, Rect::new(100, 100, 900, 700));
    sim.screen(&mut w, ScreenEvent::WorkArea);
    sim.wait(&mut w, 5_000);
    assert_eq!(sim.screens_changed, 0, "primera lectura: nada que rearmar");
    sim.monitors = vec![monitor(0)];
    sim.screen(&mut w, ScreenEvent::Changed);
    sim.wait(&mut w, 5_000);
    assert_eq!(sim.screens_changed, 1);
    // Mismo cambio de nuevo (sin cambio real): no rearma.
    sim.screen(&mut w, ScreenEvent::Changed);
    sim.wait(&mut w, 5_000);
    assert_eq!(sim.screens_changed, 1);
}

#[test]
fn a_maximized_window_left_on_a_gone_monitor_is_maximized_again_on_one_that_is_there() {
    let mut w = World::new(Vec::new());
    let mut sim = Sim::new();
    let max = sim.add(1, Rect::new(3840, 0, 5760, 1032));
    sim.get(max).maximized = true;
    sim.screen(&mut w, ScreenEvent::Changed);
    let mut effects = Vec::new();
    // A mano para ver los efectos.
    sim.wait(&mut w, 600);
    for _ in 0..4 {
        if let Some(d) = w.deadline() {
            sim.now = sim.now.max(d);
            effects.extend(w.step(Event::Tick, sim.now));
            let survey = effects.iter().find_map(|e| match e {
                Effect::Survey { req } => Some(*req),
                _ => None,
            });
            if let Some(req) = survey {
                effects.extend(w.step(Event::Surveyed { req, result: Ok(sim.survey()) }, sim.now));
            }
        }
    }
    let i = effects
        .iter()
        .position(|e| matches!(e, Effect::Normalize { .. }))
        .unwrap_or_else(|| panic!("{effects:?}"));
    assert!(matches!(effects[i + 1], Effect::Place { .. }));
    assert!(matches!(effects[i + 2], Effect::Maximize { .. }));
}
