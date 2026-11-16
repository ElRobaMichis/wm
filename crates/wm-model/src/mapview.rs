//! Qué dice el mapa de teclado (plan-core.md §5.3): puro, para poder
//! probarlo sin dibujar. El hilo `ui` lo dibuja tal cual.
//!
//! Hito 2a: las tres filas de letras. En la capa de apps, cada letra con la
//! app asignada en `apps.txt` (las libres, solo la letra, tenue); en la capa
//! de ventana, lo que hace cada tecla. 2b agrega el modo automático por
//! inicial (apps abiertas), la tecla recién usada en `accent`, la fila de
//! escritorios y el pie.

use crate::apps::AppBinding;
use crate::keyboard::Layer;

/// Las filas del teclado, como en el mapa de la demo (QWERTY).
pub const ROWS: [&str; 3] = ["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"];

/// Largo máximo de una etiqueta (cabe en la tecla con Geist de 14 px).
const LABEL_MAX: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Tiene algo: letra clara, etiqueta visible.
    Normal,
    /// Libre: solo la letra, tenue.
    Dim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCell {
    pub letter: char,
    pub label: String,
    pub tone: Tone,
}

/// Una capa del mapa: una celda por letra de [`ROWS`], en ese orden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapView {
    pub layer: Layer,
    pub cells: Vec<KeyCell>,
}

/// Qué hace cada tecla de la capa de ventana (`Caps` + Alt).
const WINDOW_KEYS: [(char, &str); 9] = [
    ('W', "arriba"),
    ('A', "izquierda"),
    ('S', "abajo"),
    ('D', "derecha"),
    ('F', "maximizar"),
    ('C', "centrar"),
    ('Z', "deshacer"),
    ('G', "agrandar"),
    ('V', "achicar"),
];

fn shorten(s: &str) -> String {
    if s.chars().count() <= LABEL_MAX {
        return s.to_owned();
    }
    let cut: String = s.chars().take(LABEL_MAX - 1).collect();
    format!("{cut}…")
}

/// La capa `layer` del mapa con estas asignaciones.
pub fn map_view(layer: Layer, bindings: &[AppBinding]) -> MapView {
    let label_of = |letter: char| -> Option<String> {
        match layer {
            Layer::Window => {
                WINDOW_KEYS.iter().find(|(l, _)| *l == letter).map(|(_, s)| (*s).to_owned())
            }
            _ => bindings
                .iter()
                .find(|b| b.letter.eq_ignore_ascii_case(&(letter as u8)))
                .map(|b| shorten(&b.app)),
        }
    };
    let cells = ROWS
        .iter()
        .flat_map(|row| row.chars())
        .map(|letter| match label_of(letter) {
            Some(label) => KeyCell { letter, label, tone: Tone::Normal },
            None => KeyCell { letter, label: String::new(), tone: Tone::Dim },
        })
        .collect();
    MapView { layer, cells }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::parse_bindings;

    fn cell(v: &MapView, letter: char) -> &KeyCell {
        v.cells.iter().find(|c| c.letter == letter).expect("letra del mapa")
    }

    #[test]
    fn the_apps_layer_shows_the_bound_apps_and_dims_the_rest() {
        let (b, _) = parse_bindings("b = brave\ns = steamwebhelper\n");
        let v = map_view(Layer::Apps, &b);
        assert_eq!(v.cells.len(), 26);
        assert_eq!(cell(&v, 'B').label, "brave");
        assert_eq!(cell(&v, 'B').tone, Tone::Normal);
        assert_eq!(cell(&v, 'S').label, "steamwebhe…", "recortada para que entre");
        assert_eq!(cell(&v, 'Q').tone, Tone::Dim);
        assert!(cell(&v, 'Q').label.is_empty());
    }

    #[test]
    fn the_window_layer_says_what_each_key_does() {
        let v = map_view(Layer::Window, &[]);
        assert_eq!(cell(&v, 'A').label, "izquierda");
        assert_eq!(cell(&v, 'C').label, "centrar");
        assert_eq!(cell(&v, 'G').label, "agrandar");
        assert_eq!(cell(&v, 'Q').tone, Tone::Dim);
        let named = v.cells.iter().filter(|c| c.tone == Tone::Normal).count();
        assert_eq!(named, 9);
    }

    #[test]
    fn labels_fit_in_a_key() {
        let (b, _) = parse_bindings("n = una-app-con-nombre-largo\n");
        for layer in [Layer::Apps, Layer::Window] {
            for c in map_view(layer, &b).cells {
                assert!(c.label.chars().count() <= LABEL_MAX, "{c:?}");
            }
        }
    }
}
