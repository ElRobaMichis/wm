//! Opciones de `wm` (`%APPDATA%\wm\settings.txt`): las escriben los ajustes y
//! las lee el core, que las recarga solo al guardarse (como `apps.txt`).
//! Ver `funciones-ocultas.md`: este archivo es el "pendiente común" que
//! permite mostrar las opciones que estaban escondidas.
//!
//! Formato: una línea `clave = valor` por opción (`sí|no`, o un número en
//! `espacio_entre_ventanas`); `#` comenta. Las claves desconocidas se ignoran
//! y las que faltan toman su valor por defecto.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Animar las ventanas al moverlas (vuelos con fantasma).
    pub animate: bool,
    /// Al ir a una app con el teclado, el puntero va al centro de su ventana.
    pub mouse_follows_focus: bool,
    /// Borde de 2 px en `accent` alrededor de la ventana activa.
    pub focus_border: bool,
    /// Avisos en pantalla cuando una tecla no hace nada.
    pub notices: bool,
    /// v1.1: espacio entre ventanas y contra los bordes, en px a 100 %
    /// (0–24; 0 = sin espacio).
    pub gap: u8,
    /// v1.1: la regla "al abrir" se aplica en el monitor del puntero.
    pub open_on_pointer: bool,
}

/// El espacio máximo entre ventanas (px a 100 %).
pub const GAP_MAX: u8 = 24;

impl Default for Options {
    fn default() -> Self {
        Self {
            animate: true,
            mouse_follows_focus: false,
            focus_border: true,
            notices: true,
            gap: 0,
            open_on_pointer: false,
        }
    }
}

const KEYS: [&str; 5] = [
    "animaciones",
    "puntero_sigue_al_foco",
    "borde_de_foco",
    "avisos",
    "abrir_en_la_pantalla_del_puntero",
];
const GAP_KEY: &str = "espacio_entre_ventanas";

fn yes(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "sí" | "si" | "yes" | "1" | "true" => Some(true),
        "no" | "0" | "false" => Some(false),
        _ => None,
    }
}

impl Options {
    pub fn parse(text: &str) -> Self {
        let mut o = Self::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            if k.trim() == GAP_KEY {
                // No numérico: sin espacio; de más: el máximo.
                o.gap = v
                    .trim()
                    .trim_end_matches("px")
                    .trim()
                    .parse::<u32>()
                    .map_or(0, |n| n.min(u32::from(GAP_MAX)) as u8);
                continue;
            }
            let Some(v) = yes(v) else { continue };
            match k.trim() {
                "animaciones" => o.animate = v,
                "puntero_sigue_al_foco" => o.mouse_follows_focus = v,
                "borde_de_foco" => o.focus_border = v,
                "avisos" => o.notices = v,
                "abrir_en_la_pantalla_del_puntero" => o.open_on_pointer = v,
                _ => {}
            }
        }
        o
    }

    pub fn format(&self) -> String {
        let b = |v: bool| if v { "sí" } else { "no" };
        let values = [
            self.animate,
            self.mouse_follows_focus,
            self.focus_border,
            self.notices,
            self.open_on_pointer,
        ];
        let mut s = String::from(
            "# Opciones de wm. Las escriben los ajustes; se pueden editar a mano\n\
             # (sí o no). wm las vuelve a leer solo al guardar.\n",
        );
        for (k, v) in KEYS.iter().zip(values) {
            s.push_str(&format!("{k} = {}\n", b(v)));
        }
        s.push_str(&format!("{GAP_KEY} = {}\n", self.gap));
        s
    }
}

/// `%APPDATA%\wm\settings.txt`.
pub fn options_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("wm").join("settings.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let o = Options {
            animate: false,
            mouse_follows_focus: true,
            focus_border: false,
            notices: true,
            gap: 8,
            open_on_pointer: true,
        };
        assert_eq!(Options::parse(&o.format()), o);
        assert_eq!(Options::parse("espacio_entre_ventanas = 99").gap, GAP_MAX);
        assert_eq!(Options::parse("espacio_entre_ventanas = x").gap, 0);
        assert_eq!(Options::parse("espacio_entre_ventanas = 12 px").gap, 12);
        assert_eq!(Options::parse(""), Options::default());
        assert_eq!(Options::parse("animaciones = quizás\nnada = sí"), Options::default());
        let o = Options::parse("  animaciones=No # comentario\nAVISOS = sí");
        assert!(!o.animate);
    }
}
