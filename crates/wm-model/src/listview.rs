//! La lista de ventanas de una letra (plan-apps.md §8): qué se dibuja, sin
//! dibujar. Con 3 o más ventanas, el segundo toque de la letra la muestra; la
//! selección se mueve con cada toque y soltar `Caps` va a la elegida.

use crate::world::WinInfo;

/// Filas que se ven a la vez.
pub const MAX_ROWS: usize = 12;
/// Largo máximo de un título (caracteres).
const MAX_TITLE: usize = 64;

/// Una fila: el título y, a la derecha, datos chicos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub title: String,
    /// "escritorio 3", "minimizada", la app si la letra mezcla apps.
    pub meta: String,
}

/// La lista lista para dibujar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListView {
    pub letter: char,
    /// "Brave" (o la app de la elegida, si la letra mezcla apps).
    pub app: String,
    /// Cuántas ventanas tiene la letra.
    pub count: usize,
    /// Las filas visibles (como mucho [`MAX_ROWS`]).
    pub rows: Vec<ListRow>,
    /// La elegida, en `rows`.
    pub selected: usize,
    /// Cuántas quedan fuera arriba y abajo.
    pub above: usize,
    pub below: usize,
    pub footer: String,
}

const SEPARATORS: [&str; 4] = [" - ", " — ", " – ", " | "];

/// Los títulos sin la parte que comparten todos (" - Brave", "Autodesk
/// AutoCAD 2025 - "), cortados a un largo razonable. Nunca deja uno vacío.
pub fn distinct_titles(titles: &[&str]) -> Vec<String> {
    let chars: Vec<Vec<char>> = titles.iter().map(|t| t.chars().collect()).collect();
    let (mut cut_front, mut cut_back) = (0, 0);
    if chars.len() >= 2 {
        let min = chars.iter().map(Vec::len).min().unwrap_or(0);
        // Sufijo común, cortado en un separador (" - Brave").
        let mut n = 0;
        while n < min
            && chars.iter().all(|c| c[c.len() - 1 - n] == chars[0][chars[0].len() - 1 - n])
        {
            n += 1;
        }
        let suffix: String = chars[0][chars[0].len() - n..].iter().collect();
        cut_back = SEPARATORS
            .iter()
            .filter_map(|s| suffix.find(s).map(|i| suffix[i..].chars().count()))
            .max()
            .unwrap_or(0);
        // Prefijo común, cortado después de un separador.
        let mut n = 0;
        while n < min && chars.iter().all(|c| c[n] == chars[0][n]) {
            n += 1;
        }
        let prefix: String = chars[0][..n].iter().collect();
        cut_front = SEPARATORS
            .iter()
            .filter_map(|s| prefix.rfind(s).map(|i| prefix[..i + s.len()].chars().count()))
            .max()
            .unwrap_or(0);
    }
    chars
        .iter()
        .map(|c| {
            let end = c.len().saturating_sub(cut_back);
            let start = cut_front.min(end);
            let mut t: String = c[start..end].iter().collect::<String>().trim().to_owned();
            if t.is_empty() {
                t = c.iter().collect::<String>().trim().to_owned();
            }
            if t.chars().count() > MAX_TITLE {
                t = t.chars().take(MAX_TITLE - 1).collect::<String>().trim_end().to_owned() + "…";
            }
            if t.is_empty() { "(sin título)".to_owned() } else { t }
        })
        .collect()
}

/// Qué filas se ven: `max` alrededor de la elegida.
pub fn window(n: usize, selected: usize, max: usize) -> std::ops::Range<usize> {
    if n <= max {
        return 0..n;
    }
    let start = selected.saturating_sub(max / 2).min(n - max);
    start..start + max
}

