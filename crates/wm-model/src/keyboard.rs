//! Máquina de estados de la tecla Hyper (Caps Lock mantenido).
//!
//! Es lógica pura: recibe eventos físicos de teclado y decide, para cada uno,
//! si el sistema lo ve, si hay que inyectar algo y qué acción de Hyper se
//! disparó. El hook de bajo nivel (wm-platform) solo la ejecuta.
//!
//! Reglas (architecture.md §5.2, keyboard-design.md §5–§6):
//! - Caps abajo/arriba nunca llegan al sistema. Un toque corto sin otra tecla
//!   se convierte en un Bloq Mayús inyectado.
//! - Con Caps abajo, toda tecla nueva se traga y puede disparar una acción.
//! - Invariante contra teclas trabadas: si el "abajo" de una tecla llegó al
//!   sistema, su "arriba" también llega; si se tragó, su "arriba" también se
//!   traga.
//! - Alt/Win apretados **antes** de Caps ya llegaron al sistema. Si se usó
//!   Hyper mientras estaban abajo, al soltarlos se inyecta una tecla máscara
//!   para que Windows no abra el menú de la app ni el menú Inicio.
//! - **Capa** ([`Layer`]): ninguna, apps (Caps) o ventana (Caps + pulgar). La
//!   decisión la informa solo cuando cambia, así el brain puede mostrar y
//!   ocultar el mapa sin perder pares (plan-core.md §1.4, invariante 8).

pub const VK_SHIFT: u8 = 0x10;
pub const VK_CAPITAL: u8 = 0x14;
pub const VK_ESCAPE: u8 = 0x1B;
pub const VK_LWIN: u8 = 0x5B;
pub const VK_RWIN: u8 = 0x5C;
pub const VK_LMENU: u8 = 0xA4;
pub const VK_RMENU: u8 = 0xA5;

/// Mantener Caps sola este tiempo muestra el mapa de teclado
/// (keyboard-design.md §10.1, plan-core.md §5.1).
pub const MAP_DELAY_MS: u32 = 400;

/// Un toque de Caps (Bloq Mayús) dura menos que esto. Es menor que
/// [`MAP_DELAY_MS`] a propósito: el toque se mide con la hora del hook (tick
/// de 10–16 ms) y el mapa con el reloj del brain, así que con el mismo
/// umbral una pulsación de ~400–416 ms podía mostrar el mapa y además
/// cambiar Bloq Mayús. Entre 350 y 400 ms no pasa nada (banda muerta).
pub const TAP_MAX_MS: u32 = 350;

/// Modificadores que, si llegaron al sistema antes de Hyper, necesitan una
/// tecla máscara al soltarse.
const MASKED_MODIFIERS: [u8; 4] = [VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN];

/// Tecla del pulgar para la capa de ventana (Alt izquierdo).
pub const THUMB: u8 = VK_LMENU;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// Código de tecla virtual (Windows `VK_*`).
    pub vk: u8,
    pub down: bool,
    /// Milisegundos del reloj del sistema (puede dar la vuelta).
    pub time_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inject {
    None,
    /// Inyectar Caps abajo + arriba (alterna Bloq Mayús y su LED).
    ToggleCapsLock,
    /// Inyectar tecla máscara abajo + arriba y después `vk` arriba.
    MaskThenRelease(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyperAction {
    /// Letra A–Z (según la distribución activa). `thumb` = pulgar apretado.
    Letter { letter: u8, thumb: bool },
    /// Dígito 0–9 de la fila superior.
    Digit { digit: u8, thumb: bool },
    /// Hyper + Esc.
    Escape,
    /// Cualquier otra tecla.
    Other { vk: u8, thumb: bool },
}

/// Capa activa del teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    /// Sin Hyper.
    #[default]
    Off,
    /// Hyper (Caps mantenido): letras = apps, números = escritorios.
    Apps,
    /// Hyper + pulgar: acomodar la ventana.
    Window,
}

/// Direcciones de la capa de ventana mantenidas, en bits ([`Decision::held`]).
pub const HELD_W: u8 = 1;
pub const HELD_A: u8 = 2;
pub const HELD_S: u8 = 4;
pub const HELD_D: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// `true`: el evento llega al sistema. `false`: se traga.
    pub pass: bool,
    pub inject: Inject,
    pub action: Option<HyperAction>,
    /// La capa cambió a este valor con este evento.
    pub layer: Option<Layer>,
    /// Con una acción: qué direcciones (W/A/S/D, ver `HELD_*`) estaban
    /// mantenidas, incluida la de la acción. Sirve para las esquinas.
    pub held: u8,
}

