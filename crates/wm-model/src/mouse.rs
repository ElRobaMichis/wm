//! Botones del mouse con `Caps` apretada (hito 5, `plan-hito5.md` §5a).
//!
//! Con Hyper abajo, el clic izquierdo y el derecho se tragan (la app no los
//! recibe) y se avisan al brain: el izquierdo arrastra para mover, el derecho
//! para cambiar el tamaño. El "arriba" de un botón tragado se traga siempre,
//! aunque `Caps` ya se haya soltado: un `WM_RBUTTONUP` huérfano abriría el
//! menú contextual. Todo lo demás pasa. O(1), sin memoria: corre en el hook.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    Down(Button),
    Up(Button),
}

/// Qué hacer con un evento: dejarlo pasar y, si hay, qué avisar al brain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseDecision {
    pub pass: bool,
    /// Empieza (`Down`) o termina (`Up`) un gesto.
    pub signal: Option<MouseEvent>,
}

impl MouseDecision {
    const PASS: Self = Self { pass: true, signal: None };
    const SWALLOW: Self = Self { pass: false, signal: None };
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HyperMouse {
    /// Botones cuyo "abajo" se tragó y que siguen abajo.
    left: bool,
    right: bool,
    /// El botón del gesto en curso (el primero que bajó).
    active: Option<Button>,
}

impl HyperMouse {
    pub const fn new() -> Self {
        Self { left: false, right: false, active: None }
    }

    fn slot(&mut self, b: Button) -> &mut bool {
        match b {
            Button::Left => &mut self.left,
            Button::Right => &mut self.right,
        }
    }

    pub fn on_event(&mut self, e: MouseEvent, hyper: bool) -> MouseDecision {
        match e {
            MouseEvent::Down(b) => {
                if *self.slot(b) {
                    // Doble abajo sin arriba (se perdió uno): se sigue tragando.
                    return MouseDecision::SWALLOW;
                }
                if !hyper {
                    return MouseDecision::PASS;
                }
                *self.slot(b) = true;
                if self.active.is_none() {
                    self.active = Some(b);
                    return MouseDecision { pass: false, signal: Some(e) };
                }
                // Un segundo botón durante el gesto: se traga sin aviso.
                MouseDecision::SWALLOW
            }
            MouseEvent::Up(b) => {
                if !*self.slot(b) {
                    return MouseDecision::PASS;
                }
                *self.slot(b) = false;
                if self.active == Some(b) {
                    self.active = None;
                    return MouseDecision { pass: false, signal: Some(e) };
                }
                MouseDecision::SWALLOW
            }
        }
    }

    /// Hay algún botón tragado todavía abajo (el hook tiene que seguir).
    pub fn held(&self) -> bool {
        self.left || self.right
    }

    /// Vuelve a reposo (reinicio del teclado, pausa, bloqueo).
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::{HyperKeyboard, Inject, KeyEvent};

    const CAPS: u8 = 0x14;

    #[test]
    fn a_click_with_caps_is_swallowed_and_signalled() {
        let mut m = HyperMouse::new();
        let d = m.on_event(MouseEvent::Down(Button::Left), true);
        assert_eq!(d, MouseDecision { pass: false, signal: Some(MouseEvent::Down(Button::Left)) });
        assert!(m.held());
        // Soltar Caps antes que el botón: el arriba se traga igual.
        let d = m.on_event(MouseEvent::Up(Button::Left), false);
        assert_eq!(d, MouseDecision { pass: false, signal: Some(MouseEvent::Up(Button::Left)) });
        assert!(!m.held());
    }

    #[test]
    fn without_caps_everything_passes() {
        let mut m = HyperMouse::new();
        assert!(m.on_event(MouseEvent::Down(Button::Right), false).pass);
        assert!(m.on_event(MouseEvent::Up(Button::Right), false).pass);
        // Un botón que bajó antes de Caps sube normal.
        assert!(m.on_event(MouseEvent::Down(Button::Left), false).pass);
        assert!(m.on_event(MouseEvent::Up(Button::Left), true).pass);
    }

    #[test]
    fn a_second_button_during_a_gesture_is_swallowed_quietly() {
        let mut m = HyperMouse::new();
        m.on_event(MouseEvent::Down(Button::Left), true);
        let d = m.on_event(MouseEvent::Down(Button::Right), true);
        assert_eq!(d, MouseDecision::SWALLOW);
        assert_eq!(m.on_event(MouseEvent::Up(Button::Right), true), MouseDecision::SWALLOW);
        assert!(m.on_event(MouseEvent::Up(Button::Left), true).signal.is_some());
    }

    /// 10 000 secuencias al azar: el sistema nunca ve un "arriba" sin su
    /// "abajo" (ni al revés), las señales alternan abajo/arriba y, al final,
    /// con los botones sueltos, no queda nada retenido. Un clic tragado dentro
    /// de una pulsación de Caps nunca cambia Bloq Mayús.
    #[test]
    fn random_sequences_never_leave_the_system_confused() {
        let mut seed: u64 = 0x6d6f_7573;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..10_000 {
            let mut m = HyperMouse::new();
            let mut kb = HyperKeyboard::new();
            // Lo que el sistema cree: botones abajo.
            let (mut sys_l, mut sys_r) = (false, false);
            // Lo que el usuario tiene apretado de verdad.
            let (mut real_l, mut real_r) = (false, false);
            let mut caps = false;
            let mut signal_down = false;
            let mut clicked_in_press = false;
            let mut t = 0u32;
            for _ in 0..30 {
                t += 30;
                match next() % 6 {
                    0 => {
                        caps = !caps;
                        let d = kb.on_key(KeyEvent { vk: CAPS, down: caps, time_ms: t });
                        if caps {
                            clicked_in_press = false;
                        } else if clicked_in_press {
                            assert_ne!(d.inject, Inject::ToggleCapsLock);
                        }
                    }
                    1..=4 => {
                        let left = next() % 2 == 0;
                        let (real, sys) = if left {
                            (&mut real_l, &mut sys_l)
                        } else {
                            (&mut real_r, &mut sys_r)
                        };
                        let b = if left { Button::Left } else { Button::Right };
                        let e = if *real { MouseEvent::Up(b) } else { MouseEvent::Down(b) };
                        *real = !*real;
                        let d = m.on_event(e, kb.is_hyper_down());
                        if let Some(s) = d.signal {
                            match s {
                                MouseEvent::Down(_) => {
                                    assert!(!signal_down, "dos inicios seguidos");
                                    signal_down = true;
                                    kb.mark_used();
                                    clicked_in_press = true;
                                }
                                MouseEvent::Up(_) => {
                                    assert!(signal_down, "fin sin inicio");
                                    signal_down = false;
                                }
                            }
                        }
                        if d.pass {
                            match e {
                                MouseEvent::Down(_) => {
                                    assert!(!*sys, "abajo dos veces para el sistema");
                                    *sys = true;
                                }
                                MouseEvent::Up(_) => {
                                    assert!(*sys, "arriba sin abajo para el sistema");
                                    *sys = false;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Soltar todo.
            for (real, b) in [(real_l, Button::Left), (real_r, Button::Right)] {
                if real {
                    m.on_event(MouseEvent::Up(b), kb.is_hyper_down());
                }
            }
            assert!(!m.held());
        }
    }
}
