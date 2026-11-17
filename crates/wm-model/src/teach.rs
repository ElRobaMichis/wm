//! "Sugerir atajos" (plan-v1.2.md §6, ítem 6a): cuando la persona va a una
//! app con Alt+Tab o la barra de tareas, un aviso le recuerda su letra
//! ("La próxima vez: Caps + B"). Los topes, puros y probados acá:
//!
//! - recién desde el 3.er cambio a mano de la sesión (los primeros no
//!   molestan);
//! - como mucho un aviso cada 10 minutos;
//! - nunca más para una letra que ya usó 5 veces con `Caps` o de la que ya
//!   vio 5 avisos.
//!
//! Lo que dura entre sesiones (usos y avisos por letra, hasta 5) se guarda
//! en `%LOCALAPPDATA%\wm\aprendizaje.txt`. Como los topes son chicos, el
//! archivo cambia pocas veces en toda la vida.

use std::path::PathBuf;

/// El aviso recién desde este cambio a mano de la sesión.
pub const AFTER_SWITCHES: u32 = 3;
/// Un aviso cada tanto, como mucho (µs).
pub const EVERY_US: u64 = 10 * 60 * 1_000_000;
/// Usos o avisos de una letra que la dan por aprendida.
pub const LEARNED: u8 = 5;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Teach {
    /// Veces que usó `Caps` + cada letra (A–Z), hasta [`LEARNED`].
    uses: [u8; 26],
    /// Avisos de cada letra, hasta [`LEARNED`].
    tips: [u8; 26],
    /// Cambios a mano a una app con letra en esta sesión.
    switches: u32,
    last_tip_us: Option<u64>,
}

fn slot(letter: u8) -> Option<usize> {
    letter.is_ascii_uppercase().then(|| usize::from(letter - b'A'))
}

impl Teach {
    /// Lee `aprendizaje.txt` ("B | usos 3 | avisos 1"); lo demás se ignora.
    pub fn parse(text: &str) -> Self {
        let mut t = Self::default();
        for line in text.lines() {
            let mut parts = line.split('|').map(str::trim);
            let Some(i) = parts.next().and_then(|l| match l.as_bytes() {
                [c] => slot(c.to_ascii_uppercase()),
                _ => None,
            }) else {
                continue;
            };
            for p in parts {
                let n = |key: &str| p.strip_prefix(key).and_then(|v| v.trim().parse::<u8>().ok());
                if let Some(n) = n("usos") {
                    t.uses[i] = n.min(LEARNED);
                } else if let Some(n) = n("avisos") {
                    t.tips[i] = n.min(LEARNED);
                }
            }
        }
        t
    }

    pub fn format(&self) -> String {
        let mut s = String::from(
            "# Sugerir atajos de wm: cuántas veces usaste Caps + cada letra y cuántos\n\
             # avisos viste (con 5 de cualquiera, esa letra no avisa más).\n",
        );
        for i in 0..26 {
            if self.uses[i] > 0 || self.tips[i] > 0 {
                let letter = (b'A' + i as u8) as char;
                s.push_str(&format!(
                    "{letter} | usos {} | avisos {}\n",
                    self.uses[i], self.tips[i]
                ));
            }
        }
        s
    }

    /// `Caps` + `letter` llegó a una ventana. `true` si cambió lo guardado.
    pub fn used(&mut self, letter: u8) -> bool {
        let Some(i) = slot(letter) else { return false };
        if self.uses[i] >= LEARNED {
            return false;
        }
        self.uses[i] += 1;
        true
    }

    /// Un cambio a mano a una app con `letter`: ¿avisar? (si avisa, cambió lo
    /// guardado).
    pub fn switched(&mut self, letter: u8, now_us: u64) -> bool {
        let Some(i) = slot(letter) else { return false };
        self.switches += 1;
        let recent = self.last_tip_us.is_some_and(|t| now_us.saturating_sub(t) < EVERY_US);
        if self.switches < AFTER_SWITCHES
            || recent
            || self.uses[i] >= LEARNED
            || self.tips[i] >= LEARNED
        {
            return false;
        }
        self.tips[i] += 1;
        self.last_tip_us = Some(now_us);
        true
    }
}

/// `%LOCALAPPDATA%\wm\aprendizaje.txt`, junto a `wm.log`.
pub fn learning_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("wm").join("aprendizaje.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60 * 1_000_000;

    #[test]
    fn the_first_switches_and_close_tips_are_quiet() {
        let mut t = Teach::default();
        assert!(!t.switched(b'B', 0));
        assert!(!t.switched(b'B', MIN));
        assert!(t.switched(b'B', 2 * MIN), "el tercero avisa");
        assert!(!t.switched(b'N', 5 * MIN), "uno cada 10 minutos");
        assert!(t.switched(b'N', 12 * MIN));
    }

    #[test]
    fn a_learned_letter_never_tips_again() {
        let mut t = Teach { switches: 10, ..Teach::default() };
        for _ in 0..5 {
            assert!(t.used(b'B'));
        }
        assert!(!t.used(b'B'), "ya no cambia nada");
        assert!(!t.switched(b'B', 0), "5 usos: aprendida");
        let mut t = Teach { switches: 10, ..Teach::default() };
        for k in 0..5 {
            assert!(t.switched(b'Q', k * 11 * MIN));
        }
        assert!(!t.switched(b'Q', 100 * MIN), "5 avisos: basta");
    }

    #[test]
    fn round_trip() {
        let mut t = Teach::default();
        t.used(b'B');
        t.used(b'B');
        t.switches = 5;
        t.switched(b'R', 0);
        let text = t.format();
        assert!(text.contains("B | usos 2 | avisos 0\nR | usos 0 | avisos 1\n"), "{text}");
        let back = Teach::parse(&text);
        assert_eq!(back.uses, t.uses);
        assert_eq!(back.tips, t.tips);
        assert_eq!(back.switches, 0, "la sesión no se guarda");
        assert_eq!(Teach::parse("basura\nz | usos 9\n").uses[25], LEARNED);
    }
}