/// La lista de la letra: `wins` en el orden del primer toque, `selected`
/// la elegida, `active_desk` el escritorio activo.
pub fn list_view(letter: u8, wins: &[WinInfo], selected: usize, active_desk: u8) -> ListView {
    let titles: Vec<&str> = wins.iter().map(|w| w.title.as_str()).collect();
    let clean = distinct_titles(&titles);
    let names: Vec<&str> = wins.iter().map(|w| w.app_id().name()).collect();
    let mixed = names.windows(2).any(|p| p[0] != p[1]);
    let all: Vec<ListRow> = wins
        .iter()
        .zip(clean)
        .map(|(w, title)| {
            let mut meta = Vec::new();
            if mixed {
                meta.push(crate::mapview::display_name(w.app_id().name()));
            }
            if let Some(d) = w.desk.filter(|&d| d != active_desk) {
                meta.push(format!("escritorio {d}"));
            }
            if w.minimized {
                meta.push("minimizada".to_owned());
            }
            ListRow { title, meta: meta.join(" · ") }
        })
        .collect();
    let selected = selected.min(all.len().saturating_sub(1));
    let range = window(all.len(), selected, MAX_ROWS);
    let letter_c = letter.to_ascii_uppercase() as char;
    let app = names.get(selected).map(|n| crate::mapview::display_name(n)).unwrap_or_default();
    ListView {
        letter: letter_c,
        app: if mixed { format!("{letter_c}") } else { app },
        count: all.len(),
        above: range.start,
        below: all.len() - range.end,
        selected: selected - range.start,
        rows: all[range].to_vec(),
        footer: format!("{letter_c} siguiente · ↑ ↓ mover · soltar Caps para ir · Esc cancelar"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{WinInfo, WinKey};

    fn win(n: isize, app: &str, title: &str) -> WinInfo {
        WinInfo {
            key: WinKey { hwnd: n, pid: n as u32 },
            app: app.into(),
            title: title.into(),
            desk: Some(1),
            ..WinInfo::default()
        }
    }

    #[test]
    fn common_suffix_and_prefix_go_away() {
        let t = distinct_titles(&["YouTube - Brave", "Wikipedia - Brave", "Kagi Search - Brave"]);
        assert_eq!(t, vec!["YouTube", "Wikipedia", "Kagi Search"]);
        let t = distinct_titles(&[
            "Autodesk AutoCAD 2025 - [Planta baja.dwg]",
            "Autodesk AutoCAD 2025 - [Corte A.dwg]",
        ]);
        assert_eq!(t, vec!["[Planta baja.dwg]", "[Corte A.dwg]"]);
    }

    #[test]
    fn titles_never_end_empty_and_one_stays_intact() {
        assert_eq!(distinct_titles(&["Brave", "Brave"]), vec!["Brave", "Brave"]);
        assert_eq!(distinct_titles(&["Solo - Brave"]), vec!["Solo - Brave"]);
        assert_eq!(distinct_titles(&["", "x"]), vec!["(sin título)", "x"]);
        let long = "á".repeat(100);
        let t = distinct_titles(&[long.as_str()]);
        assert_eq!(t[0].chars().count(), MAX_TITLE);
        assert!(t[0].ends_with('…'));
    }

    #[test]
    fn the_window_always_contains_the_selected_row() {
        for n in 0..30 {
            for s in 0..n {
                let r = window(n, s, MAX_ROWS);
                assert!(r.contains(&s), "{n} {s} {r:?}");
                assert!(r.len() <= MAX_ROWS);
            }
        }
        assert_eq!(window(5, 3, MAX_ROWS), 0..5);
    }

    #[test]
    fn rows_say_desk_minimized_and_app_only_when_needed() {
        let mut wins = vec![
            win(1, "brave", "YouTube - Brave"),
            win(2, "brave", "Wikipedia - Brave"),
            win(3, "brave", "Mapas - Brave"),
        ];
        wins[1].desk = Some(3);
        wins[2].minimized = true;
        let v = list_view(b'b', &wins, 1, 1);
        assert_eq!(v.app, "Brave");
        assert_eq!(v.count, 3);
        assert_eq!(v.selected, 1);
        assert_eq!(v.rows[0].meta, "");
        assert_eq!(v.rows[1].meta, "escritorio 3");
        assert_eq!(v.rows[2].meta, "minimizada");
        let mixed = vec![
            win(1, "nanofy", "Nanofy"),
            win(2, "notepad", "a.txt"),
            win(3, "notepad", "b.txt"),
        ];
        let v = list_view(b'n', &mixed, 0, 1);
        assert!(v.rows[0].meta.contains("Nanofy"), "{v:?}");
    }
}
