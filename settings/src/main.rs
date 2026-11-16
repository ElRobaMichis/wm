//! `wm-settings` — ventana de ajustes en Slint (spike 8, architecture.md
//! §13.10). Binario aparte: Slint importa `dwrite`, `uiautomationcore`,
//! `comctl32`... en forma estática, y `wm.exe` (guardián y core) no debe
//! cargarlas.
//!
//! Las asignaciones son las de verdad: se leen de `%APPDATA%\wm\apps.txt`
//! (el archivo que usa el core) y Guardar lo reescribe en forma atómica,
//! conservando comentarios y rutas (`wm_model::apps::update_bindings`). Las
//! apps para elegir son las de `wm_model::settings_demo::APPS` más las del
//! archivo que no estén ahí. Opciones:
//! - `--lang es|en`: idioma (por defecto, español);
//! - `--bench x,y`: ventana en esa posición, con una marca magenta arriba a
//!   la izquierda, y `SHOWN` por stdout (lo usa `wm-bench settings`);
//! - `--at x,y`: solo la posición; `--light` / `--dark`: forzar el tema;
//! - `--show-unfinished`: muestra el perfil y las opciones escondidas en la
//!   primera versión (`funciones-ocultas.md`).

#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};
use wm_model::apps::{AppBinding, bindings_path, parse_bindings, update_bindings};
use wm_model::settings_demo::{APP_NAMES, APPS};

slint::include_modules!();

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Apps que se pueden elegir: ejecutable (sin `.exe`) y nombre visible.
struct Catalog {
    exe: Vec<String>,
    name: Vec<String>,
}

impl Catalog {
    /// Las conocidas y, al final, las del archivo que no están entre ellas.
    fn new(bindings: &[AppBinding]) -> Self {
        let mut exe: Vec<String> = APPS.iter().map(|a| (*a).to_owned()).collect();
        let mut name: Vec<String> = APP_NAMES.iter().map(|a| (*a).to_owned()).collect();
        for b in bindings {
            if !exe.contains(&b.app) {
                exe.push(b.app.clone());
                name.push(b.app.clone());
            }
        }
        Self { exe, name }
    }

    fn index(&self, app: &str) -> Option<usize> {
        self.exe.iter().position(|e| e == app)
    }

    /// ¿La fila coincide con la búsqueda (letra exacta o parte del nombre)?
    fn matches(&self, letter: char, app: Option<usize>, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let letter_hit = q.len() == 1 && q.starts_with(letter.to_ascii_lowercase());
        letter_hit
            || app.is_some_and(|a| {
                self.exe[a].contains(&q) || self.name[a].to_lowercase().contains(&q)
            })
    }
}

struct State {
    catalog: Catalog,
    /// Texto del archivo al abrir: Guardar escribe sobre él.
    file_text: String,
    /// Ruta para abrir cada app, si el archivo la tenía.
    launch: HashMap<String, String>,
    assigned: Vec<(char, Option<usize>)>,
    original: Vec<(char, Option<usize>)>,
    /// Índices (en `assigned`) de las filas visibles con la búsqueda actual.
    visible: Vec<usize>,
    /// Letra elegida (índice en `assigned`).
    current: usize,
    query: String,
}

fn refresh(ui: &SettingsWindow, st: &State) {
    let items: Vec<LetterItem> = st
        .visible
        .iter()
        .map(|&i| {
            let (l, a) = st.assigned[i];
            LetterItem {
                letter: SharedString::from(l.to_string()),
                name: a.map_or_else(SharedString::new, |a| st.catalog.name[a].as_str().into()),
                exe: a.map_or_else(SharedString::new, |a| {
                    format!("{}.exe", st.catalog.exe[a]).into()
                }),
                assigned: a.is_some(),
            }
        })
        .collect();
    ui.set_items(ModelRc::new(VecModel::from(items)));
    let sel = st.visible.iter().position(|&i| i == st.current).map_or(-1, |p| p as i32);
    ui.set_selected(sel);
    let (letter, app) = st.assigned[st.current];
    ui.set_selected_letter(SharedString::from(letter.to_string()));
    ui.set_choice(app.map_or(0, |a| a as i32 + 1));
    ui.set_assigned(st.assigned.iter().filter(|(_, a)| a.is_some()).count() as i32);
    ui.set_dirty(st.assigned != st.original);
}

