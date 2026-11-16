//! Escritorios 1-4 con una plataforma simulada (hito 4, `plan-hito4.md`).
//!
//! La plataforma imita lo medido en `wm desk-check`: esconder marca y pone
//! cloak; mostrar quita las dos cosas; enfocar una escondida la destapa (la
//! marca queda); una ventana "no marcable" (elevada) falla al anotar y al
//! esconder.

use crate::keyboard::HyperAction;
use crate::world::{Effect, Event, FocusReport, WinInfo, WinKey, World};

#[derive(Debug, Clone)]
struct W {
    key: WinKey,
    app: String,
    desk: Option<u8>,
    marked: bool,
    cloaked: bool,
    minimized: bool,
    unmarkable: bool,
}

struct Plat {
    /// Orden Z: la primera, arriba.
    wins: Vec<W>,
    fg: Option<WinKey>,
    logs: Vec<String>,
    now: u64,
    next_hwnd: isize,
}

impl Plat {
    fn new(apps: &[&str]) -> Self {
        let mut p =
            Self { wins: Vec::new(), fg: None, logs: Vec::new(), now: 1_000_000, next_hwnd: 1 };
        for a in apps {
            p.open(a);
        }
        p
    }

    /// Abre una ventana nueva (arriba y con el foco, como Windows).
    fn open(&mut self, app: &str) -> WinKey {
        let key = WinKey { hwnd: self.next_hwnd, pid: self.next_hwnd as u32 + 100 };
        self.next_hwnd += 1;
        self.wins.insert(
            0,
            W {
                key,
                app: app.into(),
                desk: None,
                marked: false,
                cloaked: false,
                minimized: false,
                unmarkable: false,
            },
        );
        self.fg = Some(key);
        key
    }

    fn get(&mut self, k: WinKey) -> Option<&mut W> {
        self.wins.iter_mut().find(|w| w.key == k)
    }

    fn scan(&self) -> Vec<WinInfo> {
        self.wins
            .iter()
            .map(|w| WinInfo {
                key: w.key,
                app: w.app.clone(),
                title: w.app.clone(),
                desk: w.desk,
                marked: w.marked,
                concealed: w.cloaked || w.minimized,
                minimized: w.minimized,
            })
            .collect()
    }

    fn run(&mut self, world: &mut World, event: Event) {
        let mut queue = std::collections::VecDeque::from([event]);
        while let Some(e) = queue.pop_front() {
            self.now += 1000;
            for effect in world.step(e, self.now) {
                match effect {
                    Effect::Scan { req } => {
                        queue.push_back(Event::Windows { req, result: Ok((self.scan(), self.fg)) })
                    }
                    Effect::SetDesks { req, items } => {
                        let mut failed = Vec::new();
                        for (k, n) in items {
                            match self.get(k) {
                                Some(w) if !w.unmarkable => w.desk = Some(n),
                                _ => failed.push(k),
                            }
                        }
                        queue.push_back(Event::DesksSet { req, failed });
                    }
                    Effect::Show { req, wins } => {
                        let mut failed = Vec::new();
                        for k in wins {
                            match self.get(k) {
                                Some(w) => {
                                    w.marked = false;
                                    w.cloaked = false;
                                }
                                None => failed.push(k),
                            }
                        }
                        queue.push_back(Event::Shown { req, result: Ok(failed) });
                    }
                    Effect::Hide { req, wins } => {
                        let mut failed = Vec::new();
                        for k in wins {
                            match self.get(k) {
                                Some(w) if !w.unmarkable => {
                                    w.marked = true;
                                    w.cloaked = true;
                                }
                                _ => failed.push(k),
                            }
                        }
                        queue.push_back(Event::Hidden { req, result: Ok(failed) });
                    }
                    Effect::Focus { req, win } => {
                        let ok = self.activate(win);
                        let report = FocusReport { ok, neutral_key: false, elapsed_us: 500 };
                        queue.push_back(Event::Focused { req, result: Ok(report) });
                    }
                    Effect::FocusShell { req } => {
                        self.fg = None;
                        let report = FocusReport { ok: true, neutral_key: false, elapsed_us: 500 };
                        queue.push_back(Event::Focused { req, result: Ok(report) });
                    }
                    Effect::Log(line) => self.logs.push(line),
                    other => panic!("efecto inesperado en la prueba de escritorios: {other:?}"),
                }
            }
        }
    }

    /// Como `SetForegroundWindow`: restaura una minimizada, destapa una con
    /// cloak (la marca queda) y la pasa arriba.
    fn activate(&mut self, k: WinKey) -> bool {
        let Some(i) = self.wins.iter().position(|w| w.key == k) else { return false };
        let mut w = self.wins.remove(i);
        w.cloaked = false;
        w.minimized = false;
        self.wins.insert(0, w);
        self.fg = Some(k);
        true
    }

