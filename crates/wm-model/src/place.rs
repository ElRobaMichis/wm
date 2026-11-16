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

    /// Su bit en la máscara de direcciones mantenidas (`keyboard::HELD_*`).
    pub fn bit(self) -> u8 {
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

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Left => Dir::Right,
            Dir::Down => Dir::Up,
            Dir::Right => Dir::Left,
        }
    }

    /// "a la izquierda", "arriba"… (para "no hay otro monitor …").
    pub fn toward(self) -> &'static str {
        match self {
            Dir::Up => "arriba",
            Dir::Left => "a la izquierda",
            Dir::Down => "abajo",
            Dir::Right => "a la derecha",
        }
    }

    /// "de la izquierda", "de arriba"… (para "monitor …").
    pub fn of_side(self) -> &'static str {
        match self {
            Dir::Up => "de arriba",
            Dir::Left => "de la izquierda",
            Dir::Down => "de abajo",
            Dir::Right => "de la derecha",
        }
    }

    /// La letra de la tecla (para el log de los acordes).
    pub fn letter(self) -> char {
        match self {
            Dir::Up => 'W',
            Dir::Left => 'A',
            Dir::Down => 'S',
            Dir::Right => 'D',
        }
    }

    /// Las cuatro, en el orden de las teclas W, A, S, D.
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Left, Dir::Down, Dir::Right];

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

// ---------------------------------------------------------------------------
// La capa de ventana como estados (plan-core.md §3.1, versión 2)
// ---------------------------------------------------------------------------
//
// Una ubicación es un costado {izquierda, todo, derecha} y una altura
// {arriba, todo, abajo}; `Full` (todo, todo) también representa una ventana
// libre o maximizada. Las teclas actúan según dónde está la ventana, no según
// la tecla anterior ni el tiempo:
//
// - **R1, una tecla sola** ([`step`], esquema [`Scheme::Direct`], el
//   vigente): A/D/W/S llevan **siempre** a esa mitad (de todo el alto o de
//   todo el ancho), estés donde estés. Si la ventana ya está en esa mitad,
//   pasa al monitor vecino entrando por la mitad opuesta; sin vecino, nada.
//   "Una tecla, una mitad; repetirla, al otro monitor."
// - **Esquinas = dos teclas juntas** (R2). Con teclas sueltas no hay
//   esquinas: la misma tecla hace siempre lo mismo (decisión del usuario,
//   15/11/2026, después de probar la escalera; plan-core.md §3.1).
// - **Escalera** ([`Scheme::Ladder`], opción futura del menú de ajustes):
//   cada tecla mueve un paso en su eje (izquierda ↔ todo ↔ derecha, arriba ↔
//   todo ↔ abajo) conservando el otro, así las esquinas salen en secuencia.
// - **R2, teclas juntas** ([`chord`]): nombran los bordes que toca la
//   ventana, calculado desde donde estaba antes de la primera tecla, así que
//   el orden en que caen los dedos da igual. Dos opuestas se anulan.

/// Costado y altura de una ubicación (`None` = todo el ancho / alto).
pub fn split(p: Placement) -> (Option<Dir>, Option<Dir>) {
    use Placement as P;
    match p {
        P::Full => (None, None),
        P::LeftHalf => (Some(Dir::Left), None),
        P::RightHalf => (Some(Dir::Right), None),
        P::TopHalf => (None, Some(Dir::Up)),
        P::BottomHalf => (None, Some(Dir::Down)),
        P::TopLeft => (Some(Dir::Left), Some(Dir::Up)),
        P::TopRight => (Some(Dir::Right), Some(Dir::Up)),
        P::BottomLeft => (Some(Dir::Left), Some(Dir::Down)),
        P::BottomRight => (Some(Dir::Right), Some(Dir::Down)),
    }
}

/// La ubicación con ese costado y esa altura.
pub fn join(h: Option<Dir>, v: Option<Dir>) -> Placement {
    use Placement as P;
    match (h, v) {
        (Some(Dir::Left), Some(Dir::Up)) => P::TopLeft,
        (Some(Dir::Right), Some(Dir::Up)) => P::TopRight,
        (Some(Dir::Left), Some(Dir::Down)) => P::BottomLeft,
        (Some(Dir::Right), Some(Dir::Down)) => P::BottomRight,
        (Some(Dir::Left), None) => P::LeftHalf,
        (Some(Dir::Right), None) => P::RightHalf,
        (None, Some(Dir::Up)) => P::TopHalf,
        (None, Some(Dir::Down)) => P::BottomHalf,
        _ => P::Full,
    }
}

