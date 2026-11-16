//! Pruebas del mapa de teclado en [`World`] (hito 2a, plan-core.md §5): las
//! teclas pasan por el [`HyperKeyboard`] real (el del hook) y sus señales de
//! capa y acción llegan al modelo como en el brain.

use crate::keyboard::{
    HyperKeyboard, Inject, KeyEvent, Layer, MAP_DELAY_MS, VK_CAPITAL, VK_ESCAPE, VK_LMENU,
};
use crate::map::MAP_MAX_US;
use crate::world::{Effect, Event, WinInfo, WinKey, World};

const MS: u64 = 1000;

/// El teclado y el modelo con el mismo reloj; solo mira los efectos del mapa.
struct Rig {
    kb: HyperKeyboard,
    w: World,
    now_us: u64,
    /// El mapa se ve (según los efectos recibidos).
    shown: Option<Layer>,
    shows: usize,
    quit: bool,
    toggles: usize,
    resets: usize,
    /// Ventanas abiertas que ve el `Scan` del mapa.
    open: Vec<WinInfo>,
    views: Vec<crate::mapview::MapView>,
    keys_lit: Vec<char>,
}

impl Rig {
    fn new() -> Self {
        let mut w = World::new(Vec::new());
        w.step(Event::MapEnabled(true), 0);
        Self {
            kb: HyperKeyboard::new(),
            w,
            now_us: 1_000 * MS,
            shown: None,
            shows: 0,
            quit: false,
            toggles: 0,
            resets: 0,
            open: Vec::new(),
            views: Vec::new(),
            keys_lit: Vec::new(),
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Scan { req } => {
                    let windows = self.open.clone();
                    self.feed(Event::Windows { req, result: Ok((windows, None)) });
                }
                Effect::MapKey { letter } => {
                    assert!(self.shown.is_some(), "MapKey sin mapa");
                    self.keys_lit.push(letter);
                }
                Effect::ShowMap { layer, views } => {
                    assert_eq!(views.len(), 2);
                    self.views = views;
                    assert!(self.shown.is_none(), "dos ShowMap seguidos");
                    self.shown = Some(layer);
                    self.shows += 1;
                }
                Effect::MapLayer { layer } => {
                    assert!(self.shown.is_some(), "MapLayer sin mapa");
                    self.shown = Some(layer);
                }
                Effect::HideMap => {
                    assert!(self.shown.is_some(), "HideMap sin mapa");
                    self.shown = None;
                }
                Effect::ResetKeyboard => {
                    self.kb.reset();
                    self.resets += 1;
                }
                Effect::Quit => self.quit = true,
                // La capa de apps y de ventana piden ventanas: acá no importa.
                _ => {}
            }
        }
    }

    fn feed(&mut self, e: Event) {
        let effects = self.w.step(e, self.now_us);
        self.apply(effects);
    }

    fn key(&mut self, vk: u8, down: bool) {
        let time_ms = (self.now_us / MS) as u32;
        let d = self.kb.on_key(KeyEvent { vk, down, time_ms });
        if d.inject == Inject::ToggleCapsLock {
            self.toggles += 1;
        }
        if let Some(layer) = d.layer {
            self.feed(Event::Layer(layer));
        }
        if let Some(action) = d.action {
            self.feed(Event::Action { action, held: d.held, age_ms: 0 });
        }
    }

    /// Avanza el reloj atendiendo los plazos del modelo.
    fn wait(&mut self, ms: u64) {
        let until = self.now_us + ms * MS;
        while let Some(t) = self.w.deadline().filter(|&t| t <= until) {
            self.now_us = self.now_us.max(t);
            self.feed(Event::Tick);
        }
        self.now_us = until;
    }

    fn visible(&self) -> bool {
        assert_eq!(self.shown.is_some(), self.w.map_visible());
        self.shown.is_some()
    }
}

#[test]
fn holding_caps_shows_the_map_at_400_ms() {
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(u64::from(MAP_DELAY_MS) - 1);
    assert!(!r.visible());
    r.wait(1);
    assert_eq!(r.shown, Some(Layer::Apps));
    r.key(VK_CAPITAL, false);
    assert!(!r.visible());
    assert_eq!(r.toggles, 0, "si se vio el mapa, Bloq Mayús no cambia");
    assert_eq!(r.w.deadline(), None);
}

