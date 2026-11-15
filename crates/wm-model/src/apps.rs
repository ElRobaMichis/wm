//! Capa de apps del mapa "Apps primero" (keyboard-design.md §6.1, regla 1):
//! Hyper + letra = ir a la app con esa inicial; repetir = siguiente ventana.

/// Asignación explícita de una letra a una app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppBinding {
    /// Letra A–Z en mayúscula (código ASCII).
    pub letter: u8,
    /// Nombre del ejecutable sin extensión, en minúsculas ("chrome").
    pub app: String,
    /// Ruta para abrir la app si no está abierta.
    pub launch: Option<String>,
}

/// Lee el archivo de asignaciones. Formato, una por línea:
///
/// ```text
/// # comentario
/// c = chrome
/// t = windowsterminal | C:\Users\yo\AppData\Local\Microsoft\WindowsApps\wt.exe
/// ```
///
/// Devuelve las asignaciones válidas y los errores (número de línea y motivo).
pub fn parse_bindings(text: &str) -> (Vec<AppBinding>, Vec<String>) {
    let mut bindings: Vec<AppBinding> = Vec::new();
    let mut errors = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            errors.push(format!("línea {}: falta '='", i + 1));
            continue;
        };
        let key = key.trim();
        let letter = match key.as_bytes() {
            [c] if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
            _ => {
                errors.push(format!("línea {}: '{key}' no es una letra", i + 1));
                continue;
            }
        };
        let (app, launch) = match value.split_once('|') {
            Some((app, path)) => (app.trim(), Some(path.trim().to_owned())),
            None => (value.trim(), None),
        };
        let app = app.trim_end_matches(".exe").to_lowercase();
        if app.is_empty() {
            errors.push(format!("línea {}: falta el nombre de la app", i + 1));
            continue;
        }
        bindings.retain(|b| b.letter != letter); // la última asignación gana
        bindings.push(AppBinding { letter, app, launch: launch.filter(|p| !p.is_empty()) });
    }
    (bindings, errors)
}

/// Qué hacer al apretar Hyper + letra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target<W> {
    /// Traer esta ventana al frente.
    Focus { window: W, app: String },
    /// La app está asignada pero no abierta: abrirla.
    Launch { app: String, path: String },
    /// No hay nada para esa letra.
    Nothing,
}

/// Decide la acción para `letter`.
///
/// - `windows`: ventanas de usuario en orden Z (la primera es la de arriba),
///   con el nombre de su app.
/// - Si la letra no está asignada, se usa la primera app abierta (en orden Z)
///   cuyo nombre empiece con esa letra y que no esté asignada a otra letra.
/// - Si la app ya está al frente, pasa a su siguiente ventana: la de más abajo
///   en el orden Z, así repetir recorre todas.
pub fn resolve<W: Copy + PartialEq>(
    letter: u8,
    bindings: &[AppBinding],
    windows: &[(W, &str)],
    foreground: Option<W>,
) -> Target<W> {
    let letter = letter.to_ascii_uppercase();
    let binding = bindings.iter().find(|b| b.letter == letter);
    let app: String = match binding {
        Some(b) => b.app.clone(),
        None => {
            let initial = letter.to_ascii_lowercase();
            let auto = windows.iter().find(|(_, app)| {
                app.as_bytes().first() == Some(&initial)
                    && !bindings.iter().any(|b| b.app == *app)
            });
            match auto {
                Some((_, app)) => (*app).to_owned(),
                None => return Target::Nothing,
            }
        }
    };

    let mine: Vec<W> = windows.iter().filter(|(_, a)| *a == app).map(|(w, _)| *w).collect();
    match (mine.first(), mine.last()) {
        (None, _) => match binding.and_then(|b| b.launch.clone()) {
            Some(path) => Target::Launch { app, path },
            None => Target::Nothing,
        },
        (Some(&top), Some(&bottom)) => {
            let window = if foreground.is_some_and(|f| mine.contains(&f)) { bottom } else { top };
            Target::Focus { window, app }
        }
        (Some(_), None) => unreachable!("first y last existen juntos"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOWS: [(u32, &str); 5] =
        [(1, "code"), (2, "chrome"), (3, "slack"), (4, "chrome"), (5, "spotify")];

    #[test]
    fn parses_bindings_comments_and_errors() {
        let text = "# mis apps\nc = Chrome.exe\n\nt = windowsterminal | C:\\wt.exe\n12 = x\nz\n";
        let (b, errors) = parse_bindings(text);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], AppBinding { letter: b'C', app: "chrome".into(), launch: None });
        assert_eq!(b[1].launch.as_deref(), Some("C:\\wt.exe"));
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn unbound_letter_picks_topmost_matching_app() {
        // "c" sin asignar: la primera app que empieza con c en orden Z es code.
        assert_eq!(
            resolve(b'c', &[], &WINDOWS, None),
            Target::Focus { window: 1, app: "code".into() }
        );
    }

    #[test]
    fn binding_wins_over_auto() {
        let (b, _) = parse_bindings("c = chrome");
        assert_eq!(
            resolve(b'C', &b, &WINDOWS, Some(1)),
            Target::Focus { window: 2, app: "chrome".into() }
        );
    }

    #[test]
    fn apps_bound_to_other_letters_are_not_auto_picked() {
        let (b, _) = parse_bindings("v = code");
        assert_eq!(
            resolve(b'C', &b, &WINDOWS, None),
            Target::Focus { window: 2, app: "chrome".into() }
        );
    }

    #[test]
    fn repeating_cycles_to_the_bottom_window_of_the_app() {
        let (b, _) = parse_bindings("c = chrome");
        assert_eq!(
            resolve(b'C', &b, &WINDOWS, Some(2)),
            Target::Focus { window: 4, app: "chrome".into() }
        );
    }

    #[test]
    fn bound_but_closed_app_is_launched() {
        let (b, _) = parse_bindings("n = notion | C:\\Notion.exe");
        assert_eq!(
            resolve(b'N', &b, &WINDOWS, None),
            Target::Launch { app: "notion".into(), path: "C:\\Notion.exe".into() }
        );
    }

    #[test]
    fn nothing_for_unknown_letter() {
        assert_eq!(resolve(b'Q', &[], &WINDOWS, None), Target::Nothing);
    }
}
