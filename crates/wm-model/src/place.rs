//! Capa de ventana: adónde va una ventana (plan-core.md §3.1).
//!
//! Funciones puras sobre rectángulos: qué ubicación pide cada tecla, en qué
//! monitor está una ventana y qué rectángulo exterior hay que pedirle a
//! Windows para que el **marco visible** quede donde queremos (las ventanas
//! de Windows 10/11 tienen bordes invisibles de unos 7 px, architecture.md §8).

use crate::keyboard::{HELD_A, HELD_D, HELD_S, HELD_W};
use crate::{Placement, Rect};

/// Una dirección de la capa de ventana (W/A/S/D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Up,
    Left,
    Down,
    Right,
}

impl Dir {
    pub fn from_letter(letter: u8) -> Option<Self> {
        match letter {
            b'W' => Some(Dir::Up),
            b'A' => Some(Dir::Left),
            b'S' => Some(Dir::Down),
            b'D' => Some(Dir::Right),
            _ => None,
        }
    }

    fn bit(self) -> u8 {
        match self {
            Dir::Up => HELD_W,
            Dir::Left => HELD_A,
            Dir::Down => HELD_S,
            Dir::Right => HELD_D,
        }
    }

    pub fn vertical(self) -> bool {
        matches!(self, Dir::Up | Dir::Down)
    }

    /// ¿Forman una esquina (una vertical y una horizontal)?
    pub fn perpendicular(self, other: Dir) -> bool {
        self.vertical() != other.vertical()
    }

    /// La dirección perpendicular mantenida en `held`, si hay una sola.
    pub fn held_perpendicular(self, held: u8) -> Option<Dir> {
        let candidates: Vec<Dir> = [Dir::Up, Dir::Left, Dir::Down, Dir::Right]
            .into_iter()
            .filter(|d| d.perpendicular(self) && held & d.bit() != 0)
            .collect();
        match candidates.as_slice() {
            [one] => Some(*one),
            _ => None, // ninguna, o las dos opuestas a la vez: no es una esquina clara
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::Up => "arriba",
            Dir::Left => "izquierda",
            Dir::Down => "abajo",
            Dir::Right => "derecha",
        }
    }
}

/// Ubicación para una dirección, o la esquina si viene con su perpendicular.
pub fn placement_for(dir: Dir, perp: Option<Dir>) -> Placement {
    let (v, h) = match (dir.vertical(), perp) {
        (true, Some(p)) => (Some(dir), Some(p)),
        (false, Some(p)) => (Some(p), Some(dir)),
        (true, None) => (Some(dir), None),
        (false, None) => (None, Some(dir)),
    };
    match (v, h) {
        (Some(Dir::Up), Some(Dir::Left)) => Placement::TopLeft,
        (Some(Dir::Up), Some(Dir::Right)) => Placement::TopRight,
        (Some(Dir::Down), Some(Dir::Left)) => Placement::BottomLeft,
        (Some(Dir::Down), Some(Dir::Right)) => Placement::BottomRight,
        (Some(Dir::Up), None) => Placement::TopHalf,
        (Some(Dir::Down), None) => Placement::BottomHalf,
        (None, Some(Dir::Left)) => Placement::LeftHalf,
        (None, Some(Dir::Right)) => Placement::RightHalf,
        _ => Placement::Full, // inalcanzable: `dir` siempre está en un eje
    }
}

/// Nombre de una ubicación, para el log.
pub fn placement_name(p: Placement) -> &'static str {
    match p {
        Placement::Full => "toda la pantalla",
        Placement::LeftHalf => "mitad izquierda",
        Placement::RightHalf => "mitad derecha",
        Placement::TopHalf => "mitad de arriba",
        Placement::BottomHalf => "mitad de abajo",
        Placement::TopLeft => "esquina de arriba a la izquierda",
        Placement::TopRight => "esquina de arriba a la derecha",
        Placement::BottomLeft => "esquina de abajo a la izquierda",
        Placement::BottomRight => "esquina de abajo a la derecha",
    }
}

/// Un monitor, como se ve en una observación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorObs {
    pub bounds: Rect,
    /// Área de trabajo (sin la barra de tareas).
    pub work: Rect,
    pub dpi: u32,
}

fn overlap(a: Rect, b: Rect) -> i64 {
    let w = i64::from((a.right.min(b.right) - a.left.max(b.left)).max(0));
    let h = i64::from((a.bottom.min(b.bottom) - a.top.max(b.top)).max(0));
    w * h
}

/// Monitor donde está la mayor parte de `r`; si no toca ninguno, el de
/// centro más cercano.
pub fn monitor_of(monitors: &[MonitorObs], r: Rect) -> Option<usize> {
    let best = (0..monitors.len()).max_by_key(|&i| overlap(monitors[i].bounds, r))?;
    if overlap(monitors[best].bounds, r) > 0 {
        return Some(best);
    }
    let center = |q: Rect| (i64::from(q.left + q.right) / 2, i64::from(q.top + q.bottom) / 2);
    let (cx, cy) = center(r);
    (0..monitors.len()).min_by_key(|&i| {
        let (mx, my) = center(monitors[i].bounds);
        (mx - cx).pow(2) + (my - cy).pow(2)
    })
}