/// Qué hace una tecla sola.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// A esta ubicación del mismo monitor.
    Stay(Placement),
    /// Al monitor vecino en `dir`, a la ubicación `enter`.
    Push { dir: Dir, enter: Placement },
}

/// Cómo se mueven las ventanas con una tecla sola.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheme {
    /// Una tecla, una mitad (la vigente): la misma tecla hace siempre lo
    /// mismo; las esquinas, con dos teclas juntas.
    #[default]
    Direct,
    /// Escalera en los dos ejes: un paso por tecla, conservando el otro eje;
    /// las esquinas salen en secuencia. Pensada como opción del menú de
    /// ajustes para quien la prefiera (plan-core.md §3.1); todavía no se
    /// puede elegir.
    Ladder,
}

/// R1: una tecla sola desde `slot` (`Full` = libre o maximizada), con el
/// esquema vigente ([`Scheme::Direct`]).
pub fn step(slot: Placement, key: Dir) -> Move {
    step_with(Scheme::Direct, slot, key)
}

/// R1 con un esquema dado.
pub fn step_with(scheme: Scheme, slot: Placement, key: Dir) -> Move {
    match scheme {
        Scheme::Direct => {
            let half = if key.vertical() { join(None, Some(key)) } else { join(Some(key), None) };
            if slot == half {
                Move::Push { dir: key, enter: mirror(half, key) }
            } else {
                Move::Stay(half)
            }
        }
        Scheme::Ladder => {
            let (h, v) = split(slot);
            let (axis, other) = if key.vertical() { (v, h) } else { (h, v) };
            let join_axis =
                |c: Option<Dir>| if key.vertical() { join(other, c) } else { join(c, other) };
            if axis == Some(key) {
                Move::Push { dir: key, enter: join_axis(Some(key.opposite())) }
            } else if axis == Some(key.opposite()) {
                Move::Stay(join_axis(None)) // un paso hacia "todo"
            } else {
                Move::Stay(join_axis(Some(key)))
            }
        }
    }
}

/// R2: la ubicación que nombran las teclas `dirs` (máscara de `HELD_*`)
/// apretadas juntas. Dos opuestas se anulan; sin nada que nombrar, `None`.
pub fn chord(dirs: u8) -> Option<Placement> {
    let pick = |a: Dir, b: Dir| match (dirs & a.bit() != 0, dirs & b.bit() != 0) {
        (true, false) => Some(a),
        (false, true) => Some(b),
        _ => None,
    };
    let (h, v) = (pick(Dir::Left, Dir::Right), pick(Dir::Up, Dir::Down));
    (h.is_some() || v.is_some()).then(|| join(h, v))
}

/// Si un acorde nombra la ubicación donde ya estaba la ventana, empuja: estas
/// son las direcciones a probar, en orden (primero el costado, después la
/// altura).
pub fn chord_pushes(target: Placement) -> Vec<Dir> {
    let (h, v) = split(target);
    h.into_iter().chain(v).collect()
}

