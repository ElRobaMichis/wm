//! `wm-settings` — ventana de ajustes en Slint (spike 8, architecture.md
//! §13.10). Binario aparte: Slint importa `dwrite`, `uiautomationcore`,
//! `comctl32`... en forma estática, y `wm.exe` (guardián y core) no debe
//! cargarlas.
//!
//! Misma pantalla y mismos datos (`wm_model::settings_demo`) que la versión
//! nativa (`wm settings`), para comparar solo el toolkit. Opciones:
//! - `--lang es|en`: idioma (por defecto, español);
//! - `--bench x,y`: ventana en esa posición, con una marca magenta arriba a
//!   la izquierda, y `SHOWN` por stdout (lo usa `wm-bench settings`);
//! - `--at x,y`: solo la posición; `--light` / `--dark`: forzar el tema.

#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};
use wm_model::settings_demo::{self, APP_NAMES, APPS};

slint::include_modules!();

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

struct State {
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
                name: a.map_or_else(SharedString::new, |a| SharedString::from(APP_NAMES[a])),
                exe: a.map_or_else(SharedString::new, |a| format!("{}.exe", APPS[a]).into()),
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
    let mut choices = vec![ui.get_none_text()];
    choices.extend(APP_NAMES.iter().map(|a| SharedString::from(*a)));
    ui.set_apps(ModelRc::new(VecModel::from(choices)));

    let initial = settings_demo::initial();
    let st = Rc::new(RefCell::new(State {
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
                settings_demo::matches(l, a, &q)
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
    ui.on_save(move || {
        // El spike no guarda: el formato de configuración llega con el core.
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
    });
    let weak = ui.as_weak();
    ui.on_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
    });

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
