//! `wm-bench uia <exe> [args…]` — spike 8 (architecture.md §13.10): lo que
//! "ve" el Narrador. Usa la UI Automation **nativa** (`IUIAutomation`, la
//! misma que el Narrador); la de .NET de PowerShell 5.1 no carga los
//! adaptadores de los controles Win32 y los muestra todos como "Pane".
//!
//! Lanza la ventana, vuelca su árbol (tipo, nombre, foco de teclado) y la
//! maneja como un lector de pantalla: escribe "brave" en la búsqueda
//! (ValuePattern) y activa la primera opción con TogglePattern.

use std::ffi::c_void;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTogglePattern,
    IUIAutomationValuePattern, TreeScope_Descendants, UIA_ButtonControlTypeId,
    UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_EditControlTypeId,
    UIA_ListControlTypeId, UIA_ListItemControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_TogglePatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::{BOOL, BSTR};

fn main_window(pid: u32) -> Option<HWND> {
    struct Find(u32, Option<HWND>);
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        // SAFETY: `l` es el `&mut Find` de abajo.
        let f = unsafe { &mut *(l.0 as *mut Find) };
        let mut p = 0u32;
        // SAFETY: consultas de solo lectura.
        unsafe {
            GetWindowThreadProcessId(h, Some(&mut p));
            if p == f.0 && IsWindowVisible(h).as_bool() && GetWindowTextLengthW(h) > 0 {
                f.1 = Some(h);
                return false.into();
            }
        }
        true.into()
    }
    let mut f = Find(pid, None);
    // SAFETY: el callback solo usa `l` como el `&mut Find`.
    let _ = unsafe { EnumWindows(Some(cb), LPARAM(&mut f as *mut Find as isize)) };
    f.1
}

fn type_name(id: i32) -> &'static str {
    match id {
        x if x == UIA_ButtonControlTypeId.0 => "Botón",
        x if x == UIA_CheckBoxControlTypeId.0 => "Casilla",
        x if x == UIA_RadioButtonControlTypeId.0 => "Opción",
        x if x == UIA_EditControlTypeId.0 => "Edición",
        x if x == UIA_ComboBoxControlTypeId.0 => "Desplegable",
        x if x == UIA_ListControlTypeId.0 => "Lista",
        x if x == UIA_ListItemControlTypeId.0 => "Ítem",
        50020 => "Texto",
        50026 => "Grupo",
        50033 => "Panel",
        50032 => "Ventana",
        50037 => "Barra de título",
        50010 => "Barra de menú",
        50011 => "Menú",
        50014 => "Barra de desplazamiento",
        _ => "otro",
    }
}

const INTERACTIVE: [&str; 7] =
    ["Botón", "Casilla", "Opción", "Edición", "Desplegable", "Lista", "Ítem"];

pub fn run(args: &[String]) -> ExitCode {
    let Some(exe) = args.first() else {
        eprintln!("Uso: wm-bench uia <exe> [args…]");
        return ExitCode::from(2);
    };
    let Ok(mut child) = Command::new(exe).args(&args[1..]).stdout(Stdio::null()).spawn() else {
        eprintln!("No se pudo lanzar {exe}");
        return ExitCode::FAILURE;
    };
    let mut hwnd = None;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        hwnd = main_window(child.id());
        if hwnd.is_some() {
            break;
        }
    }
    std::thread::sleep(Duration::from_millis(800));
    let result = hwnd.map(dump);
    let _ = child.kill();
    let _ = child.wait();
    match result {
        Some(Ok(())) => ExitCode::SUCCESS,
        Some(Err(e)) => {
            eprintln!("UI Automation: {e}");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("No apareció la ventana.");
            ExitCode::FAILURE
        }
    }
}

struct Node {
    el: IUIAutomationElement,
    kind: &'static str,
    name: String,
    focusable: bool,
    enabled: bool,
}

fn nodes(uia: &IUIAutomation, root: &IUIAutomationElement) -> windows::core::Result<Vec<Node>> {
    // SAFETY: llamadas COM estándar sobre interfaces válidas.
    unsafe {
        let all = root.FindAll(TreeScope_Descendants, &uia.CreateTrueCondition()?)?;
        let mut out = Vec::new();
        for i in 0..all.Length()? {
            let el = all.GetElement(i)?;
            let kind = type_name(el.CurrentControlType()?.0);
            let name = el.CurrentName().unwrap_or_else(|_| BSTR::new()).to_string();
            let focusable = el.CurrentIsKeyboardFocusable()?.as_bool();
            let enabled = el.CurrentIsEnabled()?.as_bool();
            out.push(Node { el, kind, name, focusable, enabled });
        }
        Ok(out)
    }
}

