//! Capa de apps del mapa "Apps primero" (keyboard-design.md §6.1, regla 1):
//! Hyper + letra = ir a la app con esa inicial; repetir = siguiente ventana.

/// Asignación explícita de una letra a una app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppBinding {
    /// Letra A–Z en mayúscula (código ASCII).
    pub letter: u8,
    /// Nombre del ejecutable sin extensión, en minúsculas ("chrome").
    pub app: String,
    /// Otros ejecutables de la misma app ("steamwebhelper" para Steam).
    pub aliases: Vec<String>,
    /// Cómo abrirla si no está abierta: ruta de un `.exe` o
    /// `shell:AppsFolder\<AppID>` (accesos directos y apps de la Store).
    pub launch: Option<String>,
    /// Nombre visible ("Steam"), para el mapa y los avisos.
    pub label: Option<String>,
    /// v1.1: dónde va al abrirse ("al abrir: derecha").
    pub open: Option<crate::rules::OpenRule>,
    /// Campos que esta versión no entiende: se conservan al reescribir.
    pub extra: Vec<String>,
}

impl AppBinding {
    /// Todos los nombres de ejecutable de la app.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.app.as_str()).chain(self.aliases.iter().map(String::as_str))
    }

    /// El nombre para mostrar.
    pub fn display(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.app)
    }

    /// ¿La ventana es de esta app? Por el ejecutable (o un alias), por su
    /// familia (el lanzador que la abrió desde su carpeta: Steam →
    /// steamwebhelper) o por el AUMID del destino (apps de la Store).
    pub fn matches(&self, id: &AppId) -> bool {
        self.names().any(|n| n == id.exe || id.family == Some(n))
            || id.aumid.is_some_and(|a| {
                self.launch.as_deref().is_some_and(|l| {
                    l.strip_prefix("shell:AppsFolder\\").is_some_and(|x| x.eq_ignore_ascii_case(a))
                })
            })
    }
}

/// Quién es la app de una ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppId<'a> {
    /// Ejecutable sin extensión, en minúsculas.
    pub exe: &'a str,
    /// El ejecutable de la familia (el ancestro que la abrió desde su
    /// carpeta), si es otro: "steam" para "steamwebhelper".
    pub family: Option<&'a str>,
    /// AUMID del proceso (apps MSIX y de la Store).
    pub aumid: Option<&'a str>,
}

impl<'a> AppId<'a> {
    pub fn exe(exe: &'a str) -> Self {
        Self { exe, family: None, aumid: None }
    }

    /// El nombre de la app: el de la familia si hay, si no el ejecutable.
    pub fn name(&self) -> &'a str {
        self.family.unwrap_or(self.exe)
    }
}

/// Lee el archivo de asignaciones. Formato, una por línea:
///
/// ```text
/// # comentario
/// c = chrome
/// s = steam, steamwebhelper | C:\Program Files (x86)\Steam\steam.exe | Steam
/// t = windowsterminal | shell:AppsFolder\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App
/// ```
///
/// `letra = app[, otro exe…] | cómo abrirla (opcional) | nombre visible
/// (opcional)`. Devuelve las asignaciones válidas y los errores (número de
/// línea y motivo).
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
        let mut fields = value.split('|').map(str::trim);
        let names: Vec<String> = fields
            .next()
            .unwrap_or("")
            .split(',')
            .map(|n| n.trim().trim_end_matches(".exe").to_lowercase())
            .filter(|n| !n.is_empty())
            .collect();
        let launch = fields.next().filter(|p| !p.is_empty()).map(str::to_owned);
        let label = fields.next().filter(|p| !p.is_empty()).map(str::to_owned);
        // Desde el cuarto campo, pares "clave: valor" (v1.1); lo que no se
        // entiende se guarda tal cual.
        let mut open = None;
        let mut extra = Vec::new();
        for f in fields.filter(|f| !f.is_empty()) {
            match f.to_lowercase().strip_prefix(crate::rules::OPEN_KEY) {
                Some(v) => match crate::rules::OpenRule::parse(v) {
                    Some(r) => open = Some(r),
                    None => {
                        errors.push(format!("línea {}: «{f}» no es una ubicación", i + 1));
                        extra.push(f.to_owned());
                    }
                },
                None => extra.push(f.to_owned()),
            }
        }
        let Some((app, aliases)) = names.split_first() else {
            errors.push(format!("línea {}: falta el nombre de la app", i + 1));
            continue;
        };
        bindings.retain(|b| b.letter != letter); // la última asignación gana
        bindings.push(AppBinding {
            letter,
            app: app.clone(),
            aliases: aliases.to_vec(),
            launch,
            label,
            open,
            extra,
        });
    }
    (bindings, errors)
}