/// Escribe las asignaciones en `path`: primero a un temporal y después lo
/// renombra encima, así nunca queda un archivo a medias.
fn save(path: &Path, st: &State) -> std::io::Result<()> {
    let bindings: Vec<AppBinding> = st
        .assigned
        .iter()
        .filter_map(|&(letter, app)| {
            let app = st.catalog.exe[app?].clone();
            Some(AppBinding { letter: letter as u8, launch: st.launch.get(&app).cloned(), app })
        })
        .collect();
    let text = update_bindings(&st.file_text, &bindings);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

fn main() -> Result<(), slint::PlatformError> {
    let lang = arg("--lang").unwrap_or_else(|| "es".to_owned());
    let bench = arg("--bench").and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?))
    });

    let ui = SettingsWindow::new()?;
    // Forzar tema (para revisar los dos sin cambiar el de Windows).
    if std::env::args().any(|a| a == "--light") {
        ui.global::<Theme>().set_dark(false);
    } else if std::env::args().any(|a| a == "--dark") {
        ui.global::<Theme>().set_dark(true);
    }
    // Después de crear la primera ventana: antes, Slint no tiene contexto y
    // la selección falla (en silencio si no se mira el resultado).
    if let Err(e) = slint::select_bundled_translation(&lang) {
        eprintln!("No se pudo elegir el idioma «{lang}»: {e:?}");
    }
    let path = bindings_path();
    let file_text = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let (bindings, _errors) = parse_bindings(&file_text);
    let catalog = Catalog::new(&bindings);
    let launch: HashMap<String, String> =
        bindings.iter().filter_map(|b| Some((b.app.clone(), b.launch.clone()?))).collect();
    let initial: Vec<(char, Option<usize>)> = (b'A'..=b'Z')
        .map(|l| {
            let app = bindings.iter().find(|b| b.letter == l).and_then(|b| catalog.index(&b.app));
            (l as char, app)
        })
        .collect();

    let mut choices = vec![ui.get_none_text()];
    choices.extend(catalog.name.iter().map(|a| SharedString::from(a.as_str())));
    ui.set_apps(ModelRc::new(VecModel::from(choices)));

    let st = Rc::new(RefCell::new(State {
        catalog,
        file_text,
        launch,
        assigned: initial.clone(),
        original: initial,
        visible: (0..26).collect(),
        current: 0,
        query: String::new(),
    }));
    refresh(&ui, &st.borrow());

    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_search(move |q| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        st.query = q.to_string();
        let q = st.query.clone();
        st.visible = (0..26)
            .filter(|&i| {
                let (l, a) = st.assigned[i];
                st.catalog.matches(l, a, &q)
            })
            .collect();
        if let Some(&first) = st.visible.first() {
            st.current = first;
        }
        refresh(&ui, &st);
    });
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_select(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        if let Some(&i) = st.visible.get(index.max(0) as usize) {
            st.current = i;
            let (letter, app) = st.assigned[i];
            ui.set_selected_letter(SharedString::from(letter.to_string()));
            ui.set_choice(app.map_or(0, |a| a as i32 + 1));
        }
    });
    let s = st.clone();
    ui.on_find_letter(move |text| {
        let st = s.borrow();
        let Some(c) = text.chars().next().filter(|c| c.is_ascii_alphabetic() && text.len() == 1)
        else {
            return -1;
        };
        let c = c.to_ascii_uppercase();
        st.visible.iter().position(|&i| st.assigned[i].0 == c).map_or(-1, |p| p as i32)
    });
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_choose(move |choice| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        let cur = st.current;
        st.assigned[cur].1 = if choice <= 0 { None } else { Some(choice as usize - 1) };
        refresh(&ui, &st);
    });
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some(path) = path.as_deref() else {
            eprintln!("Sin %APPDATA%: no hay dónde guardar.");
            return;
        };
        // Si falla, la ventana queda abierta con los cambios.
        match save(path, &s.borrow()) {
            Ok(()) => {
                let _ = ui.hide();
            }
            Err(e) => eprintln!("No se pudo guardar {}: {e}", path.display()),
        }
    });
    let weak = ui.as_weak();
    ui.on_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
    });

    // `--show-unfinished`: muestra lo escondido en la primera versión (el
    // perfil y las tres opciones; funciones-ocultas.md), para seguir
    // trabajando en eso.
    if std::env::args().any(|a| a == "--show-unfinished") {
        ui.set_show_unfinished(true);
    }
    if let Some((x, y)) = bench {
        ui.set_bench_mark(true);
        ui.window().set_position(slint::PhysicalPosition::new(x, y));
    }
    // `--at x,y`: solo la posición (capturas para revisar el diseño).
    if let Some((x, y)) = arg("--at").and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?))
    }) {
        ui.window().set_position(slint::PhysicalPosition::new(x, y));
    }
    ui.show()?;
    if bench.is_some() {
        println!("SHOWN");
    }
    slint::run_event_loop()
}