    fn key(&mut self, world: &mut World, action: HyperAction) {
        self.run(world, Event::Action { action, held: 0, age_ms: 0 });
    }

    fn go(&mut self, world: &mut World, n: u8) {
        self.key(world, HyperAction::Digit { digit: n, thumb: false });
    }

    fn carry(&mut self, world: &mut World, n: u8) {
        self.key(world, HyperAction::Digit { digit: n, thumb: true });
    }

    fn letter(&mut self, world: &mut World, l: u8) {
        self.key(world, HyperAction::Letter { letter: l, thumb: false });
    }

    fn undo(&mut self, world: &mut World) {
        self.key(world, HyperAction::Letter { letter: b'Z', thumb: true });
    }

    fn visible(&self, k: WinKey) -> bool {
        self.wins.iter().any(|w| w.key == k && !w.cloaked)
    }

    /// Invariantes: visible ⇔ de este escritorio (o en todos); la prop
    /// coincide con el modelo; el foco nunca en una escondida; nada en vuelo.
    fn check(&self, world: &World) {
        assert_eq!(world.in_flight(), 0, "quedó algo en vuelo; log: {:?}", self.logs);
        assert!(!world.syncing());
        let active = world.active_desk();
        for w in &self.wins {
            let sticky = world.desks_sticky(w.key);
            let desk = world.desk_of(w.key);
            if sticky {
                assert!(!w.cloaked, "una ventana en todos los escritorios quedó escondida");
                continue;
            }
            let desk = desk.expect("toda ventana tiene escritorio");
            assert_eq!(w.desk, Some(desk), "la prop no coincide con el modelo");
            assert_eq!(
                !w.cloaked,
                desk == active,
                "ventana {:?} del escritorio {desk} con el {active} activo; log: {:?}",
                w.key,
                self.logs
            );
            assert_eq!(w.marked, w.cloaked, "marca y cloak van juntos");
        }
        if let Some(f) = self.fg {
            assert!(
                self.visible(f),
                "el foco quedó en una ventana escondida; log: {:?}",
                self.logs
            );
        }
    }
}

fn boot(apps: &[&str]) -> (World, Plat) {
    let mut world = World::new(Vec::new());
    let mut p = Plat::new(apps);
    p.run(&mut world, Event::Boot { recover: false });
    p.check(&world);
    (world, p)
}

#[test]
fn carry_hides_and_focus_moves_to_the_next_window() {
    let (mut world, mut p) = boot(&["aaa", "bbb", "ccc"]);
    let top = p.fg.expect("foco");
    p.carry(&mut world, 2);
    p.check(&world);
    assert!(!p.visible(top));
    assert_ne!(p.fg, Some(top));
    assert_eq!(world.desk_of(top), Some(2));
    // Ir al 2: solo ella, con el foco.
    p.go(&mut world, 2);
    p.check(&world);
    assert_eq!(world.active_desk(), 2);
    assert_eq!(p.fg, Some(top));
    assert_eq!(p.wins.iter().filter(|w| !w.cloaked).count(), 1);
    // Y de vuelta.
    p.go(&mut world, 1);
    p.check(&world);
    assert_eq!(p.wins.iter().filter(|w| !w.cloaked).count(), 2);
}

#[test]
fn going_back_focuses_the_last_window_used_there() {
    let (mut world, mut p) = boot(&["aaa", "bbb", "ccc"]);
    // Enfocar "bbb" (no es la de arriba) y cambiar de escritorio.
    p.letter(&mut world, b'B');
    let b = p.fg.expect("foco");
    p.go(&mut world, 3);
    p.check(&world);
    assert_eq!(p.fg, None, "escritorio vacío: foco al escritorio de Windows");
    p.go(&mut world, 1);
    p.check(&world);
    assert_eq!(p.fg, Some(b));
}

#[test]
fn undo_brings_a_carried_window_back() {
    let (mut world, mut p) = boot(&["aaa", "bbb"]);
    let top = p.fg.expect("foco");
    p.carry(&mut world, 4);
    assert!(!p.visible(top));
    p.undo(&mut world);
    p.check(&world);
    assert!(p.visible(top));
    assert_eq!(world.desk_of(top), Some(1));
    assert_eq!(p.fg, Some(top));
}

#[test]
fn letter_jumps_to_the_desk_of_the_window() {
    let (mut world, mut p) = boot(&["aaa", "bbb", "ccc"]);
    p.letter(&mut world, b'B');
    let b = p.fg.expect("foco");
    p.carry(&mut world, 2);
    assert_eq!(world.active_desk(), 1);
    p.letter(&mut world, b'B');
    p.check(&world);
    assert_eq!(world.active_desk(), 2);
    assert_eq!(p.fg, Some(b));
}