/// Archivo de asignaciones: `%APPDATA%\wm\apps.txt` (`None` sin `APPDATA`).
/// Lo lee el core y lo escribe la ventana de ajustes.
pub fn bindings_path() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|dir| std::path::PathBuf::from(dir).join("wm").join("apps.txt"))
}

const HEADER: &str = "\
# Caps + letra: la app de cada letra. Lo escribe la ventana de ajustes;
# también se puede editar a mano (los comentarios se conservan).
# letra = app (el ejecutable, sin .exe)[, otros ejecutables] | cómo abrirla si está cerrada (opcional) | nombre (opcional)
";

/// Letra de una línea de asignación (`None` si es comentario, está en blanco
/// o no empieza con "letra =").
fn binding_letter(raw: &str) -> Option<u8> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, _) = line.split_once('=')?;
    match key.trim().as_bytes() {
        [c] if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

fn format_binding(b: &AppBinding) -> String {
    let letter = b.letter.to_ascii_lowercase() as char;
    let names = b.names().collect::<Vec<_>>().join(", ");
    if b.open.is_some() || !b.extra.is_empty() {
        let mut parts =
            vec![names, b.launch.clone().unwrap_or_default(), b.label.clone().unwrap_or_default()];
        if let Some(r) = b.open {
            parts.push(format!("{} {}", crate::rules::OPEN_KEY, r.format()));
        }
        parts.extend(b.extra.iter().cloned());
        return format!(
            "{letter} = {}
",
            parts.join(" | ").replace(" |  | ", " | | ")
        );
    }
    match (&b.launch, &b.label) {
        (_, Some(label)) => {
            format!("{letter} = {names} | {} | {label}\n", b.launch.as_deref().unwrap_or(""))
        }
        (Some(path), None) => format!("{letter} = {names} | {path}\n"),
        (None, None) => format!("{letter} = {names}\n"),
    }
}

/// Escribe `bindings` sobre el texto de un archivo de asignaciones (vacío si
/// todavía no existe). Conserva comentarios, líneas en blanco y líneas con
/// errores; cambia cada asignación en su lugar, borra las letras que ya no
/// están (y las repetidas) y agrega las nuevas al final, en orden.
pub fn update_bindings(text: &str, bindings: &[AppBinding]) -> String {
    let mut out = String::new();
    if text.trim().is_empty() {
        out.push_str(HEADER);
    }
    let mut written = [false; 26];
    for raw in text.lines() {
        let Some(letter) = binding_letter(raw) else {
            if !text.trim().is_empty() {
                out.push_str(raw);
                out.push('\n');
            }
            continue;
        };
        let slot = &mut written[usize::from(letter - b'A')];
        if *slot {
            continue;
        }
        if let Some(b) = bindings.iter().find(|b| b.letter == letter) {
            out.push_str(&format_binding(b));
            *slot = true;
        }
    }
    let mut rest: Vec<&AppBinding> =
        bindings.iter().filter(|b| !written[usize::from(b.letter - b'A')]).collect();
    rest.sort_by_key(|b| b.letter);
    for b in rest {
        out.push_str(&format_binding(b));
    }
    out
}

/// Qué hacer al apretar Hyper + letra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target<W> {
    /// Las ventanas de la letra, de la más reciente a la más vieja (el orden
    /// Z): el mundo elige cuál, como Alt+Tab (plan-apps.md §5).
    Focus { windows: Vec<W>, app: String },
    /// La app está asignada pero no tiene ventanas: abrirla (o, si ya corre
    /// en la bandeja, abrirla otra vez para que muestre su ventana).
    Launch { app: String, path: String },
    /// Asignada, sin ventanas ni destino: abrirla desde el ejecutable de su
    /// proceso, si está corriendo (Steam en la bandeja).
    Revive { app: String, names: Vec<String> },
    /// No hay nada para esa letra.
    Nothing,
}

