//! `wm-settings` — ventana de ajustes en Slint (spike 8, architecture.md
//! §13.10). Binario aparte: Slint importa `dwrite`, `uiautomationcore`,
//! `comctl32`... en forma estática, y `wm.exe` (guardián y core) no debe
//! cargarlas.
//!
//! Las asignaciones son las de verdad: se leen de `%APPDATA%\wm\apps.txt`
//! (el archivo que usa el core) y Guardar lo reescribe en forma atómica,
//! conservando comentarios y rutas (`wm_model::apps::update_bindings`). Las
//! apps para elegir (plan-apps.md §4): las del archivo, las **abiertas
//! ahora** y **todas** las del menú Inicio (`wm_platform::catalog`, en un
//! hilo: la ventana abre al instante). Opciones:
//! - `--lang es|en`: idioma (por defecto, español);
//! - `--bench x,y`: ventana en esa posición, con una marca magenta arriba a
//!   la izquierda, y `SHOWN` por stdout (lo usa `wm-bench settings`);
//! - `--at x,y`: solo la posición; `--light` / `--dark`: forzar el tema;
//! - `--show-unfinished`: muestra el perfil y las opciones escondidas en la
//!   primera versión (`funciones-ocultas.md`).

#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use slint::{Model, ModelRc, SharedString, VecModel};
use wm_model::apps::{AppBinding, bindings_path, parse_bindings, update_bindings};
use wm_model::mapview::display_name;
use wm_model::options::{Options, options_path};
use wm_model::rules::OpenRule;

slint::include_modules!();

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Una app que se puede elegir.
#[derive(Debug, Clone)]
struct Entry {
    /// Nombre visible.
    name: String,
    /// Ejecutables (el primero es el principal).
    names: Vec<String>,
    /// Cómo abrirla.
    launch: Option<String>,
    /// Tiene una ventana abierta ahora.
    open: bool,
}

/// Las apps que se pueden elegir. Solo se agregan al final (las letras
/// guardan índices).
struct Catalog {
    entries: Vec<Entry>,
}

/// Para ordenar y buscar sin mayúsculas ni acentos.
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            c => c,
        })
        .collect()
}

impl Catalog {
    /// Las del archivo y las abiertas ahora.
    fn new(bindings: &[AppBinding]) -> Self {
        let mut c = Self { entries: Vec::new() };
        for b in bindings {
            c.add(Entry {
                name: b.label.clone().unwrap_or_else(|| display_name(&b.app)),
                names: b.names().map(str::to_owned).collect(),
                launch: b.launch.clone(),
                open: false,
            });
        }
        for w in wm_platform::windows_list::user_windows() {
            let mut names = Vec::new();
            if let Some(f) = &w.family {
                names.push(f.clone());
            }
            names.push(w.app.clone());
            let launch = match &w.family {
                Some(f) => wm_platform::process_info::running_exe(std::slice::from_ref(f)),
                None => wm_platform::process_info::info(w.pid).map(|i| i.path),
            };
            c.add(Entry { name: display_name(&names[0]), names, launch, open: true });
        }
        c
    }

    /// Agrega `e`, o la junta con una que comparte un ejecutable (completa
    /// lo que le falte).
    fn add(&mut self, e: Entry) {
        if let Some(old) =
            self.entries.iter_mut().find(|o| o.names.iter().any(|n| e.names.contains(n)))
        {
            old.open |= e.open;
            for n in e.names {
                if !old.names.contains(&n) {
                    old.names.push(n);
                }
            }
            if old.launch.is_none() {
                old.launch = e.launch;
            }
            return;
        }
        self.entries.push(e);
    }

