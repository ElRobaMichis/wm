//! Registro local de uso (plan-v1.2.md §2, keyboard-design.md §12.3): qué
//! atajos usa la persona y cuántas veces por día, para las 1–2 semanas de uso
//! real. Solo en su PC (`%LOCALAPPDATA%\wm\uso.txt`), apagado por defecto y
//! **sin títulos ni nombres de app**: solo la tecla ("Caps + B").
//!
//! Como los avisos (`notices.rs`), sale de las líneas del log: cada acción
//! del usuario ya deja una, así que no hace falta tocar el modelo.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::notices::notice_for;

/// Cuántos días se guardan.
pub const KEEP_DAYS: i64 = 60;

/// Qué cuenta una línea del log, si es una acción del usuario.
pub fn key_for(line: &str) -> Option<String> {
    if line.starts_with("Caps + ") {
        let end = [line.find(" → "), line.find(':')].into_iter().flatten().min()?;
        let key = line[..end].trim();
        // La segunda línea de `Caps` + letra cuando cambia de escritorio
        // (`world_desks.rs`, `label`): la primera ya contó "Caps + B".
        if key == "Caps + letra" {
            return None;
        }
        // "Caps + B «…»" no pasa: siempre hay → o : antes de un título.
        let failed =
            line.contains("FALLÓ") || notice_for(line).is_some_and(|n| !n.starts_with("Abriendo"));
        return Some(if failed { format!("{key} (no hizo nada)") } else { key.to_owned() });
    }
    for (start, key) in [
        ("Mapa de teclado visible", "Mapa"),
        ("Regla al abrir → ", "Regla al abrir"),
        ("Atajos en pausa.", "Pausar atajos"),
        ("Mostrar todas las ventanas", "Mostrar todas las ventanas"),
        ("Abrir siempre así → ", "Abrir siempre así"),
    ] {
        if line.starts_with(start) {
            return Some(key.to_owned());
        }
    }
    // v1.2 §6: "A mano (Alt+Tab o la barra) → brave: tiene la B."
    if line.starts_with("A mano ")
        && let Some(rest) = line.split(": tiene la ").nth(1)
    {
        let letter = rest.trim_end_matches('.').trim();
        return Some(format!("A mano: app con letra {letter}"));
    }
    None
}

/// Contadores por día y acción.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Usage {
    counts: BTreeMap<(String, String), u32>,
}

impl Usage {
    /// Lee `uso.txt`; las líneas que no entiende se ignoran.
    pub fn parse(text: &str) -> Self {
        let mut u = Self::default();
        for line in text.lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let mut parts = line.split(" | ");
            let (Some(day), Some(key), Some(n), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let (day, key) = (day.trim(), key.trim());
            let Ok(n) = n.trim().parse::<u32>() else { continue };
            if days_from(day).is_none() || key.is_empty() {
                continue;
            }
            *u.counts.entry((day.to_owned(), key.to_owned())).or_default() += n;
        }
        u
    }

    pub fn add(&mut self, day: &str, key: String) {
        *self.counts.entry((day.to_owned(), key)).or_default() += 1;
    }

    /// Saca los días de hace más de [`KEEP_DAYS`] respecto de `today`.
    pub fn prune(&mut self, today: &str) {
        let Some(t) = days_from(today) else { return };
        self.counts.retain(|(day, _), _| days_from(day).is_some_and(|d| t - d <= KEEP_DAYS));
    }

    /// Día por día; en cada día, lo más usado primero.
    pub fn format(&self) -> String {
        let mut s = String::from(
            "# Registro de uso de wm. Solo en esta PC; no se envía a ningún lado.\n\
             # día | qué | veces\n",
        );
        let mut days: BTreeMap<&str, Vec<(&str, u32)>> = BTreeMap::new();
        for ((day, key), &n) in &self.counts {
            days.entry(day).or_default().push((key, n));
        }
        for (day, mut keys) in days {
            keys.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
            for (key, n) in keys {
                s.push_str(&format!("{day} | {key} | {n}\n"));
            }
        }
        s
    }