/// Monitor vecino de `from` en la dirección `dir` (para empujar,
/// keyboard-design.md §14.4): el más cercano entre los que están del todo de
/// ese lado; entre dos igual de cerca, el que más se solapa en el otro eje.
/// Nunca devuelve `from`.
pub fn neighbor(monitors: &[MonitorObs], from: usize, dir: Dir) -> Option<usize> {
    let a = monitors.get(from)?.bounds;
    // (distancia entre bordes, solape en el otro eje) de cada candidato.
    let key = |b: Rect| -> Option<(i64, i64)> {
        let (gap, overlap) = match dir {
            Dir::Left => (a.left - b.right, a.bottom.min(b.bottom) - a.top.max(b.top)),
            Dir::Right => (b.left - a.right, a.bottom.min(b.bottom) - a.top.max(b.top)),
            Dir::Up => (a.top - b.bottom, a.right.min(b.right) - a.left.max(b.left)),
            Dir::Down => (b.top - a.bottom, a.right.min(b.right) - a.left.max(b.left)),
        };
        (gap >= 0).then_some((i64::from(gap), i64::from(overlap.max(0))))
    };
    (0..monitors.len())
        .filter(|&i| i != from)
        .filter_map(|i| key(monitors[i].bounds).map(|k| (i, k)))
        // Primero los que se solapan en el otro eje (los que "están al lado").
        .min_by_key(|&(_, (gap, overlap))| (overlap == 0, gap, -overlap))
        .map(|(i, _)| i)
}

/// La ubicación espejada al cruzar a un monitor vecino en `dir`: empujar a
/// la izquierda desde la mitad izquierda entra por la mitad derecha, y una
/// esquina conserva la otra componente (arriba sigue arriba).
pub fn mirror(p: Placement, dir: Dir) -> Placement {
    use Placement as P;
    if dir.vertical() {
        match p {
            P::TopHalf => P::BottomHalf,
            P::BottomHalf => P::TopHalf,
            P::TopLeft => P::BottomLeft,
            P::BottomLeft => P::TopLeft,
            P::TopRight => P::BottomRight,
            P::BottomRight => P::TopRight,
            other => other,
        }
    } else {
        match p {
            P::LeftHalf => P::RightHalf,
            P::RightHalf => P::LeftHalf,
            P::TopLeft => P::TopRight,
            P::TopRight => P::TopLeft,
            P::BottomLeft => P::BottomRight,
            P::BottomRight => P::BottomLeft,
            other => other,
        }
    }
}

/// Mayor diferencia por borde entre dos rectángulos.
pub fn edge_error(a: Rect, b: Rect) -> i32 {
    (a.left - b.left)
        .abs()
        .max((a.top - b.top).abs())
        .max((a.right - b.right).abs())
        .max((a.bottom - b.bottom).abs())
}