    fn index(&self, app: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.names.iter().any(|n| n == app))
    }

    /// ¿La fila coincide con la búsqueda (letra exacta o parte del nombre)?
    fn matches(&self, letter: char, app: Option<usize>, query: &str) -> bool {
        let q = fold(query.trim());
        if q.is_empty() {
            return true;
        }
        let letter_hit = q.len() == 1 && q.starts_with(letter.to_ascii_lowercase());
        letter_hit || app.is_some_and(|a| self.entry_matches(a, &q))
    }

    fn entry_matches(&self, i: usize, q: &str) -> bool {
        let e = &self.entries[i];
        fold(&e.name).contains(q) || e.names.iter().any(|n| n.contains(q))
    }

    /// Las filas del selector con la búsqueda `query`: "Ninguna", las
    /// abiertas y todas (A→Z). Devuelve las filas y, por fila, la app (o
    /// `None` para "Ninguna" y los títulos).
    fn rows(&self, query: &str, none: &str, open: &str, all: &str) -> (Vec<PickerRow>, Vec<Row>) {
        let q = fold(query.trim());
        let mut rows = Vec::new();
        let mut map = Vec::new();
        if q.is_empty() || fold(none).contains(&q) {
            rows.push(PickerRow { kind: 2, name: none.into(), detail: SharedString::new() });
            map.push(Row::None);
        }
        let mut order: Vec<usize> = (0..self.entries.len())
            .filter(|&i| q.is_empty() || self.entry_matches(i, &q))
            .collect();
        order.sort_by_key(|&i| fold(&self.entries[i].name));
        for (title, want_open) in [(open, true), (all, false)] {
            let part: Vec<usize> =
                order.iter().copied().filter(|&i| self.entries[i].open == want_open).collect();
            if part.is_empty() {
                continue;
            }
            rows.push(PickerRow { kind: 0, name: title.into(), detail: SharedString::new() });
            map.push(Row::Title);
            for i in part {
                let e = &self.entries[i];
                rows.push(PickerRow {
                    kind: 1,
                    name: e.name.as_str().into(),
                    detail: format!("{}.exe", e.names[0]).into(),
                });
                map.push(Row::App(i));
            }
        }
        (rows, map)
    }
}

/// Qué es cada fila del selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    None,
    Title,
    App(usize),
}

struct State {
    catalog: Catalog,
    /// Texto del archivo al abrir: Guardar escribe sobre él.
    file_text: String,
    /// Filas del selector abierto (qué es cada una).
    picker: Vec<Row>,
    assigned: Vec<(char, Option<usize>)>,
    original: Vec<(char, Option<usize>)>,
    /// Índices (en `assigned`) de las filas visibles con la búsqueda actual.
    visible: Vec<usize>,
    /// Letra elegida (índice en `assigned`).
    current: usize,
    query: String,
    /// Opciones al abrir (`settings.txt`).
    options: Options,
    /// v1.1: la regla "al abrir" de cada letra (A–Z) y la del archivo.
    rules: Vec<Option<OpenRule>>,
    original_rules: Vec<Option<OpenRule>>,
    /// Campos que esta versión no entiende, por letra: se conservan.
    extras: Vec<Vec<String>>,
}

/// Las reglas que ofrece el desplegable "Al abrir" (plan-v1.1.md §7: cinco;
/// el resto solo a mano en `apps.txt`).
const OPEN_CHOICES: [Option<OpenRule>; 5] = [
    None,
    Some(OpenRule::Place(wm_model::Placement::LeftHalf)),
    Some(OpenRule::Place(wm_model::Placement::RightHalf)),
    Some(OpenRule::Maximized),
    Some(OpenRule::Centered(80)),
];

/// Las opciones del desplegable para `rule` y cuál está elegida: si la regla
/// no es una de las cinco (una esquina, escrita a mano), va como sexta.
fn open_options(ui: &SettingsWindow, rule: Option<OpenRule>) -> (Vec<SharedString>, i32) {
    let labels = ui.get_open_labels();
    let mut options: Vec<SharedString> = (0..5).filter_map(|i| labels.row_data(i)).collect();
    let i = match OPEN_CHOICES.iter().position(|c| *c == rule) {
        Some(i) => i as i32,
        None => {
            options.push(rule.map_or_else(SharedString::new, |r| r.format().into()));
            5
        }
    };
    (options, i)
}