/// El acorde escrito con sus teclas en orden fijo ("W + A").
pub fn chord_name(dirs: u8) -> String {
    Dir::ALL
        .into_iter()
        .filter(|d| dirs & d.bit() != 0)
        .map(|d| d.letter().to_string())
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Para una ventana que no tomó el tamaño pedido (tamaño mínimo o fijo): el
/// tamaño que logró (`w` × `h`), **pegado al borde exterior** del lugar
/// pedido. Por eje: si el lugar toca solo el borde derecho (o de abajo) del
/// área, se alinea a ese borde; si toca los dos, o ninguno, se centra; si
/// toca solo el izquierdo (o de arriba), se alinea a ese. Así una mitad
/// derecha que no puede achicarse no se sale de la pantalla por la derecha.
pub fn anchor(target: Rect, work: Rect, w: i32, h: i32) -> Rect {
    let axis = |lo: i32, hi: i32, wlo: i32, whi: i32, size: i32| -> i32 {
        match (lo == wlo, hi == whi) {
            (false, true) => hi - size,
            (true, false) => lo,
            _ => lo + (hi - lo - size) / 2,
        }
    };
    let left = axis(target.left, target.right, work.left, work.right, w);
    let top = axis(target.top, target.bottom, work.top, work.bottom, h);
    Rect::new(left, top, left + w, top + h)
}

/// Un rectángulo de `w` × `h` centrado en `work` (achicado si no entra).
pub fn centered(work: Rect, w: i32, h: i32) -> Rect {
    let (w, h) = (w.min(work.width()), h.min(work.height()));
    let left = work.left + (work.width() - w) / 2;
    let top = work.top + (work.height() - h) / 2;
    Rect::new(left, top, left + w, top + h)
}

/// `pct` % del ancho y del alto de `work`, centrado.
pub fn centered_percent(work: Rect, pct: i32) -> Rect {
    centered(work, work.width() * pct / 100, work.height() * pct / 100)
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

/// Las ubicaciones que reconocemos (todas menos `Full`).
const SLOTS: [Placement; 8] = [
    Placement::LeftHalf,
    Placement::RightHalf,
    Placement::TopHalf,
    Placement::BottomHalf,
    Placement::TopLeft,
    Placement::TopRight,
    Placement::BottomLeft,
    Placement::BottomRight,
];

/// En qué mitad o esquina del área `work` está el marco `frame` (±1 px).
pub fn slot_of(work: Rect, frame: Rect) -> Option<Placement> {
    SLOTS.into_iter().find(|&p| edge_error(crate::place(work, p), frame) <= 1)
}

/// ¿La ubicación está pegada al borde del lado `dir`? Solo desde ahí,
/// seguir en esa dirección empuja al monitor vecino ("A siempre lleva a la
/// izquierda": desde la mitad derecha, primero va a la mitad izquierda).
pub fn on_side(p: Placement, dir: Dir) -> bool {
    use Placement as P;
    match dir {
        Dir::Left => matches!(p, P::LeftHalf | P::TopLeft | P::BottomLeft),
        Dir::Right => matches!(p, P::RightHalf | P::TopRight | P::BottomRight),
        Dir::Up => matches!(p, P::TopHalf | P::TopLeft | P::TopRight),
        Dir::Down => matches!(p, P::BottomHalf | P::BottomLeft | P::BottomRight),
    }
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
    fn slots_are_recognized_and_know_their_side() {
        let work = mon(0, 1920).work;
        for p in SLOTS {
            assert_eq!(slot_of(work, place_in(work, p)), Some(p));
        }
        assert_eq!(slot_of(work, Rect::new(10, 10, 500, 500)), None);
        assert!(on_side(Placement::TopLeft, Dir::Left));
        assert!(on_side(Placement::TopLeft, Dir::Up));
        assert!(!on_side(Placement::RightHalf, Dir::Left));
        assert!(
            !on_side(Placement::TopHalf, Dir::Left),
            "la de arriba no está pegada a un costado"
        );
    }

    use Placement as P;

    /// Todas las ubicaciones: libre (Full) y las ocho.
    const ALL: [Placement; 9] = [
        P::Full,
        P::LeftHalf,
        P::RightHalf,
        P::TopHalf,
        P::BottomHalf,
        P::TopLeft,
        P::TopRight,
        P::BottomLeft,
        P::BottomRight,
    ];

    #[test]
    fn split_and_join_are_inverse() {
        for p in ALL {
            let (h, v) = split(p);
            assert_eq!(join(h, v), p);
        }
    }

    #[test]
    fn direct_one_key_one_half_and_repeat_pushes() {
        let halves = [
            (Dir::Left, P::LeftHalf),
            (Dir::Right, P::RightHalf),
            (Dir::Up, P::TopHalf),
            (Dir::Down, P::BottomHalf),
        ];
        for from in ALL {
            for (key, half) in halves {
                let want = if from == half {
                    Move::Push { dir: key, enter: mirror(half, key) }
                } else {
                    Move::Stay(half)
                };
                assert_eq!(step(from, key), want, "{from:?} + {key:?}");
            }
        }
    }

    #[test]
    fn direct_the_same_key_always_does_the_same() {
        // Salvo el empuje (ya estaba ahí), una tecla da siempre la misma mitad.
        for key in Dir::ALL {
            let results: std::collections::HashSet<Placement> = ALL
                .into_iter()
                .filter_map(|p| match step(p, key) {
                    Move::Stay(q) => Some(q),
                    Move::Push { .. } => None,
                })
                .collect();
            assert_eq!(results.len(), 1, "{key:?}: {results:?}");
        }
    }

    #[test]
    fn every_push_comes_back_with_the_opposite_key() {
        for scheme in [Scheme::Direct, Scheme::Ladder] {
            for p in ALL {
                for key in Dir::ALL {
                    if let Move::Push { dir, enter } = step_with(scheme, p, key) {
                        assert_eq!(
                            step_with(scheme, enter, dir.opposite()),
                            Move::Push { dir: dir.opposite(), enter: p },
                            "{scheme:?}: {p:?} + {key:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn ladder_steps_one_axis_at_a_time() {
        use Dir::{Down as S, Left as A, Right as D, Up as W};
        let ladder = |p, k| step_with(Scheme::Ladder, p, k);
        assert_eq!(ladder(P::BottomLeft, W), Move::Stay(P::LeftHalf));
        assert_eq!(ladder(P::LeftHalf, W), Move::Stay(P::TopLeft));
        assert_eq!(ladder(P::TopHalf, A), Move::Stay(P::TopLeft));
        assert_eq!(ladder(P::LeftHalf, D), Move::Stay(P::Full), "pasa por todo el ancho");
        assert_eq!(ladder(P::Full, S), Move::Stay(P::BottomHalf));
        assert_eq!(ladder(P::TopLeft, A), Move::Push { dir: A, enter: P::TopRight });
        // Desde cualquier lugar, la escalera llega a todo con teclas solas.
        for start in ALL {
            let mut seen = std::collections::HashSet::from([start]);
            let mut frontier = vec![start];
            while let Some(p) = frontier.pop() {
                for key in [A, D, W, S] {
                    if let Move::Stay(q) = ladder(p, key)
                        && seen.insert(q)
                    {
                        frontier.push(q);
                    }
                }
            }
            assert_eq!(seen.len(), 9, "desde {start:?}");
        }
    }

    #[test]
    fn chords_name_borders_and_opposites_cancel() {
        assert_eq!(chord(HELD_W | HELD_A), Some(P::TopLeft));
        assert_eq!(chord(HELD_S | HELD_D), Some(P::BottomRight));
        assert_eq!(chord(HELD_A | HELD_D), None, "opuestas se anulan");
        assert_eq!(chord(HELD_W | HELD_A | HELD_D), Some(P::TopHalf));
        assert_eq!(chord(HELD_W | HELD_A | HELD_S | HELD_D), None);
        assert_eq!(chord_pushes(P::TopLeft), vec![Dir::Left, Dir::Up]);
        assert_eq!(chord_name(HELD_A | HELD_W), "W + A");
    }

    #[test]
    fn anchoring_sticks_to_the_outer_edge_of_the_slot() {
        let work = Rect::new(0, 0, 1920, 1040);
        let right = place_in(work, P::RightHalf);
        // Más ancha que media pantalla: pegada a la derecha, no afuera.
        assert_eq!(anchor(right, work, 1200, 1040), Rect::new(720, 0, 1920, 1040));
        let left = place_in(work, P::LeftHalf);
        assert_eq!(anchor(left, work, 1200, 1040), Rect::new(0, 0, 1200, 1040));
        // Fija y más chica, en la esquina de abajo a la derecha: en la esquina.
        let br = place_in(work, P::BottomRight);
        assert_eq!(anchor(br, work, 800, 400), Rect::new(1120, 640, 1920, 1040));
        // Centrada (no toca bordes): sigue centrada.
        let c = centered(work, 800, 600);
        assert_eq!(anchor(c, work, 900, 700), centered(work, 900, 700));
    }

    #[test]
    fn centering_keeps_the_size_and_fits_in_the_area() {
        let work = Rect::new(0, 0, 1920, 1040);
        assert_eq!(centered(work, 800, 600), Rect::new(560, 220, 1360, 820));
        assert_eq!(centered(work, 4000, 600).width(), 1920, "más grande que el área: se achica");
        assert_eq!(centered_percent(work, 60), Rect::new(384, 208, 1536, 832));
    }

    #[test]
    fn edge_error_is_the_worst_edge() {
        let a = Rect::new(0, 0, 100, 100);
        assert_eq!(edge_error(a, a), 0);
        assert_eq!(edge_error(a, Rect::new(1, 0, 100, 97)), 3);
    }
}
