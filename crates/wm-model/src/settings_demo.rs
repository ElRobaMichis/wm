//! Datos de la pantalla de ajustes de prueba (spike 8, architecture.md
//! §13.10): los mismos para Slint y para la versión nativa, así se compara
//! solo el toolkit.

/// Apps que se pueden asignar (40).
pub const APPS: [&str; 40] = [
    "brave",
    "chrome",
    "firefox",
    "edge",
    "code",
    "windowsterminal",
    "rayo",
    "spotify",
    "discord",
    "slack",
    "teams",
    "zoom",
    "outlook",
    "thunderbird",
    "notion",
    "obsidian",
    "figma",
    "photoshop",
    "blender",
    "steam",
    "vlc",
    "obs64",
    "notepad",
    "word",
    "excel",
    "powerpnt",
    "onenote",
    "whatsapp",
    "telegram",
    "signal",
    "postman",
    "docker",
    "idea64",
    "pycharm64",
    "rider64",
    "gimp",
    "inkscape",
    "krita",
    "audacity",
    "keepassxc",
];

/// Nombre visible de cada app de [`APPS`] (mismo orden).
pub const APP_NAMES: [&str; 40] = [
    "Brave",
    "Chrome",
    "Firefox",
    "Edge",
    "VS Code",
    "Windows Terminal",
    "Rayo",
    "Spotify",
    "Discord",
    "Slack",
    "Teams",
    "Zoom",
    "Outlook",
    "Thunderbird",
    "Notion",
    "Obsidian",
    "Figma",
    "Photoshop",
    "Blender",
    "Steam",
    "VLC",
    "OBS Studio",
    "Notepad",
    "Word",
    "Excel",
    "PowerPoint",
    "OneNote",
    "WhatsApp",
    "Telegram",
    "Signal",
    "Postman",
    "Docker Desktop",
    "IntelliJ IDEA",
    "PyCharm",
    "Rider",
    "GIMP",
    "Inkscape",
    "Krita",
    "Audacity",
    "KeePassXC",
];

/// Letras asignadas al empezar (12): letra → índice en [`APPS`].
pub const ASSIGNED: [(u8, usize); 12] = [
    (b'B', 0),
    (b'C', 4),
    (b'D', 8),
    (b'E', 6),
    (b'F', 1),
    (b'M', 12),
    (b'N', 14),
    (b'O', 15),
    (b'S', 7),
    (b'T', 5),
    (b'W', 27),
    (b'Z', 11),
];

/// App asignada a cada letra A–Z (`None` = libre).
pub fn initial() -> Vec<(char, Option<usize>)> {
    (b'A'..=b'Z')
        .map(|l| (l as char, ASSIGNED.iter().find(|(x, _)| *x == l).map(|(_, a)| *a)))
        .collect()
}

/// ¿La fila coincide con la búsqueda (letra exacta o parte del nombre)?
pub fn matches(letter: char, app: Option<usize>, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let letter_hit = q.len() == 1 && q.starts_with(letter.to_ascii_lowercase());
    letter_hit
        || app.is_some_and(|a| APPS[a].contains(&q) || APP_NAMES[a].to_lowercase().contains(&q))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_of_twenty_six_assigned() {
        let rows = initial();
        assert_eq!(rows.len(), 26);
        assert_eq!(rows.iter().filter(|(_, a)| a.is_some()).count(), 12);
    }

    #[test]
    fn search_by_letter_or_app() {
        assert!(matches('B', Some(0), "b"));
        assert!(matches('B', Some(0), "rav"));
        assert!(!matches('A', None, "rav"));
        assert!(matches('A', None, ""));
    }
}