#[test]
fn an_unmarkable_window_stays_on_every_desk() {
    let mut world = World::new(Vec::new());
    let mut p = Plat::new(&["aaa", "admin"]);
    let admin = p.wins[0].key;
    p.wins[0].unmarkable = true;
    p.run(&mut world, Event::Boot { recover: false });
    p.check(&world);
    assert!(world.desks_sticky(admin));
    p.go(&mut world, 2);
    p.check(&world);
    assert!(p.visible(admin));
}

#[test]
fn boot_after_a_crash_trusts_the_desk_of_each_window() {
    let mut world = World::new(Vec::new());
    let mut p = Plat::new(&["aaa", "bbb", "ccc"]);
    // Como las dejó el core anterior: `restore_all` las mostró a todas, pero
    // `wm.desk` quedó. La del frente es del 2.
    p.wins[0].desk = Some(2);
    p.wins[1].desk = Some(1);
    p.wins[2].desk = Some(2);
    p.run(&mut world, Event::Boot { recover: true });
    p.check(&world);
    assert_eq!(world.active_desk(), 2);
    assert!(!p.visible(p.wins.iter().find(|w| w.desk == Some(1)).unwrap().key));
}

#[test]
fn a_normal_boot_puts_everything_on_desk_1() {
    let mut world = World::new(Vec::new());
    let mut p = Plat::new(&["aaa", "bbb"]);
    p.wins[0].desk = Some(3);
    p.run(&mut world, Event::Boot { recover: false });
    p.check(&world);
    assert_eq!(world.active_desk(), 1);
    assert!(p.wins.iter().all(|w| w.desk == Some(1) && !w.cloaked));
}

#[test]
fn new_windows_are_adopted_on_the_active_desk() {
    let (mut world, mut p) = boot(&["aaa"]);
    p.go(&mut world, 2);
    let new = p.open("nueva");
    p.go(&mut world, 1);
    p.check(&world);
    assert_eq!(world.desk_of(new), Some(2));
    assert!(!p.visible(new));
}

#[test]
fn a_key_during_a_change_waits_its_turn() {
    let (mut world, mut p) = boot(&["aaa", "bbb"]);
    // Dos teclas sin respuestas en el medio: la segunda queda en espera.
    let e1 = world.step(
        Event::Action { action: HyperAction::Digit { digit: 2, thumb: false }, held: 0, age_ms: 0 },
        2_000_000,
    );
    assert!(matches!(e1[0], Effect::Scan { .. }));
    let e2 = world.step(
        Event::Action { action: HyperAction::Digit { digit: 3, thumb: false }, held: 0, age_ms: 0 },
        2_001_000,
    );
    assert!(e2.is_empty(), "{e2:?}");
    let Effect::Scan { req } = e1[0] else { unreachable!() };
    p.run(&mut world, Event::Windows { req, result: Ok((p.scan(), p.fg)) });
    p.check(&world);
    assert_eq!(world.active_desk(), 3);
}

/// 10 000 secuencias al azar: ir, llevar, letras, deshacer, abrir, cerrar y
/// minimizar. Después de cada paso valen las invariantes.
#[test]
fn random_sequences_keep_the_invariants() {
    let mut seed: u64 = 0x5eed_4d35;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let apps = ["aaa", "bbb", "ccc", "ddd"];
    for _ in 0..10_000 / 20 {
        let (mut world, mut p) = boot(&apps[..2]);
        for _ in 0..20 {
            let op = next() % 8;
            p.logs.push(format!("== paso {op}; fg {:?}", p.fg));
            match op {
                0 | 1 => p.go(&mut world, (next() % 4) as u8 + 1),
                2 => p.carry(&mut world, (next() % 4) as u8 + 1),
                3 => p.letter(&mut world, b'A' + (next() % 4) as u8),
                4 => p.undo(&mut world),
                5 => {
                    let app = apps[(next() % 4) as usize];
                    p.open(app);
                    // La adopta el próximo Scan (el de la próxima tecla).
                    let here = world.active_desk();
                    p.go(&mut world, here);
                }
                6 => {
                    // Cerrar la del frente (Windows activa la de abajo visible).
                    if let Some(f) = p.fg {
                        p.wins.retain(|w| w.key != f);
                        p.fg = p.wins.iter().find(|w| !w.cloaked && !w.minimized).map(|w| w.key);
                    }
                }
                _ => {
                    // Minimizar la del frente.
                    if let Some(f) = p.fg
                        && let Some(w) = p.get(f)
                    {
                        w.minimized = true;
                        p.fg = p.wins.iter().find(|w| !w.cloaked && !w.minimized).map(|w| w.key);
                    }
                }
            }
            p.check(&world);
        }
    }
}
