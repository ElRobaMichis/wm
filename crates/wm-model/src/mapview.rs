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
//! - **Fila 1–4 (hito 4):** los escritorios. En la capa de apps, el activo
//!   en `accent` y cada uno con su cantidad de ventanas (vacío, en `text-3`);
//!   en la capa de ventana, "llevar" en los que no son el activo.

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
    /// El escritorio activo: `accent`.
    Accent,
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
    /// La fila 1–4 (escritorios), arriba de las letras.
    pub digits: Vec<KeyCell>,
    pub footer: String,
}

/// Lo que el mapa muestra de los escritorios.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeskSummary {
    pub active: u8,
    /// Ventanas en cada escritorio (1 a 4).
    pub counts: [usize; 4],
}

impl Default for DeskSummary {
    fn default() -> Self {
        Self { active: 1, counts: [0; 4] }
    }
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

const APPS_FOOTER: &str = "letra = ir a la app  ·  número = escritorio  ·  Alt = acomodar  ·  arrastrar con el mouse = mover la ventana";
const WINDOW_FOOTER: &str = "dos juntas = esquina  ·  repetir = otro monitor  ·  Z = deshacer  ·  número = llevar  ·  clic derecho + arrastrar = tamaño";

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
    map_view_with(layer, bindings, open, DeskSummary::default())
}

/// La fila de escritorios de una capa.
fn digits(layer: Layer, desks: DeskSummary) -> Vec<KeyCell> {
    (1..=4u8)
        .map(|n| {
            let letter = char::from(b'0' + n);
            let count = desks.counts[usize::from(n - 1)];
            let active = n == desks.active;
            let (label, tone) = match layer {
                Layer::Window if active => (String::new(), Tone::Dim),
                Layer::Window => ("llevar".to_owned(), Tone::Open),
                _ => {
                    let label = match count {
                        0 if active => "aquí".to_owned(),
                        0 => String::new(),
                        1 => "1 ventana".to_owned(),
                        n => format!("{n} ventanas"),
                    };
                    let tone = if active {
                        Tone::Accent
                    } else if count > 0 {
                        Tone::Normal
                    } else {
                        Tone::Dim
                    };
                    (label, tone)
                }
            };
            KeyCell { letter, label, tone }
        })
        .collect()
}

/// Como [`map_view`], con los escritorios.
pub fn map_view_with(
    layer: Layer,
    bindings: &[AppBinding],
    open: &[&str],
    desks: DeskSummary,
) -> MapView {
    map_view_targets(layer, bindings, open, desks, &[])
}

/// Como [`map_view_with`], con la app a la que va **de verdad** cada letra
/// ahora (`targets`: letra → app de la ventana a la que va el próximo toque;
/// plan-apps.md §5). Una letra con destino se muestra como abierta.
pub fn map_view_targets(
    layer: Layer,
    bindings: &[AppBinding],
    open: &[&str],
    desks: DeskSummary,
    targets: &[(u8, String, usize)],
) -> MapView {
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
                        app.starts_with(lower)
                            && !bindings.iter().any(|b| b.names().any(|n| n == **app))
                    })
                };
                let found = targets.iter().find(|(l, _, _)| *l == letter as u8);
                let target = found.map(|(_, a, _)| a);
                // Con 3 o más ventanas en la letra (hay lista): "Brave · 5".
                let count = found.map_or(0, |(_, _, n)| *n);
                let with_count = |name: String| -> String {
                    if count < 3 {
                        return shorten(&name);
                    }
                    let tail = format!(" · {count}");
                    let room = LABEL_MAX.saturating_sub(tail.chars().count()).max(3);
                    let base: String = if name.chars().count() > room {
                        name.chars().take(room - 1).collect::<String>() + "…"
                    } else {
                        name
                    };
                    base + &tail
                };
                match bound {
                    Some(b) => {
                        let tone = if target.is_some() || b.names().any(|n| open.contains(&n)) {
                            Tone::Open
                        } else {
                            Tone::Normal
                        };
                        let name = b.label.clone().unwrap_or_else(|| display_name(&b.app));
                        (with_count(name), tone)
                    }
                    None if target.is_some() => {
                        (with_count(display_name(target.expect("visto"))), Tone::Normal)
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
    MapView { layer, cells, digits: digits(layer, desks), footer }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_letter_with_a_list_shows_how_many_windows() {
        let targets = vec![(b'B', "brave".to_owned(), 5), (b'N', "notepad".to_owned(), 2)];
        let v = map_view_targets(Layer::Apps, &[], &[], DeskSummary::default(), &targets);
        let b = v.cells.iter().find(|c| c.letter == 'B').unwrap();
        assert_eq!(b.label, "Brave · 5");
        let n = v.cells.iter().find(|c| c.letter == 'N').unwrap();
        assert_eq!(n.label, "Notepad", "con 2 no hay lista");
        let long = vec![(b'K', "kicadpcbnewer".to_owned(), 12)];
        let v = map_view_targets(Layer::Apps, &[], &[], DeskSummary::default(), &long);
        let k = v.cells.iter().find(|c| c.letter == 'K').unwrap();
        assert!(k.label.ends_with(" · 12") && k.label.chars().count() <= LABEL_MAX, "{}", k.label);
    }

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
    fn the_desk_row_shows_the_active_desk_and_counts() {
        let desks = DeskSummary { active: 2, counts: [3, 1, 0, 0] };
        let v = map_view_with(Layer::Apps, &[], &[], desks);
        assert_eq!(v.digits.len(), 4);
        assert_eq!(v.digits[0].label, "3 ventanas");
        assert_eq!(v.digits[0].tone, Tone::Normal);
        assert_eq!(v.digits[1].tone, Tone::Accent);
        assert_eq!(v.digits[1].label, "1 ventana");
        assert_eq!(v.digits[2].tone, Tone::Dim);
        let w = map_view_with(Layer::Window, &[], &[], desks);
        assert_eq!(w.digits[1].tone, Tone::Dim, "no se lleva al escritorio en el que está");
        assert_eq!(w.digits[0].label, "llevar");
        for c in v.digits.iter().chain(&w.digits) {
            assert!(c.label.chars().count() <= LABEL_MAX, "{c:?}");
        }
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