/// Decide la acción para `letter` (plan-apps.md §2 y §5).
///
/// - `windows`: ventanas de usuario en orden Z (la primera es la de arriba).
/// - Asignada: solo las ventanas de esa app.
/// - Sin asignar: todas las ventanas de las apps cuyo nombre empieza con la
///   letra y que no están asignadas a otra letra, en una sola lista.
pub fn resolve<W: Copy + PartialEq>(
    letter: u8,
    bindings: &[AppBinding],
    windows: &[(W, AppId)],
) -> Target<W> {
    let letter = letter.to_ascii_uppercase();
    if let Some(b) = bindings.iter().find(|b| b.letter == letter) {
        let mine: Vec<W> =
            windows.iter().filter(|(_, id)| b.matches(id)).map(|(w, _)| *w).collect();
        if !mine.is_empty() {
            return Target::Focus { windows: mine, app: b.display().to_owned() };
        }
        return match &b.launch {
            Some(path) => Target::Launch { app: b.display().to_owned(), path: path.clone() },
            None => Target::Revive {
                app: b.display().to_owned(),
                names: b.names().map(str::to_owned).collect(),
            },
        };
    }
    let initial = letter.to_ascii_lowercase();
    let auto: Vec<&(W, AppId)> = windows
        .iter()
        .filter(|(_, id)| {
            id.name().as_bytes().first() == Some(&initial)
                && !bindings.iter().any(|b| b.matches(id))
        })
        .collect();
    match auto.first() {
        Some((_, id)) => Target::Focus {
            windows: auto.iter().map(|(w, _)| *w).collect(),
            app: id.name().to_owned(),
        },
        None => Target::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[(u32, &'static str)]) -> Vec<(u32, AppId<'static>)> {
        list.iter().map(|&(w, exe)| (w, AppId::exe(exe))).collect()
    }

    const WINDOWS: [(u32, &str); 5] =
        [(1, "code"), (2, "chrome"), (3, "slack"), (4, "chrome"), (5, "spotify")];

    #[test]
    fn parses_bindings_comments_and_errors() {
        let text = "# mis apps\nc = Chrome.exe\n\nt = windowsterminal | C:\\wt.exe\n12 = x\nz\n";
        let (b, errors) = parse_bindings(text);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], AppBinding { letter: b'C', app: "chrome".into(), ..Default::default() });
        assert_eq!(b[1].launch.as_deref(), Some("C:\\wt.exe"));
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn parses_aliases_launch_and_label() {
        let (b, _) = parse_bindings(
            "s = steam, steamwebhelper | C:\\Steam\\steam.exe | Steam\nk = claude | | Claude\n",
        );
        assert_eq!(b[0].aliases, vec!["steamwebhelper".to_owned()]);
        assert_eq!(b[0].launch.as_deref(), Some("C:\\Steam\\steam.exe"));
        assert_eq!(b[0].display(), "Steam");
        assert_eq!(b[1].launch, None);
        assert_eq!(b[1].label.as_deref(), Some("Claude"));
    }

    #[test]
    fn unbound_letter_lists_every_window_of_its_apps_by_recency() {
        // "c" sin asignar: code y chrome, todas sus ventanas, en orden Z.
        assert_eq!(
            resolve(b'c', &[], &ids(&WINDOWS)),
            Target::Focus { windows: vec![1, 2, 4], app: "code".into() }
        );
    }

    #[test]
    fn binding_wins_over_auto() {
        let (b, _) = parse_bindings("c = chrome");
        assert_eq!(
            resolve(b'C', &b, &ids(&WINDOWS)),
            Target::Focus { windows: vec![2, 4], app: "chrome".into() }
        );
    }

    #[test]
    fn apps_bound_to_other_letters_are_not_auto_picked() {
        let (b, _) = parse_bindings("v = code");
        assert_eq!(
            resolve(b'C', &b, &ids(&WINDOWS)),
            Target::Focus { windows: vec![2, 4], app: "chrome".into() }
        );
    }

    #[test]
    fn steam_is_found_by_its_family() {
        // La ventana de Steam es de steamwebhelper, hijo de steam.exe.
        let windows =
            vec![(1u32, AppId { exe: "steamwebhelper", family: Some("steam"), aumid: None })];
        let (b, _) = parse_bindings("s = steam");
        assert_eq!(
            resolve(b'S', &b, &windows),
            Target::Focus { windows: vec![1], app: "steam".into() }
        );
        // Sin asignar, también cuenta como Steam (la S).
        assert!(matches!(resolve(b'S', &[], &windows), Target::Focus { .. }));
    }

    #[test]
    fn store_apps_are_found_by_aumid() {
        let windows =
            vec![(1u32, AppId { exe: "claude", family: None, aumid: Some("Claude_x!Claude") })];
        let (b, _) = parse_bindings("k = otra | shell:AppsFolder\\Claude_x!Claude | Claude");
        assert_eq!(
            resolve(b'K', &b, &windows),
            Target::Focus { windows: vec![1], app: "Claude".into() }
        );
    }

    #[test]
    fn bound_but_closed_app_is_launched() {
        let (b, _) = parse_bindings("n = notion | C:\\Notion.exe");
        assert_eq!(
            resolve(b'N', &b, &ids(&WINDOWS)),
            Target::Launch { app: "notion".into(), path: "C:\\Notion.exe".into() }
        );
    }

    #[test]
    fn bound_app_without_windows_or_path_is_revived() {
        let (b, _) = parse_bindings("s = steam, steamwebhelper");
        assert_eq!(
            resolve(b'S', &b, &ids(&WINDOWS)),
            Target::Revive {
                app: "steam".into(),
                names: vec!["steam".into(), "steamwebhelper".into()]
            }
        );
    }

    #[test]
    fn nothing_for_unknown_letter() {
        assert_eq!(resolve::<u32>(b'Q', &[], &ids(&WINDOWS)), Target::Nothing);
    }

    fn bind(letter: u8, app: &str, launch: Option<&str>) -> AppBinding {
        AppBinding { letter, app: app.into(), launch: launch.map(Into::into), ..Default::default() }
    }

    #[test]
    fn open_rules_and_unknown_fields_round_trip() {
        let text = "b = brave | C:/b.exe | Brave | al abrir: derecha
n = notepad | | | al abrir: centrada 60 | escritorio: 2
";
        let (b, errors) = parse_bindings(text);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(b[0].open, Some(crate::rules::OpenRule::Place(crate::Placement::RightHalf)));
        assert_eq!(b[1].open, Some(crate::rules::OpenRule::Centered(60)));
        assert_eq!(b[1].extra, vec!["escritorio: 2".to_owned()]);
        let again = update_bindings(text, &b);
        assert_eq!(parse_bindings(&again).0, b, "{again}");
        // Un archivo viejo (tres campos) se lee igual.
        let (old, _) = parse_bindings(
            "s = steam | C:/s.exe | Steam
",
        );
        assert_eq!(old[0].open, None);
    }

    #[test]
    fn aliases_and_labels_round_trip() {
        let b = AppBinding {
            letter: b'S',
            app: "steam".into(),
            aliases: vec!["steamwebhelper".into()],
            launch: None,
            label: Some("Steam".into()),
            ..Default::default()
        };
        let text = update_bindings("", std::slice::from_ref(&b));
        assert!(text.ends_with("s = steam, steamwebhelper |  | Steam\n"), "{text}");
        assert_eq!(parse_bindings(&text).0, vec![b]);
    }

    #[test]
    fn new_file_gets_header_and_sorted_bindings() {
        let text = update_bindings("", &[bind(b'R', "brave", None), bind(b'C', "chrome", None)]);
        assert!(text.starts_with("# Caps + letra"));
        assert!(text.ends_with("c = chrome\nr = brave\n"));
        let (b, errors) = parse_bindings(&text);
        assert!(errors.is_empty());
        assert_eq!(b, vec![bind(b'C', "chrome", None), bind(b'R', "brave", None)]);
    }

    #[test]
    fn update_keeps_comments_and_changes_in_place() {
        let old =
            "# mías\nc = chrome\n\n# trabajo\nt = windowsterminal | C:\\wt.exe\nz\nc = edge\n";
        let new = [bind(b'C', "code", None), bind(b'T', "windowsterminal", Some("C:\\wt.exe"))];
        let text = update_bindings(old, &new);
        assert_eq!(text, "# mías\nc = code\n\n# trabajo\nt = windowsterminal | C:\\wt.exe\nz\n");
    }

    #[test]
    fn update_removes_freed_letters_and_appends_new_ones() {
        let old = "c = chrome\ns = spotify\n";
        let text = update_bindings(old, &[bind(b'S', "spotify", None), bind(b'B', "brave", None)]);
        assert_eq!(text, "s = spotify\nb = brave\n");
    }

    #[test]
    fn crlf_files_round_trip() {
        let text = update_bindings("# x\r\nc = chrome\r\n", &[bind(b'C', "chrome", None)]);
        assert_eq!(text, "# x\nc = chrome\n");
    }
}
