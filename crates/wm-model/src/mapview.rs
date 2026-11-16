//! Qué dice el mapa de teclado (plan-core.md §5.3): puro, para poder
//! probarlo sin dibujar. El hilo `ui` lo dibuja tal cual.
//!
//! - **Capa de apps:** cada letra con su app. Asignada y abierta en `text`;
//!   asignada y cerrada, o automática por inicial (una app abierta sin
//!   asignar, como resuelve `Caps` + letra), en `text-2`; libre, solo la
//!   letra en `text-3`. Con el nombre visible ("VS Code", no "code").
//! - **Capa de ventana:** lo que hace cada tecla con el pulgar.
//! - **Pie:** una línea de ayuda.
//!
//! Falta (hito 4): la fila de escritorios 1–4.

use crate::apps::AppBinding;
use crate::keyboard::Layer;
use crate::settings_demo::{APP_NAMES, APPS};

/// Las filas del teclado (QWERTY).
pub const ROWS: [&str; 3] = ["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"];

/// Largo máximo de una etiqueta (cabe en la tecla con Geist de 14 px; si
/// igual no entra, el dibujo la achica).
const LABEL_MAX: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// La app está abierta (o la tecla hace algo, en la capa de ventana):
    /// `text`.
    Open,
    /// Asignada y cerrada, o automática por inicial: `text-2`.
    Normal,
    /// Libre: solo la letra, `text-3`.
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
    pub footer: String,
}

/// Qué hace cada tecla de la capa de ventana (`Caps` + Alt).
const WINDOW_KEYS: [(char, &str); 9] = [
    // Geist tiene las flechas (U+2190–2193, verificado en su tabla cmap).
    ('W', "↑ arriba"),
    ('A', "← izquierda"),
    ('S', "↓ abajo"),
    ('D', "→ derecha"),
    ('F', "maximizar"),
    ('C', "centrar"),
    ('Z', "deshacer"),
    ('G', "agrandar"),
    ('V', "achicar"),
];

const APPS_FOOTER: &str = "letra = ir a la app  ·  Alt = acomodar la ventana";
const WINDOW_FOOTER: &str = "dos juntas = esquina  ·  repetir = otro monitor  ·  Z = deshacer";

fn shorten(s: &str) -> String {
    if s.chars().count() <= LABEL_MAX {
        return s.to_owned();
    }
    let cut: String = s.chars().take(LABEL_MAX - 1).collect();
    format!("{cut}…")
}

/// Ejecutables cuyo nombre no es el de la app (vistos en la PC del usuario).
const ALIASES: [(&str, &str); 2] = [("steamwebhelper", "Steam"), ("rayo", "Rayo")];

/// Nombre visible de una app (ejecutable sin extensión, en minúsculas).
pub fn display_name(exe: &str) -> String {
    if let Some(i) = APPS.iter().position(|a| *a == exe) {
        return APP_NAMES[i].to_owned();
    }
    // Ejecutables cuyo nombre no es el de la app.
    if let Some((_, name)) = ALIASES.iter().find(|(e, _)| *e == exe) {
        return (*name).to_owned();
    }
    let mut c = exe.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// La capa `layer` del mapa. `open`: las apps abiertas (ejecutables), en
/// orden Z (la primera, arriba), como las ve `Caps` + letra.
pub fn map_view(layer: Layer, bindings: &[AppBinding], open: &[&str]) -> MapView {
    let cell = |letter: char| -> KeyCell {
        let (label, tone) = match layer {
            Layer::Window => match WINDOW_KEYS.iter().find(|(l, _)| *l == letter) {
                Some((_, s)) => ((*s).to_owned(), Tone::Open),
                None => (String::new(), Tone::Dim),
            },
            _ => {
                let lower = letter.to_ascii_lowercase();
                let bound = bindings.iter().find(|b| b.letter == letter as u8);
                // Automática: la primera app abierta con esa inicial que no
                // esté asignada a otra letra (igual que `apps::resolve`).
                let auto = || {
                    open.iter().find(|app| {
                        app.starts_with(lower) && !bindings.iter().any(|b| b.app == **app)
                    })
                };
                match bound {
                    Some(b) => {
                        let tone =
                            if open.contains(&b.app.as_str()) { Tone::Open } else { Tone::Normal };
                        (shorten(&display_name(&b.app)), tone)
                    }
                    None => match auto() {
                        Some(app) => (shorten(&display_name(app)), Tone::Normal),
                        None => (String::new(), Tone::Dim),
                    },
                }
            }
        };
        KeyCell { letter, label, tone }
    };
    let cells = ROWS.iter().flat_map(|row| row.chars()).map(cell).collect();
    let footer = if layer == Layer::Window { WINDOW_FOOTER } else { APPS_FOOTER }.to_owned();
    MapView { layer, cells, footer }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::parse_bindings;

    fn cell(v: &MapView, letter: char) -> &KeyCell {
        v.cells.iter().find(|c| c.letter == letter).expect("letra del mapa")
    }

    #[test]
    fn the_apps_layer_shows_bound_and_open_apps() {
        let (b, _) = parse_bindings("b = brave\nn = notion\n");
        let open = ["brave", "code", "steamwebhelper", "chrome"];
        let v = map_view(Layer::Apps, &b, &open);
        assert_eq!(v.cells.len(), 26);
        assert_eq!(
            cell(&v, 'B'),
            &KeyCell { letter: 'B', label: "Brave".into(), tone: Tone::Open }
        );
        assert_eq!(cell(&v, 'N').tone, Tone::Normal, "asignada y cerrada");
        assert_eq!(cell(&v, 'C').label, "VS Code", "automática: la primera abierta con C");
        assert_eq!(cell(&v, 'C').tone, Tone::Normal);
        assert_eq!(cell(&v, 'S').label, "Steam", "nombre conocido del ejecutable");
        assert_eq!(cell(&v, 'Q').tone, Tone::Dim);
        assert!(cell(&v, 'Q').label.is_empty());
        assert!(v.footer.contains("Alt"));
    }

    #[test]
    fn an_app_bound_to_another_letter_is_not_automatic() {
        let (b, _) = parse_bindings("x = chrome\n");
        let v = map_view(Layer::Apps, &b, &["chrome"]);
        assert_eq!(cell(&v, 'C').tone, Tone::Dim);
        assert_eq!(cell(&v, 'X').label, "Chrome");
    }

    #[test]
    fn the_window_layer_says_what_each_key_does() {
        let v = map_view(Layer::Window, &[], &["brave"]);
        assert_eq!(cell(&v, 'A').label, "← izquierda");
        assert_eq!(cell(&v, 'C').label, "centrar");
        assert_eq!(cell(&v, 'B').tone, Tone::Dim, "las apps no aparecen en la capa de ventana");
        let named = v.cells.iter().filter(|c| c.tone == Tone::Open).count();
        assert_eq!(named, 9);
        assert!(v.footer.contains("esquina"));
    }

    #[test]
    fn unknown_apps_are_capitalized_and_shortened() {
        let v = map_view(Layer::Apps, &[], &["kicadpcbnewer"]);
        assert_eq!(cell(&v, 'K').label, "Kicadpcbne…");
    }

    #[test]
    fn labels_fit_in_a_key() {
        let (b, _) = parse_bindings("n = una-app-con-nombre-largo\n");
        for layer in [Layer::Apps, Layer::Window] {
            for c in map_view(layer, &b, &["otra-app-con-nombre-largo"]).cells {
                assert!(c.label.chars().count() <= LABEL_MAX, "{c:?}");
            }
        }
    }
}