impl Decision {
    const PASS: Self =
        Self { pass: true, inject: Inject::None, action: None, layer: None, held: 0 };
    const SWALLOW: Self =
        Self { pass: false, inject: Inject::None, action: None, layer: None, held: 0 };
}

/// Conjunto de 256 teclas virtuales.
#[derive(Debug, Clone, Copy, Default)]
struct KeySet([u64; 4]);

impl KeySet {
    const fn new() -> Self {
        Self([0; 4])
    }
    fn get(&self, vk: u8) -> bool {
        self.0[usize::from(vk >> 6)] & (1 << (vk & 63)) != 0
    }
    fn set(&mut self, vk: u8) {
        self.0[usize::from(vk >> 6)] |= 1 << (vk & 63);
    }
    fn clear(&mut self, vk: u8) {
        self.0[usize::from(vk >> 6)] &= !(1 << (vk & 63));
    }
}

#[derive(Debug, Clone)]
pub struct HyperKeyboard {
    caps_down: bool,
    caps_since: u32,
    /// Se apretó alguna otra tecla durante este Hyper.
    chord: bool,
    /// Teclas cuyo "abajo" se tragó.
    swallowed: KeySet,
    /// Teclas cuyo "abajo" llegó al sistema y siguen abajo.
    passed: KeySet,
    /// Modificadores que necesitan máscara al soltarse.
    need_mask: KeySet,
}

impl Default for HyperKeyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl HyperKeyboard {
    pub const fn new() -> Self {
        Self {
            caps_down: false,
            caps_since: 0,
            chord: false,
            swallowed: KeySet::new(),
            passed: KeySet::new(),
            need_mask: KeySet::new(),
        }
    }

    pub fn is_hyper_down(&self) -> bool {
        self.caps_down
    }

    /// Capa activa ahora.
    pub fn layer(&self) -> Layer {
        if !self.caps_down {
            Layer::Off
        } else if self.swallowed.get(THUMB) || self.passed.get(THUMB) {
            Layer::Window
        } else {
            Layer::Apps
        }
    }

    /// Direcciones tragadas (apretadas con Hyper) que siguen abajo.
    fn held(&self) -> u8 {
        [(b'W', HELD_W), (b'A', HELD_A), (b'S', HELD_S), (b'D', HELD_D)]
            .into_iter()
            .filter(|&(vk, _)| self.swallowed.get(vk))
            .fold(0, |acc, (_, bit)| acc | bit)
    }

    pub fn on_key(&mut self, e: KeyEvent) -> Decision {
        let before = self.layer();
        let mut d = self.decide(e);
        let after = self.layer();
        if after != before {
            d.layer = Some(after);
        }
        if d.action.is_some() {
            d.held = self.held();
        }
        d
    }

    fn decide(&mut self, e: KeyEvent) -> Decision {
        if e.vk == VK_CAPITAL {
            return self.on_caps(e);
        }
        if !e.down {
            return self.release(e.vk);
        }
        if self.passed.get(e.vk) {
            // Repetición automática de una tecla que bajó antes de Hyper: con
            // Hyper abajo no debe escribir nada; sin Hyper, sigue normal.
            return if self.caps_down { Decision::SWALLOW } else { Decision::PASS };
        }
        if self.swallowed.get(e.vk) {
            // Repetición de una tecla tragada: se sigue tragando, sin acción.
            return Decision::SWALLOW;
        }
        if !self.caps_down {
            self.passed.set(e.vk);
            return Decision::PASS;
        }

        // Tecla nueva con Hyper abajo.
        self.swallowed.set(e.vk);
        self.chord = true;
        for m in MASKED_MODIFIERS {
            if self.passed.get(m) {
                self.need_mask.set(m);
            }
        }
        let thumb = e.vk != THUMB && (self.swallowed.get(THUMB) || self.passed.get(THUMB));
        let action = match e.vk {
            b'A'..=b'Z' => Some(HyperAction::Letter { letter: e.vk, thumb }),
            b'0'..=b'9' => Some(HyperAction::Digit { digit: e.vk - b'0', thumb }),
            VK_ESCAPE => Some(HyperAction::Escape),
            VK_LMENU | VK_RMENU | VK_LWIN | VK_RWIN | VK_SHIFT | 0xA0..=0xA3 => None,
            vk => Some(HyperAction::Other { vk, thumb }),
        };
        Decision { pass: false, inject: Inject::None, action, layer: None, held: 0 }
    }