fn refresh(ui: &SettingsWindow, st: &State) {
    let items: Vec<LetterItem> = st
        .visible
        .iter()
        .map(|&i| {
            let (l, a) = st.assigned[i];
            LetterItem {
                letter: SharedString::from(l.to_string()),
                name: a
                    .map_or_else(SharedString::new, |a| st.catalog.entries[a].name.as_str().into()),
                exe: a.map_or_else(SharedString::new, |a| {
                    let rule =
                        st.rules[i].map_or_else(String::new, |r| format!(" · {}", r.short()));
                    format!("{}.exe{rule}", st.catalog.entries[a].names[0]).into()
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
    ui.set_choice_name(
        app.map_or_else(|| ui.get_none_text(), |a| st.catalog.entries[a].name.as_str().into()),
    );
    ui.set_assigned(st.assigned.iter().filter(|(_, a)| a.is_some()).count() as i32);
    let rule = st.rules[st.current];
    let (options, chosen) = open_options(ui, rule);
    ui.set_open_options(ModelRc::new(VecModel::from(options)));
    ui.set_open_choice(chosen);
    ui.set_choice_assigned(app.is_some());
    ui.set_dirty(
        st.assigned != st.original || st.rules != st.original_rules || options_of(ui) != st.options,
    );
}

fn options_of(ui: &SettingsWindow) -> Options {
    Options {
        animate: ui.get_animations(),
        mouse_follows_focus: ui.get_mouse_follows(),
        focus_border: ui.get_focus_border(),
        notices: ui.get_notices(),
    }
}

/// Escribe `settings.txt` (temporal + renombrar, como `apps.txt`).
fn save_options(o: &Options) -> std::io::Result<()> {
    let Some(path) = options_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, o.format())?;
    std::fs::rename(&tmp, path)
}

/// Escribe las asignaciones en `path`: primero a un temporal y después lo
/// renombra encima, así nunca queda un archivo a medias.
fn save(path: &Path, st: &State) -> std::io::Result<()> {
    let bindings: Vec<AppBinding> = st
        .assigned
        .iter()
        .filter_map(|&(letter, app)| {
            let e = &st.catalog.entries[app?];
            let (app, aliases) = e.names.split_first()?;
            // El nombre visible solo si no es el que ya se deduce del exe.
            let label = (e.name != display_name(app)).then(|| e.name.clone());
            let i = usize::from(letter as u8 - b'A');
            Some(AppBinding {
                letter: letter as u8,
                app: app.clone(),
                aliases: aliases.to_vec(),
                launch: e.launch.clone(),
                label,
                open: st.rules[i],
                extra: st.extras[i].clone(),
            })
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

thread_local! {
    /// El estado, para el resultado del hilo del catálogo (que vuelve al
    /// hilo de la interfaz con `invoke_from_event_loop`).
    static STATE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

/// Arma las filas del selector con la búsqueda `query`.
fn picker_rows(ui: &SettingsWindow, st: &mut State, query: &str) {
    let (rows, map) =
        st.catalog.rows(query, &ui.get_none_text(), &ui.get_open_text(), &ui.get_all_text());
    st.picker = map;
    ui.set_picker_rows(ModelRc::new(VecModel::from(rows)));
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
    let rules: Vec<Option<OpenRule>> = (b'A'..=b'Z')
        .map(|l| bindings.iter().find(|b| b.letter == l).and_then(|b| b.open))
        .collect();
    let extras: Vec<Vec<String>> = (b'A'..=b'Z')
        .map(|l| {
            bindings.iter().find(|b| b.letter == l).map(|b| b.extra.clone()).unwrap_or_default()
        })
        .collect();
    let initial: Vec<(char, Option<usize>)> = (b'A'..=b'Z')
        .map(|l| {
            let app = bindings.iter().find(|b| b.letter == l).and_then(|b| catalog.index(&b.app));
            (l as char, app)
        })
        .collect();

    let options = options_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| Options::parse(&t))
        .unwrap_or_default();
    ui.set_animations(options.animate);
    ui.set_mouse_follows(options.mouse_follows_focus);
    ui.set_focus_border(options.focus_border);
    ui.set_notices(options.notices);
    let st = Rc::new(RefCell::new(State {
        catalog,
        file_text,
        picker: Vec::new(),
        assigned: initial.clone(),
        original: initial,
        visible: (0..26).collect(),
        current: 0,
        query: String::new(),
        options,
        original_rules: rules.clone(),
        rules,
        extras,
    }));
    refresh(&ui, &st.borrow());

    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_options_changed(move || {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, &s.borrow());
        }
    });

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
            ui.set_choice_name(app.map_or_else(
                || ui.get_none_text(),
                |a| st.catalog.entries[a].name.as_str().into(),
            ));
            // La fila "Al abrir" de esta letra (v1.1): antes se quedaba con la
            // de la letra de cuando se abrió la ventana.
            let (options, chosen) = open_options(&ui, st.rules[i]);
            ui.set_open_options(ModelRc::new(VecModel::from(options)));
            ui.set_open_choice(chosen);
            ui.set_choice_assigned(app.is_some());
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
    // Selector de apps.
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_picker_open(move || {
        if let Some(ui) = weak.upgrade() {
            picker_rows(&ui, &mut s.borrow_mut(), "");
        }
    });
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_picker_search(move |q| {
        if let Some(ui) = weak.upgrade() {
            picker_rows(&ui, &mut s.borrow_mut(), &q);
        }
    });
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_picker_choose(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        let cur = st.current;
        let before = st.assigned[cur].1;
        match st.picker.get(i.max(0) as usize).copied() {
            Some(Row::None) => st.assigned[cur].1 = None,
            Some(Row::App(a)) => st.assigned[cur].1 = Some(a),
            _ => return,
        }
        // Otra app en la letra: la regla era de la anterior.
        if st.assigned[cur].1 != before {
            st.rules[cur] = None;
        }
        refresh(&ui, &st);
    });
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_open_chosen(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        let cur = st.current;
        if let Some(&rule) = OPEN_CHOICES.get(i.max(0) as usize) {
            st.rules[cur] = rule;
        }
        refresh(&ui, &st);
    });
    let weak = ui.as_weak();
    let s = st.clone();
    ui.on_picker_enter(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = s.borrow_mut();
        let cur = st.current;
        if let Some(a) = st.picker.iter().find_map(|r| match r {
            Row::App(a) => Some(*a),
            _ => None,
        }) {
            st.assigned[cur].1 = Some(a);
        }
        refresh(&ui, &st);
    });

    // Todas las apps del menú Inicio, en un hilo: la ventana ya está.
    ui.set_catalog_loading(true);
    STATE.with(|cell| *cell.borrow_mut() = Some(st.clone()));
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let found = wm_platform::catalog::installed().unwrap_or_default();
        let choices: Vec<wm_platform::catalog::Choice> =
            found.iter().filter_map(wm_platform::catalog::choice).collect();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_catalog_loading(false);
            STATE.with(|cell| {
                let Some(st) = cell.borrow().clone() else { return };
                let mut st = st.borrow_mut();
                for c in choices {
                    st.catalog.add(Entry {
                        name: c.label,
                        names: c.names,
                        launch: c.launch,
                        open: false,
                    });
                }
                if ui.get_picking() {
                    let q = ui.get_picker_query().to_string();
                    picker_rows(&ui, &mut st, &q);
                }
                refresh(&ui, &st);
            });
        });
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
        let opts = options_of(&ui);
        if opts != s.borrow().options
            && let Err(e) = save_options(&opts)
        {
            eprintln!("No se pudieron guardar las opciones: {e}");
            return;
        }
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
