//! La geometría del arrastre con el mouse (hito 5, `plan-hito5.md` §5b-5c):
//! dónde va la ventana para cada posición del cursor. Pura; el carril
//! `wm-drag` la consulta una vez por cuadro.

use crate::Rect;

/// Mover, o cambiar el tamaño desde una esquina (la opuesta queda fija).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Move,
    /// La esquina que se mueve: `left` (si no, la derecha) y `top` (si no,
    /// la de abajo).
    Resize {
        left: bool,
        top: bool,
    },
}

/// La esquina del cuadrante donde está el cursor.
pub fn corner_of(frame: Rect, p: (i32, i32)) -> Mode {
    let cx = (frame.left + frame.right) / 2;
    let cy = (frame.top + frame.bottom) / 2;
    Mode::Resize { left: p.0 < cx, top: p.1 < cy }
}

/// Una maximizada que se empieza a arrastrar vuelve a su tamaño normal
/// (`size`, ancho y alto exteriores) bajo el cursor, en la misma proporción
/// horizontal, como al arrastrar el título en Windows.
pub fn unmaximized(maxed: Rect, size: (i32, i32), grab: (i32, i32)) -> Rect {
    let (w, h) = size;
    let fx = f64::from(grab.0 - maxed.left) / f64::from(maxed.width().max(1));
    let left = grab.0 - (fx * f64::from(w)).round() as i32;
    let dy = (grab.1 - maxed.top).clamp(0, (h - 1).max(0));
    let top = grab.1 - dy;
    Rect::new(left, top, left + w, top + h)
}

/// Límites de tamaño (exterior) para cambiar el tamaño.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub min_w: i32,
    pub min_h: i32,
    pub max_w: i32,
    pub max_h: i32,
}

impl Bounds {
    /// Lo mínimo razonable: que se vea el título y los botones.
    pub const DEFAULT: Self = Self { min_w: 160, min_h: 80, max_w: i32::MAX, max_h: i32::MAX };
}

/// Un arrastre en curso.
#[derive(Debug, Clone, Copy)]
pub struct Session {
    mode: Mode,
    grab: (i32, i32),
    start: Rect,
    bounds: Bounds,
    slop: i32,
    started: bool,
}