    fn on_caps(&mut self, e: KeyEvent) -> Decision {
        if e.down {
            if !self.caps_down {
                self.caps_down = true;
                self.caps_since = e.time_ms;
                self.chord = false;
            }
            return Decision::SWALLOW;
        }
        let was_down = self.caps_down;
        self.caps_down = false;
        let quick = e.time_ms.wrapping_sub(self.caps_since) < TAP_MAX_MS;
        let inject =
            if was_down && !self.chord && quick { Inject::ToggleCapsLock } else { Inject::None };
        Decision { pass: false, inject, action: None, layer: None, held: 0 }
    }

    fn release(&mut self, vk: u8) -> Decision {
        if self.swallowed.get(vk) {
            self.swallowed.clear(vk);
            return Decision::SWALLOW;
        }
        self.passed.clear(vk);
        if self.need_mask.get(vk) {
            self.need_mask.clear(vk);
            return Decision {
                pass: false,
                inject: Inject::MaskThenRelease(vk),
                action: None,
                layer: None,
                held: 0,
            };
        }
        Decision::PASS
    }

    /// Con Hyper abajo se usó el mouse (hito 5): cuenta como una tecla del
    /// acorde. Soltar Caps ya no cambia Bloq Mayús, y Alt/Win que habían
    /// pasado se enmascaran al soltarse (no abren menús).
    pub fn mark_used(&mut self) {
        if !self.caps_down {
            return;
        }
        self.chord = true;
        for m in MASKED_MODIFIERS {
            if self.passed.get(m) {
                self.need_mask.set(m);
            }
        }
    }

    /// Vuelve al estado de reposo sin dejar teclas trabadas del lado nuestro
    /// (al reinstalar el hook, al bloquear la sesión, etc.).
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(vk: u8, down: bool, time_ms: u32) -> KeyEvent {
        KeyEvent { vk, down, time_ms }
    }

    #[test]
    fn without_caps_everything_passes() {
        let mut k = HyperKeyboard::new();
        for (i, &vk) in [b'A', VK_LMENU, b'1', VK_LWIN].iter().enumerate() {
            assert_eq!(k.on_key(ev(vk, true, i as u32)), Decision::PASS);
            assert_eq!(k.on_key(ev(vk, false, i as u32)), Decision::PASS);
        }
    }

    #[test]
    fn short_tap_toggles_caps_lock() {
        let mut k = HyperKeyboard::new();
        assert!(!k.on_key(ev(VK_CAPITAL, true, 1000)).pass);
        let up = k.on_key(ev(VK_CAPITAL, false, 1100));
        assert_eq!(up.inject, Inject::ToggleCapsLock);
        assert!(!up.pass);
    }