#[test]
fn a_tap_never_shows_the_map_and_toggles_caps_lock() {
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(100);
    r.key(VK_CAPITAL, false);
    r.wait(1000);
    assert_eq!(r.shows, 0);
    assert_eq!(r.toggles, 1);
    assert_eq!(r.w.deadline(), None, "el brain vuelve a dormir");
}

#[test]
fn a_quick_shortcut_suppresses_the_map_for_the_whole_press() {
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(50);
    r.key(b'W', true);
    r.key(b'W', false);
    r.wait(2000); // Caps sigue apretada
    assert_eq!(r.shows, 0);
    r.key(VK_CAPITAL, false);
}

#[test]
fn the_thumb_picks_the_layer_before_and_after_showing() {
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(100);
    r.key(VK_LMENU, true); // el pulgar no es una acción
    r.wait(400);
    assert_eq!(r.shown, Some(Layer::Window));
    r.key(VK_LMENU, false);
    assert_eq!(r.shown, Some(Layer::Apps));
    r.key(VK_LMENU, true);
    assert_eq!(r.shown, Some(Layer::Window));
    r.key(VK_LMENU, false);
    r.key(VK_CAPITAL, false);
    assert!(!r.visible());
}

#[test]
fn an_action_with_the_map_visible_keeps_it_and_escape_closes_it_without_quitting() {
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(500);
    r.key(b'W', true);
    r.key(b'W', false);
    assert!(r.visible(), "D7: sigue visible");
    r.key(VK_ESCAPE, true);
    r.key(VK_ESCAPE, false);
    assert!(!r.visible());
    assert!(!r.quit, "Esc cerró el mapa, no el programa");
    r.wait(1000);
    assert_eq!(r.shows, 1, "no vuelve en la misma pulsación");
    r.key(VK_CAPITAL, false);
    // Sin mapa, Caps + Esc sigue saliendo (hasta la bandeja, hito 3).
    r.key(VK_CAPITAL, true);
    r.key(VK_ESCAPE, true);
    assert!(r.quit);
}

#[test]
fn a_keyboard_reset_hides_the_map() {
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(500);
    assert!(r.visible());
    r.kb.reset();
    r.feed(Event::KeyboardReset);
    assert!(!r.visible());
    assert_eq!(r.w.deadline(), None);
}

#[test]
fn the_map_gives_up_after_30_seconds_and_resets_the_keyboard() {
    // Si se perdió "Caps arriba" (ventana elevada al frente), el mapa no queda
    // pegado y Hyper no queda trabado.
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(500);
    r.wait(MAP_MAX_US / MS);
    assert!(!r.visible());
    assert_eq!(r.resets, 1);
    assert!(!r.kb.is_hyper_down());
    assert_eq!(r.w.layer(), Layer::Off);
}

#[test]
fn using_the_map_for_more_than_30_seconds_does_not_reset_the_keyboard() {
    // Prueba del usuario: Caps mantenida más de 30 s acomodando ventanas con
    // el mapa visible; el tope reiniciaba el teclado en pleno uso.
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(500);
    assert!(r.visible());
    for _ in 0..6 {
        r.wait(10_000);
        r.key(VK_LMENU, true);
        r.key(b'A', true);
        r.key(b'A', false);
        r.key(VK_LMENU, false);
    }
    assert!(r.visible(), "60 s en uso: sigue visible");
    assert_eq!(r.resets, 0);
    r.wait(MAP_MAX_US / MS); // 30 s sin tocar nada: ahora sí
    assert!(!r.visible());
    assert_eq!(r.resets, 1);
}

#[test]
fn a_repeated_layer_after_a_silent_reset_starts_a_new_press() {
    // El hook se reinició sin avisar (modo seguro, bloqueo) con el mapa
    // visible y Caps sigue físicamente apretada: la repetición automática
    // vuelve a armar Hyper.
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(500);
    assert!(r.visible());
    r.kb.reset();
    r.key(VK_CAPITAL, true); // repetición: Layer(Apps) otra vez
    assert!(!r.visible(), "se oculta y se vuelve a armar");
    r.wait(400);
    assert!(r.visible());
    r.key(VK_CAPITAL, false);
    assert!(!r.visible());
}