    /// El total de cada acción, lo más usado primero (para `wm uso`).
    pub fn totals(&self) -> Vec<(String, u32)> {
        let mut t: BTreeMap<&str, u32> = BTreeMap::new();
        for ((_, key), &n) in &self.counts {
            *t.entry(key).or_default() += n;
        }
        let mut v: Vec<(String, u32)> = t.into_iter().map(|(k, n)| (k.to_owned(), n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// Los días con algo, del primero al último.
    pub fn days(&self) -> Vec<String> {
        let mut d: Vec<String> = self.counts.keys().map(|(day, _)| day.clone()).collect();
        d.dedup();
        d
    }
}

/// Días desde el 1/1/1970 para `aaaa-mm-dd` (algoritmo de Howard Hinnant).
fn days_from(day: &str) -> Option<i64> {
    let mut it = day.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || day.len() != 10 {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `%LOCALAPPDATA%\wm\uso.txt`, junto a `wm.log`.
pub fn usage_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("wm").join("uso.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_counted_by_key_without_app_names() {
        let cases = [
            (
                "Caps + B → brave «Inbox - Gmail»: ok en 1.2 ms (desde la tecla 2.0 ms)",
                Some("Caps + B"),
            ),
            (
                "Caps + Alt + A → mitad izquierda (brave): 0 px en 1 pasada, 4.4 ms desde la tecla",
                Some("Caps + Alt + A"),
            ),
            (
                "Caps + Alt + A → mitad izquierda (brave): no hay otro monitor a la izquierda.",
                Some("Caps + Alt + A (no hizo nada)"),
            ),
            ("Caps + Q: no hay ninguna app abierta con la Q.", Some("Caps + Q (no hizo nada)")),
            ("Caps + N: abriendo notion… (espera)", Some("Caps + N")),
            (
                "Caps + 2 → escritorio 2: 1 mostradas, 2 escondidas, 9.0 ms desde la tecla",
                Some("Caps + 2"),
            ),
            ("Caps + Alt + W + D → esquina (code): 0 px en 1 pasada", Some("Caps + Alt + W + D")),
            (
                "Caps + clic derecho: el tamaño de brave no se puede cambiar (está maximizada).",
                Some("Caps + clic derecho (no hizo nada)"),
            ),
            (
                "Caps + Alt + Z → deshacer (mitad izquierda): FALLÓ: acceso denegado",
                Some("Caps + Alt + Z (no hizo nada)"),
            ),
            ("Mapa de teclado visible (monitor 1), 5.0 ms.", Some("Mapa")),
            ("Regla al abrir → mitad derecha (brave, «x»)", Some("Regla al abrir")),
            ("Regla al abrir (brave): la ventana no se quedó quieta; no la muevo.", None),
            ("Atajos en pausa.", Some("Pausar atajos")),
            ("Mostrar todas las ventanas → 3 mostradas", Some("Mostrar todas las ventanas")),
            ("A mano (Alt+Tab o la barra) → brave: tiene la B.", Some("A mano: app con letra B")),
            ("Opciones recargadas: animaciones sí.", None),
            ("Escritorios → 4 ventanas", None),
            ("Caps + letra → escritorio 2: 1 mostradas, 0 escondidas, 3.0 ms desde la tecla", None),
        ];
        for (line, want) in cases {
            assert_eq!(key_for(line).as_deref(), want, "{line}");
        }
    }

    #[test]
    fn round_trip_sorted_and_pruned() {
        let mut u = Usage::default();
        u.add("2026-11-18", "Caps + B".into());
        u.add("2026-11-18", "Mapa".into());
        u.add("2026-11-18", "Mapa".into());
        u.add("2026-09-01", "Caps + Q".into());
        let text = u.format();
        assert!(text.contains("2026-11-18 | Mapa | 2\n2026-11-18 | Caps + B | 1\n"), "{text}");
        assert_eq!(Usage::parse(&text), u);
        assert_eq!(u.totals()[0], ("Mapa".to_owned(), 2));
        u.prune("2026-11-18");
        assert_eq!(u.days(), vec!["2026-11-18".to_owned()], "septiembre ya pasó los 60 días");
        let bad = Usage::parse(
            "basura\n2026-13-01 | x | 1\n2026-11-18 | Mapa | dos\n2026-11-18 | Mapa | 3\n",
        );
        assert_eq!(bad.totals(), vec![("Mapa".to_owned(), 3)]);
    }

    #[test]
    fn day_numbers() {
        assert_eq!(days_from("1970-01-01"), Some(0));
        assert_eq!(days_from("2026-03-01").unwrap() - days_from("2026-02-28").unwrap(), 1);
        assert_eq!(days_from("2024-03-01").unwrap() - days_from("2024-02-28").unwrap(), 2);
        assert_eq!(days_from("2026-1-01"), None);
    }
}