impl Session {
    /// `grab`: dónde se apretó el botón; `start`: el rectángulo exterior de
    /// la ventana en ese momento; `slop`: cuánto hay que mover para que
    /// empiece (`SM_CXDRAG`: un clic quieto no mueve nada).
    pub fn new(mode: Mode, grab: (i32, i32), start: Rect, bounds: Bounds, slop: i32) -> Self {
        Self { mode, grab, start, bounds, slop: slop.max(0), started: false }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Ya pasó el umbral (hubo movimiento de verdad).
    pub fn moved(&self) -> bool {
        self.started
    }

    /// Dónde va la ventana con el cursor en `p`. `None` hasta pasar el umbral.
    pub fn at(&mut self, p: (i32, i32)) -> Option<Rect> {
        let (dx, dy) = (p.0 - self.grab.0, p.1 - self.grab.1);
        if !self.started {
            if dx.abs() <= self.slop && dy.abs() <= self.slop {
                return None;
            }
            self.started = true;
        }
        let s = self.start;
        Some(match self.mode {
            Mode::Move => Rect::new(s.left + dx, s.top + dy, s.right + dx, s.bottom + dy),
            Mode::Resize { left, top } => {
                let b = self.bounds;
                let (mut l, mut t, mut r, mut bo) = (s.left, s.top, s.right, s.bottom);
                if left {
                    l = (s.left + dx).clamp(s.right - b.max_w, s.right - b.min_w);
                } else {
                    r = (s.right + dx).clamp(s.left + b.min_w, s.left.saturating_add(b.max_w));
                }
                if top {
                    t = (s.top + dy).clamp(s.bottom - b.max_h, s.bottom - b.min_h);
                } else {
                    bo = (s.bottom + dy).clamp(s.top + b.min_h, s.top.saturating_add(b.max_h));
                }
                Rect::new(l, t, r, bo)
            }
        })
    }

    /// Se pidió `asked` y la app quedó en `got` (tamaño mínimo o máximo): se
    /// aprende, y desde el cuadro siguiente la esquina opuesta queda fija con
    /// ese límite (si no, la ventana "resbala" al tirar contra su mínimo).
    pub fn learn(&mut self, asked: Rect, got: Rect) {
        if !matches!(self.mode, Mode::Resize { .. }) {
            return;
        }
        let b = &mut self.bounds;
        if got.width() > asked.width() {
            b.min_w = b.min_w.max(got.width());
        } else if got.width() < asked.width() {
            b.max_w = b.max_w.min(got.width()).max(b.min_w);
        }
        if got.height() > asked.height() {
            b.min_h = b.min_h.max(got.height());
        } else if got.height() < asked.height() {
            b.max_h = b.max_h.min(got.height()).max(b.min_h);
        }
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Rect = Rect::new(100, 100, 500, 400);

    #[test]
    fn a_click_without_moving_does_nothing() {
        let mut s = Session::new(Mode::Move, (300, 200), START, Bounds::DEFAULT, 4);
        assert_eq!(s.at((302, 203)), None);
        assert!(!s.moved());
        assert_eq!(s.at((320, 200)), Some(Rect::new(120, 100, 520, 400)));
        // Ya empezado, volver cerca del punto de agarre igual mueve.
        assert_eq!(s.at((301, 200)), Some(Rect::new(101, 100, 501, 400)));
    }

    #[test]
    fn resizing_keeps_the_opposite_corner_and_the_limits() {
        let mut seed: u64 = 0x6472_6167;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..10_000 {
            let left = next() % 2 == 0;
            let top = next() % 2 == 0;
            let b = Bounds { min_w: 200, min_h: 120, max_w: 900, max_h: 700 };
            let mut s = Session::new(Mode::Resize { left, top }, (300, 250), START, b, 0);
            let p = ((next() % 3000) as i32 - 1000, (next() % 3000) as i32 - 1000);
            let Some(r) = s.at(p) else { continue };
            assert!((b.min_w..=b.max_w).contains(&r.width()), "{r:?}");
            assert!((b.min_h..=b.max_h).contains(&r.height()), "{r:?}");
            // La esquina opuesta no se mueve.
            if left {
                assert_eq!(r.right, START.right);
            } else {
                assert_eq!(r.left, START.left);
            }
            if top {
                assert_eq!(r.bottom, START.bottom);
            } else {
                assert_eq!(r.top, START.top);
            }
        }
    }

    #[test]
    fn a_learned_minimum_anchors_the_opposite_corner() {
        let mode = Mode::Resize { left: true, top: true };
        let mut s = Session::new(mode, (100, 100), START, Bounds::DEFAULT, 0);
        let asked = s.at((380, 300)).expect("empezó"); // 120 × 100: menos que su mínimo
        // La app no baja de 300 × 250 y queda pegada arriba a la izquierda.
        let got = Rect::new(asked.left, asked.top, asked.left + 300, asked.top + 250);
        s.learn(asked, got);
        let r = s.at((380, 300)).expect("sigue");
        assert_eq!((r.width(), r.height()), (300, 250));
        assert_eq!((r.right, r.bottom), (START.right, START.bottom), "la esquina opuesta fija");
    }

    #[test]
    fn the_corner_is_the_quadrant_of_the_cursor() {
        assert_eq!(corner_of(START, (120, 120)), Mode::Resize { left: true, top: true });
        assert_eq!(corner_of(START, (480, 390)), Mode::Resize { left: false, top: false });
    }

    #[test]
    fn a_maximized_window_comes_back_under_the_cursor() {
        let maxed = Rect::new(0, 0, 1920, 1040);
        let r = unmaximized(maxed, (960, 600), (1440, 10));
        assert_eq!((r.width(), r.height()), (960, 600));
        // El cursor queda al 75 % del ancho, como estaba.
        assert_eq!(1440 - r.left, 720);
        assert_eq!(r.top, 0);
    }
}
