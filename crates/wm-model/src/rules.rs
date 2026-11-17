//! v1.1 (plan-v1.1.md): reglas por app. Hoy, "al abrir": dónde va la
//! primera ventana principal de la app cuando se abre.

use crate::Placement;

/// Dónde va una app al abrirse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRule {
    /// Una mitad o una esquina.
    Place(Placement),
    Maximized,
    /// Centrada, a este porcentaje del área (60 u 80).
    Centered(u8),
}

const NAMES: [(Placement, &str); 8] = [
    (Placement::LeftHalf, "izquierda"),
    (Placement::RightHalf, "derecha"),
    (Placement::TopHalf, "arriba"),
    (Placement::BottomHalf, "abajo"),
    (Placement::TopLeft, "arriba-izquierda"),
    (Placement::TopRight, "arriba-derecha"),
    (Placement::BottomLeft, "abajo-izquierda"),
    (Placement::BottomRight, "abajo-derecha"),
];

impl OpenRule {
    /// "derecha", "maximizada", "centrada 60"…
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim().to_lowercase();
        if t == "maximizada" {
            return Some(Self::Maximized);
        }
        if let Some(rest) = t.strip_prefix("centrada") {
            let pct = match rest.trim().trim_end_matches('%').trim() {
                "" => 80,
                n => n.parse::<u8>().ok().filter(|p| (30..=100).contains(p))?,
            };
            return Some(Self::Centered(pct));
        }
        NAMES.iter().find(|(_, n)| *n == t).map(|(p, _)| Self::Place(*p))
    }

    pub fn format(self) -> String {
        match self {
            Self::Maximized => "maximizada".into(),
            Self::Centered(80) => "centrada".into(),
            Self::Centered(p) => format!("centrada {p}"),
            Self::Place(p) => NAMES
                .iter()
                .find(|(q, _)| *q == p)
                .map_or_else(|| "maximizada".into(), |(_, n)| (*n).to_owned()),
        }
    }

    /// Nombre corto para la lista de letras de los ajustes.
    pub fn short(self) -> &'static str {
        match self {
            Self::Maximized => "máx.",
            Self::Centered(_) => "centro",
            Self::Place(Placement::LeftHalf) => "izq.",
            Self::Place(Placement::RightHalf) => "der.",
            Self::Place(Placement::TopHalf) => "arriba",
            Self::Place(Placement::BottomHalf) => "abajo",
            Self::Place(_) => "esquina",
        }
    }
}

impl OpenRule {
    /// Para los avisos: "en la mitad derecha", "maximizada", "centrada al
    /// 80 %"…
    pub fn describe(self) -> String {
        match self {
            Self::Maximized => "maximizada".into(),
            Self::Centered(p) => format!("centrada al {p} %"),
            Self::Place(p) => format!("en la {}", crate::place::placement_name(p)),
        }
    }
}

/// La regla que describe una ventana acomodada (plan-v1.2.md §4, "abrir
/// siempre así"): maximizada de verdad, una mitad o esquina (`placement`,
/// como la ve `World::locate`), o centrada. Una centrada con su propio
/// tamaño (un toque de C) no tiene el mismo porcentaje de ancho y de alto:
/// cuenta el lado más grande, de 30 a 100 % del área de trabajo. Libre:
/// `None`.
pub fn rule_like(
    frame: crate::Rect,
    maximized: bool,
    placement: Placement,
    work: crate::Rect,
) -> Option<OpenRule> {
    if maximized {
        return Some(OpenRule::Maximized);
    }
    if placement != Placement::Full {
        return Some(OpenRule::Place(placement));
    }
    if work.width() <= 0 || work.height() <= 0 {
        return None;
    }
    let tolerance = (work.width() / 100).max(2);
    let dx = (frame.left + frame.right) - (work.left + work.right);
    let dy = (frame.top + frame.bottom) - (work.top + work.bottom);
    if dx.abs() > 2 * tolerance || dy.abs() > 2 * tolerance {
        return None;
    }
    let pct = |a: i32, b: i32| (a * 100 + b / 2) / b;
    let pct = pct(frame.width(), work.width()).max(pct(frame.height(), work.height()));
    (30..=100).contains(&pct).then_some(OpenRule::Centered(pct as u8))
}

/// El campo de `apps.txt`: "al abrir: derecha".
pub const OPEN_KEY: &str = "al abrir:";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_round_trip() {
        for t in ["izquierda", "derecha", "abajo-derecha", "maximizada", "centrada", "centrada 60"]
        {
            let r = OpenRule::parse(t).unwrap_or_else(|| panic!("{t}"));
            assert_eq!(r.format(), t);
        }
        assert_eq!(OpenRule::parse("Centrada 80%"), Some(OpenRule::Centered(80)));
        assert_eq!(OpenRule::parse("centrada 5"), None);
        assert_eq!(OpenRule::parse("al costado"), None);
    }

    #[test]
    fn a_window_tells_its_rule() {
        let work = crate::Rect::new(0, 0, 1920, 1040);
        let half = crate::place(work, Placement::RightHalf);
        assert_eq!(
            rule_like(half, false, Placement::RightHalf, work),
            Some(OpenRule::Place(Placement::RightHalf))
        );
        assert_eq!(rule_like(work, true, Placement::Full, work), Some(OpenRule::Maximized));
        let c60 = crate::place::centered_percent(work, 60);
        assert_eq!(rule_like(c60, false, Placement::Full, work), Some(OpenRule::Centered(60)));
        let off = crate::Rect::new(c60.left + 40, c60.top, c60.right + 40, c60.bottom);
        assert_eq!(rule_like(off, false, Placement::Full, work), None, "corrida: libre");
        let own = crate::place::centered(work, 800, 600);
        assert_eq!(
            rule_like(own, false, Placement::Full, work),
            Some(OpenRule::Centered(58)),
            "un toque de C"
        );
        let small = crate::place::centered_percent(work, 20);
        assert_eq!(rule_like(small, false, Placement::Full, work), None, "muy chica");
        assert_eq!(OpenRule::Place(Placement::RightHalf).describe(), "en la mitad derecha");
        assert_eq!(OpenRule::Centered(80).describe(), "centrada al 80 %");
    }
}