#[test]
fn a_disabled_map_never_shows() {
    let mut r = Rig::new();
    r.feed(Event::MapEnabled(false));
    r.key(VK_CAPITAL, true);
    r.wait(2000);
    assert_eq!(r.shows, 0);
    assert_eq!(r.w.deadline(), None);
}

// ---- Propiedad: el mapa nunca queda pegado ----

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn the_map_always_ends_hidden_and_only_shows_with_hyper_down() {
    let keys = [VK_CAPITAL, VK_LMENU, b'W', b'A', b'B', VK_ESCAPE, b'1'];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for seq in 0..10_000 {
        let mut r = Rig::new();
        let mut down = [false; 7];
        // Toques de Caps de esta pulsación que mostraron el mapa.
        let mut shows_at_press = 0;
        for _ in 0..rng.below(30) {
            match rng.below(10) {
                0..=5 => {
                    let i = rng.below(keys.len() as u64) as usize;
                    let vk = keys[i];
                    let press = !down[i];
                    if vk == VK_CAPITAL && press {
                        shows_at_press = r.shows;
                    }
                    let toggles = r.toggles;
                    r.key(vk, press);
                    down[i] = press;
                    if r.toggles > toggles {
                        assert_eq!(r.shows, shows_at_press, "secuencia {seq}: mapa + Bloq Mayús");
                    }
                }
                6 | 7 => r.wait(rng.below(700)),
                8 => {
                    // Reinicio silencioso del hook (modo seguro, bloqueo).
                    r.kb.reset();
                }
                _ => {
                    r.kb.reset();
                    r.feed(Event::KeyboardReset);
                }
            }
            if r.quit {
                break;
            }
            if r.visible() {
                assert_ne!(r.w.layer(), Layer::Off, "secuencia {seq}: mapa con Hyper arriba");
            }
        }
        // Soltar todo lo que quedó apretado.
        for (i, &vk) in keys.iter().enumerate() {
            if down[i] {
                r.key(vk, false);
            }
        }
        r.wait(1000);
        // Un reinicio silencioso puede haber perdido el "Caps arriba": el
        // tope de 30 s lo cubre.
        r.wait(MAP_MAX_US / MS + 1000);
        assert!(!r.visible(), "secuencia {seq}: el mapa quedó visible");
        assert_eq!(r.w.deadline(), None, "secuencia {seq}: quedó un plazo");
    }
}

#[test]
fn the_map_lists_the_open_apps_and_lights_the_key_just_used() {
    use crate::mapview::Tone;
    let mut r = Rig::new();
    r.open =
        vec![WinInfo { key: WinKey { hwnd: 1, pid: 10 }, app: "brave".into(), title: "x".into() }];
    r.key(VK_CAPITAL, true);
    r.wait(500);
    let apps = &r.views[0];
    let b = apps.cells.iter().find(|c| c.letter == 'B').unwrap();
    assert_eq!((b.label.as_str(), b.tone), ("Brave", Tone::Normal), "automática por inicial");
    r.key(b'Q', true);
    r.key(b'Q', false);
    assert_eq!(r.keys_lit, vec!['Q']);
    r.key(VK_CAPITAL, false);
}

#[test]
fn releasing_caps_before_the_scan_answers_shows_nothing() {
    let mut r = Rig::new();
    r.key(VK_CAPITAL, true);
    r.wait(399);
    // El plazo vence y pide las ventanas; Caps se suelta antes de la respuesta.
    r.now_us += 1000;
    let effects = r.w.step(Event::Tick, r.now_us);
    let Some(Effect::Scan { req }) = effects.first().cloned() else { panic!("{effects:?}") };
    r.key(VK_CAPITAL, false);
    let effects = r.w.step(Event::Windows { req, result: Ok((Vec::new(), None)) }, r.now_us);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(r.shows, 0);
}