/// Rectángulo exterior para que el marco visible quede en `target`, con los
/// márgenes actuales (`outer` − `frame`) escalados de `from_dpi` a `to_dpi`
/// (los bordes invisibles crecen con la escala).
pub fn outer_for(frame: Rect, outer: Rect, target: Rect, from_dpi: u32, to_dpi: u32) -> Rect {
    let s = |m: i32| (i64::from(m) * i64::from(to_dpi.max(1)) / i64::from(from_dpi.max(1))) as i32;
    Rect {
        left: target.left - s(frame.left - outer.left),
        top: target.top - s(frame.top - outer.top),
        right: target.right + s(outer.right - frame.right),
        bottom: target.bottom + s(outer.bottom - frame.bottom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place as place_in;

    fn mon(left: i32, width: i32) -> MonitorObs {
        let bounds = Rect::new(left, 0, left + width, 1080);
        MonitorObs { bounds, work: Rect::new(left, 0, left + width, 1040), dpi: 96 }
    }

    #[test]
    fn letters_map_to_directions() {
        assert_eq!(Dir::from_letter(b'W'), Some(Dir::Up));
        assert_eq!(Dir::from_letter(b'D'), Some(Dir::Right));
        assert_eq!(Dir::from_letter(b'F'), None);
    }

    #[test]
    fn halves_and_corners() {
        assert_eq!(placement_for(Dir::Left, None), Placement::LeftHalf);
        assert_eq!(placement_for(Dir::Down, None), Placement::BottomHalf);
        assert_eq!(placement_for(Dir::Up, Some(Dir::Left)), Placement::TopLeft);
        assert_eq!(
            placement_for(Dir::Left, Some(Dir::Up)),
            Placement::TopLeft,
            "el orden da igual"
        );
        assert_eq!(placement_for(Dir::Right, Some(Dir::Down)), Placement::BottomRight);
    }

    #[test]
    fn held_perpendicular_needs_exactly_one() {
        assert_eq!(Dir::Left.held_perpendicular(HELD_W | HELD_A), Some(Dir::Up));
        assert_eq!(Dir::Left.held_perpendicular(HELD_A | HELD_D), None, "misma línea");
        assert_eq!(Dir::Left.held_perpendicular(HELD_W | HELD_S), None, "dos opuestas");
        assert_eq!(Dir::Up.held_perpendicular(HELD_W), None);
    }

    #[test]
    fn monitor_is_the_one_with_most_of_the_window() {
        let ms = [mon(0, 1920), mon(1920, 1920)];
        assert_eq!(monitor_of(&ms, Rect::new(1800, 100, 2200, 500)), Some(1));
        assert_eq!(monitor_of(&ms, Rect::new(100, 100, 500, 500)), Some(0));
        // Totalmente afuera (a la derecha de todo): el más cercano.
        assert_eq!(monitor_of(&ms, Rect::new(5000, 100, 5200, 500)), Some(1));
        assert_eq!(monitor_of(&[], Rect::new(0, 0, 1, 1)), None);
    }

    #[test]
    fn outer_rect_keeps_the_invisible_borders() {
        // Marco visible con bordes invisibles de 7 px a los costados y abajo.
        let frame = Rect::new(100, 100, 900, 700);
        let outer = Rect::new(93, 100, 907, 707);
        let target = place_in(mon(0, 1920).work, Placement::LeftHalf);
        let got = outer_for(frame, outer, target, 96, 96);
        assert_eq!(got, Rect::new(-7, 0, 967, 1047));
        // A 150 % los bordes crecen: 7 → 10.
        assert_eq!(outer_for(frame, outer, target, 96, 144).left, -10);
    }

    #[test]
    fn neighbor_is_the_closest_monitor_on_that_side() {
        // Tres en fila: 0 | 1 | 2.
        let ms = [mon(0, 1920), mon(1920, 1920), mon(3840, 1920)];
        assert_eq!(neighbor(&ms, 1, Dir::Left), Some(0));
        assert_eq!(neighbor(&ms, 1, Dir::Right), Some(2));
        assert_eq!(neighbor(&ms, 0, Dir::Right), Some(1), "el más cercano, no el de más allá");
        assert_eq!(neighbor(&ms, 0, Dir::Left), None);
        assert_eq!(neighbor(&ms, 1, Dir::Up), None);
    }

    #[test]
    fn neighbor_prefers_the_one_beside_over_a_diagonal_one() {
        let a = mon(0, 1920);
        // Uno arriba a la derecha (en diagonal) y otro a la derecha, más lejos.
        let diagonal = MonitorObs {
            bounds: Rect::new(1920, -1080, 3840, 0),
            work: Rect::new(1920, -1080, 3840, -40),
            dpi: 96,
        };
        let beside = mon(2000, 1920);
        assert_eq!(neighbor(&[a, diagonal, beside], 0, Dir::Right), Some(2));
        // Solo en diagonal: igual sirve.
        assert_eq!(neighbor(&[a, diagonal], 0, Dir::Right), Some(1));
        assert_eq!(neighbor(&[a, diagonal], 0, Dir::Up), Some(1));
    }

    #[test]
    fn neighbor_never_returns_itself_and_round_trips() {
        let ms = [mon(0, 1920), mon(1920, 2560)];
        for (from, dir, back) in [(0, Dir::Right, Dir::Left), (1, Dir::Left, Dir::Right)] {
            let n = neighbor(&ms, from, dir).expect("hay vecino");
            assert_ne!(n, from);
            assert_eq!(neighbor(&ms, n, back), Some(from), "ida y vuelta");
        }
    }

    #[test]
    fn mirror_enters_by_the_opposite_side_keeping_the_other_component() {
        assert_eq!(mirror(Placement::LeftHalf, Dir::Left), Placement::RightHalf);
        assert_eq!(mirror(Placement::TopLeft, Dir::Left), Placement::TopRight);
        assert_eq!(mirror(Placement::BottomRight, Dir::Down), Placement::TopRight);
        assert_eq!(mirror(Placement::TopHalf, Dir::Up), Placement::BottomHalf);
        for p in [Placement::LeftHalf, Placement::TopLeft, Placement::BottomRight] {
            for d in [Dir::Left, Dir::Up] {
                assert_eq!(mirror(mirror(p, d), d), p, "espejar dos veces es la identidad");
            }
        }
    }

    #[test]
    fn edge_error_is_the_worst_edge() {
        let a = Rect::new(0, 0, 100, 100);
        assert_eq!(edge_error(a, a), 0);
        assert_eq!(edge_error(a, Rect::new(1, 0, 100, 97)), 3);
    }
}