fn dump(hwnd: HWND) -> windows::core::Result<()> {
    // SAFETY: inicialización COM y llamadas UIA estándar.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let root = uia.ElementFromHandle(hwnd)?;
        println!("Ventana «{}»", root.CurrentName()?);
        let list = nodes(&uia, &root)?;
        println!("{} elementos en el árbol", list.len());
        for n in list.iter().filter(|n| INTERACTIVE.contains(&n.kind) && n.kind != "Ítem") {
            println!(
                "  {:<12} «{}»{}{}",
                n.kind,
                n.name,
                if n.focusable { "" } else { "  [sin foco de teclado]" },
                if n.enabled { "" } else { "  [deshabilitado]" }
            );
        }
        let items: Vec<&Node> = list.iter().filter(|n| n.kind == "Ítem").collect();
        let first: Vec<&str> = items.iter().take(3).map(|n| n.name.as_str()).collect();
        println!("  ítems de lista: {} (primeros: {})", items.len(), first.join(" | "));
        let unnamed: Vec<&str> = list
            .iter()
            .filter(|n| INTERACTIVE.contains(&n.kind) && n.kind != "Lista")
            .filter(|n| n.name.trim().is_empty())
            .map(|n| n.kind)
            .collect();
        println!("  controles interactivos sin nombre: {} {unnamed:?}", unnamed.len());
        let others = list.iter().filter(|n| n.kind == "otro" || n.kind == "Panel").count();
        println!("  elementos genéricos (panel/otro): {others}");

        // Como un lector de pantalla: escribir en la búsqueda.
        if let Some(edit) = list.iter().find(|n| n.kind == "Edición") {
            match edit.el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                Ok(v) => {
                    v.SetValue(&BSTR::from("brave"))?;
                    std::thread::sleep(Duration::from_millis(500));
                    let after = nodes(&uia, &root)?;
                    let hits: Vec<String> =
                        after.iter().filter(|n| n.kind == "Ítem").map(|n| n.name.clone()).collect();
                    let n = hits.len();
                    println!("  búsqueda «brave» por UIA → {n} ítem(s): {}", hits.join(" | "));
                }
                Err(e) => println!("  búsqueda por UIA: sin ValuePattern ({e})"),
            }
        }
        // Activar la primera opción con TogglePattern.
        let toggle = list.iter().find_map(|n| {
            n.el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                .ok()
                .map(|t| (n, t))
        });
        match toggle {
            Some((n, t)) => {
                let before = t.CurrentToggleState()?.0;
                t.Toggle()?;
                std::thread::sleep(Duration::from_millis(300));
                let after = t.CurrentToggleState()?.0;
                println!("  Toggle por UIA en «{}» ({}): {before} → {after}", n.name, n.kind);
            }
            None => println!("  ningún control con TogglePattern"),
        }
        let _: *const c_void = std::ptr::null();
        tab_walk(&uia, hwnd)?;
        Ok(())
    }
}

/// Recorre la ventana con Tab (teclas reales) y anota dónde queda el foco:
/// muestra qué controles se alcanzan con el teclado y en qué orden.
fn tab_walk(uia: &IUIAutomation, hwnd: HWND) -> windows::core::Result<()> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VK_TAB,
    };
    use wm_platform::windows_list::{WindowId, activate};
    let _ = activate(WindowId(hwnd.0 as isize));
    std::thread::sleep(Duration::from_millis(300));
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_TAB,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..16 {
        // SAFETY: estructuras locales; la ventana de prueba está al frente.
        unsafe { SendInput(&[key(false), key(true)], size_of::<INPUT>() as i32) };
        std::thread::sleep(Duration::from_millis(150));
        // SAFETY: llamada UIA estándar.
        let label = unsafe {
            match uia.GetFocusedElement() {
                Ok(e) => format!(
                    "{} «{}»",
                    type_name(e.CurrentControlType()?.0),
                    e.CurrentName().unwrap_or_default()
                ),
                Err(_) => "?".to_owned(),
            }
        };
        seen.push(label);
    }
    println!("  Tab ×16 → {}", seen.join(" → "));
    let mut distinct = seen.clone();
    distinct.sort();
    distinct.dedup();
    println!("  controles alcanzados con Tab: {}", distinct.len());
    Ok(())
}