    #[test]
    fn long_hold_alone_does_not_toggle() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, 1000));
        assert_eq!(k.on_key(ev(VK_CAPITAL, false, 1000 + MAP_DELAY_MS)).inject, Inject::None);
    }

    #[test]
    fn the_tap_ends_before_the_map_can_show() {
        // Banda muerta: entre el toque y el mapa no pasa nada.
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, 1000));
        let up = k.on_key(ev(VK_CAPITAL, false, 1000 + TAP_MAX_MS - 1));
        assert_eq!(up.inject, Inject::ToggleCapsLock);
        k.on_key(ev(VK_CAPITAL, true, 5000));
        let up = k.on_key(ev(VK_CAPITAL, false, 5000 + TAP_MAX_MS));
        assert_eq!(up.inject, Inject::None, "a los 350 ms ya no es toque");
        const { assert!(TAP_MAX_MS + 30 < MAP_DELAY_MS) };
    }

    #[test]
    fn tap_survives_clock_wraparound() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, u32::MAX - 50));
        assert_eq!(k.on_key(ev(VK_CAPITAL, false, 49)).inject, Inject::ToggleCapsLock);
    }

    #[test]
    fn chord_emits_letter_and_does_not_toggle() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, 0));
        let d = k.on_key(ev(b'C', true, 10));
        assert!(!d.pass);
        assert_eq!(d.action, Some(HyperAction::Letter { letter: b'C', thumb: false }));
        assert!(!k.on_key(ev(b'C', false, 20)).pass);
        assert_eq!(k.on_key(ev(VK_CAPITAL, false, 30)).inject, Inject::None);
    }

    #[test]
    fn auto_repeat_does_not_repeat_the_action() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, 0));
        assert!(k.on_key(ev(b'C', true, 10)).action.is_some());
        assert!(k.on_key(ev(b'C', true, 40)).action.is_none());
        assert!(k.on_key(ev(b'C', true, 70)).action.is_none());
    }

    #[test]
    fn thumb_after_caps_is_swallowed_and_marks_the_window_layer() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, 0));
        let alt = k.on_key(ev(VK_LMENU, true, 5));
        assert!(!alt.pass);
        assert_eq!(alt.action, None);
        let a = k.on_key(ev(b'A', true, 10));
        assert_eq!(a.action, Some(HyperAction::Letter { letter: b'A', thumb: true }));
        assert!(!k.on_key(ev(b'A', false, 20)).pass);
        assert!(!k.on_key(ev(VK_LMENU, false, 25)).pass);
    }

    #[test]
    fn alt_before_caps_is_masked_on_release() {
        let mut k = HyperKeyboard::new();
        assert!(k.on_key(ev(VK_LMENU, true, 0)).pass); // la app ya vio Alt abajo
        k.on_key(ev(VK_CAPITAL, true, 5));
        let d = k.on_key(ev(b'D', true, 10));
        assert_eq!(d.action, Some(HyperAction::Letter { letter: b'D', thumb: true }));
        k.on_key(ev(b'D', false, 20));
        k.on_key(ev(VK_CAPITAL, false, 30));
        let up = k.on_key(ev(VK_LMENU, false, 40));
        assert!(!up.pass);
        assert_eq!(up.inject, Inject::MaskThenRelease(VK_LMENU));
    }

    #[test]
    fn alt_before_caps_without_hyper_use_is_left_alone() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_LMENU, true, 0));
        k.on_key(ev(VK_CAPITAL, true, 5));
        k.on_key(ev(VK_CAPITAL, false, 50));
        assert_eq!(k.on_key(ev(VK_LMENU, false, 60)), Decision::PASS);
    }

    #[test]
    fn layer_changes_are_reported_once() {
        let mut k = HyperKeyboard::new();
        assert_eq!(k.on_key(ev(VK_CAPITAL, true, 0)).layer, Some(Layer::Apps));
        assert_eq!(k.on_key(ev(VK_CAPITAL, true, 30)).layer, None, "repetición");
        assert_eq!(k.on_key(ev(VK_LMENU, true, 40)).layer, Some(Layer::Window));
        assert_eq!(k.on_key(ev(VK_LMENU, false, 50)).layer, Some(Layer::Apps));
        assert_eq!(k.on_key(ev(VK_CAPITAL, false, 60)).layer, Some(Layer::Off));
    }

    #[test]
    fn alt_held_before_caps_starts_in_window_layer() {
        let mut k = HyperKeyboard::new();
        assert_eq!(k.on_key(ev(VK_LMENU, true, 0)).layer, None);
        assert_eq!(k.on_key(ev(VK_CAPITAL, true, 5)).layer, Some(Layer::Window));
        assert_eq!(k.on_key(ev(VK_LMENU, false, 9)).layer, Some(Layer::Apps));
    }

    #[test]
    fn actions_carry_the_held_directions() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, 0));
        k.on_key(ev(VK_LMENU, true, 1));
        assert_eq!(k.on_key(ev(b'W', true, 10)).held, HELD_W);
        assert_eq!(k.on_key(ev(b'A', true, 20)).held, HELD_W | HELD_A, "esquina");
        k.on_key(ev(b'W', false, 30));
        assert_eq!(k.on_key(ev(b'D', true, 40)).held, HELD_A | HELD_D);
        assert_eq!(k.on_key(ev(b'F', true, 50)).held, HELD_A | HELD_D);
    }

    #[test]
    fn letter_released_after_caps_still_swallowed() {
        let mut k = HyperKeyboard::new();
        k.on_key(ev(VK_CAPITAL, true, 0));
        k.on_key(ev(b'S', true, 10));
        k.on_key(ev(VK_CAPITAL, false, 20));
        assert!(!k.on_key(ev(b'S', false, 30)).pass, "su abajo se tragó");
    }

    // ---- Propiedad: nunca hay teclas trabadas ni "arriba" huérfanos ----

    /// Generador pseudoaleatorio determinista (xorshift), sin dependencias.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Lo que el sistema cree que está apretado, según lo que le dejamos pasar
    /// y lo que inyectamos.
    #[derive(Default)]
    struct SystemView(KeySet);

    impl SystemView {
        fn apply(&mut self, e: KeyEvent, d: Decision) {
            if d.pass {
                if e.down {
                    self.0.set(e.vk);
                } else {
                    assert!(self.0.get(e.vk), "arriba huérfano de {:#x}", e.vk);
                    self.0.clear(e.vk);
                }
            }
            match d.inject {
                Inject::None | Inject::ToggleCapsLock => {}
                Inject::MaskThenRelease(vk) => {
                    assert!(self.0.get(vk), "máscara para {vk:#x} que el sistema no tiene abajo");
                    self.0.clear(vk);
                }
            }
        }
        fn is_empty(&self) -> bool {
            self.0.0 == [0; 4]
        }
    }

    #[test]
    fn random_sequences_never_leave_stuck_keys() {
        const KEYS: [u8; 11] = [
            VK_CAPITAL, b'A', b'C', b'1', b'W', b'D', VK_LMENU, VK_RMENU, VK_LWIN, VK_SHIFT,
            VK_ESCAPE,
        ];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..10_000 {
            let mut k = HyperKeyboard::new();
            let mut sys = SystemView::default();
            let mut physical = KeySet::new();
            let mut t: u32 = rng.next() as u32; // incluye vueltas del reloj
            let mut seen = Layer::Off;
            for _ in 0..rng.below(40) + 1 {
                t = t.wrapping_add(rng.below(700) as u32);
                let vk = KEYS[rng.below(KEYS.len() as u64) as usize];
                // Abajo si no está apretada; si está, arriba o repetición.
                let down = !physical.get(vk) || rng.below(4) == 0;
                if down {
                    physical.set(vk);
                } else {
                    physical.clear(vk);
                }
                let e = ev(vk, down, t);
                let d = k.on_key(e);
                sys.apply(e, d);
                if let Some(l) = d.layer {
                    assert_ne!(l, seen, "señal de capa repetida");
                    seen = l;
                }
                assert_eq!(seen, k.layer(), "la capa vista por las señales se desfasó");
            }
            // Soltar todo lo que quedó apretado físicamente.
            for vk in KEYS {
                if physical.get(vk) {
                    t = t.wrapping_add(rng.below(300) as u32);
                    let e = ev(vk, false, t);
                    let d = k.on_key(e);
                    sys.apply(e, d);
                    if let Some(l) = d.layer {
                        seen = l;
                    }
                }
            }
            assert_eq!(seen, Layer::Off, "la capa no volvió a Off al soltar todo");
            assert!(sys.is_empty(), "quedaron teclas trabadas en el sistema");
            assert!(!k.is_hyper_down());
        }
    }
}
