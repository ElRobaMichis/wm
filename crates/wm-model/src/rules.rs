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
}
