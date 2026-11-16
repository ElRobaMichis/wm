//! Ventana de ajustes con controles nativos de Win32 (comctl32 v6): la
//! alternativa a Slint del spike 8 (architecture.md §13.10). Misma pantalla y
//! mismos datos (`wm_model::settings_demo`); textos de los mismos `.po`.
//!
//! Lo que trae el sistema sin escribirlo: accesibilidad (UI Automation),
//! IME, teclas muertas, alto contraste, mnemónicos y la navegación con Tab
//! (`IsDialogMessageW`). Lo que no: modo oscuro de los controles y el estilo
//! de Windows 11 (Fluent).

use std::cell::RefCell;
use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HFONT, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    EM_SETCUEBANNER, ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_AUTORADIOBUTTON, BS_DEFPUSHBUTTON, BS_GROUPBOX,
    BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST,
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EN_CHANGE,
    ES_AUTOHSCROLL, GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW, GetWindowTextW,
    IDCANCEL, IDOK, IsDialogMessageW, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL,
    LBN_SELCHANGE, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, MSG, PostQuitMessage, RegisterClassW, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_SETFONT, WM_SIZE, WNDCLASSW, WS_BORDER,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_GROUP, WS_OVERLAPPEDWINDOW,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{PCWSTR, w};
use wm_model::i18n::Catalog;
use wm_model::settings_demo::{self, APPS};

const ID_SEARCH: i32 = 100;
const ID_APPS_FIRST: i32 = 101;
const ID_WINDOWS_FIRST: i32 = 102;
const ID_LIST: i32 = 103;
const ID_COMBO: i32 = 104;
const ID_BORDER: i32 = 105;
const ID_MOUSE: i32 = 106;
const ID_ANIMATE: i32 = 107;

const PO_ES: &str = include_str!("../../../settings/lang/es/LC_MESSAGES/wm-settings.po");

/// Opciones de lanzamiento.
pub struct Options {
    pub lang: String,
    /// Posición fija y marca magenta para medir el arranque.
    pub bench: Option<(i32, i32)>,
}

struct Ui {
    hwnd: HWND,
    cat: Catalog,
    title: HWND,
    search_label: HWND,
    search: HWND,
    list_label: HWND,
    question: HWND,
    radios: [HWND; 2],
    list: HWND,
    combo_label: HWND,
    combo: HWND,
    group: HWND,
    checks: [HWND; 3],
    status: HWND,
    cancel: HWND,
    save: HWND,
    mark: Option<HWND>,
    font: HFONT,
    title_font: HFONT,
    mark_brush: HBRUSH,
    assigned: Vec<(char, Option<usize>)>,
    original: Vec<(char, Option<usize>)>,
    visible: Vec<usize>,
    current: usize,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: i32,
    ex: WINDOW_EX_STYLE,
    id: i32,
) -> HWND {
    let t = wide(text);
    // SAFETY: ventana hija propia; `t` vive durante la llamada.
    unsafe {
        CreateWindowExW(
            ex,
            class,
            PCWSTR(t.as_ptr()),
            WINDOW_STYLE(style as u32) | WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0,
            0,
            10,
            10,
            Some(parent),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU(id as isize as *mut c_void)),
            None,
            None,
        )
        .unwrap_or_default()
    }
}

fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: mensajes a controles propios del mismo hilo.
    unsafe { SendMessageW(h, msg, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

fn set_text(h: HWND, s: &str) {
    let t = wide(s);
    // SAFETY: control propio; `t` vive durante la llamada.
    let _ = unsafe { SetWindowTextW(h, PCWSTR(t.as_ptr())) };
}

fn font(dpi: u32, points: i32, weight: i32) -> HFONT {
    let height = -(points * dpi as i32 / 72);
    // SAFETY: crea una fuente propia (se libera al cambiar de DPI o al cerrar).
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            0,
            w!("Segoe UI"),
        )
    }
}

impl Ui {
    fn all(&self) -> Vec<HWND> {
        let mut v = vec![self.search_label, self.search, self.question, self.list_label, self.list];
        v.extend([self.combo_label, self.combo]);
        v.extend(self.radios);
        v.push(self.group);
        v.extend(self.checks);
        v.extend([self.status, self.cancel, self.save]);
        v
    }

    fn set_fonts(&mut self, dpi: u32) {
        // SAFETY: fuentes propias; se reemplazan y se libera la anterior.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.font.0));
            let _ = DeleteObject(HGDIOBJ(self.title_font.0));
        }
        self.font = font(dpi, 9, FW_NORMAL.0 as i32);
        self.title_font = font(dpi, 15, FW_SEMIBOLD.0 as i32);
        for h in self.all() {
            send(h, WM_SETFONT, self.font.0 as usize, 1);
        }
        send(self.title, WM_SETFONT, self.title_font.0 as usize, 1);
    }

    fn layout(&self) {
        // SAFETY: consultas y movimientos de ventanas propias.
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd).max(96) as i32;
            let s = |v: i32| v * dpi / 96;
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let (w, h) = (rc.right, rc.bottom);
            let m = s(16);
            let inner = w - 2 * m;
            let put = |hwnd: HWND, x: i32, y: i32, cx: i32, cy: i32| {
                let _ = SetWindowPos(hwnd, None, x, y, cx, cy, SWP_NOZORDER | SWP_NOACTIVATE);
            };
            let mut y = m;
            put(self.title, m, y, inner, s(32));
            y += s(40);
            put(self.search_label, m, y + s(5), s(60), s(20));
            put(self.search, m + s(64), y, inner - s(64), s(28));
            y += s(38);
            put(self.question, m, y, inner, s(20));
            y += s(22);
            put(self.radios[0], m, y, inner / 2, s(24));
            put(self.radios[1], m + inner / 2, y, inner / 2, s(24));
            y += s(30);
            put(self.list_label, m, y, inner, s(20));
            y += s(22);
            let buttons_y = h - m - s(30);
            put(self.status, m, buttons_y + s(6), inner - s(220), s(20));
            put(self.cancel, w - m - s(210), buttons_y, s(100), s(30));
            put(self.save, w - m - s(100), buttons_y, s(100), s(30));
            let group_h = s(24) + 3 * s(26) + s(12);
            let group_y = buttons_y - s(12) - group_h;
            put(self.group, m, group_y, inner, group_h);
            for (i, c) in self.checks.iter().enumerate() {
                put(*c, m + s(12), group_y + s(24) + i as i32 * s(26), inner - s(24), s(24));
            }
            let combo_y = group_y - s(12) - s(28);
            put(self.combo_label, m, combo_y + s(5), s(130), s(20));
            put(self.combo, m + s(134), combo_y, inner - s(134), s(400));
            put(self.list, m, y, inner, combo_y - s(12) - y);
            if let Some(mark) = self.mark {
                let _ = SetWindowPos(mark, None, 0, 0, 48, 64, SWP_NOACTIVATE);
            }
        }
    }

    fn refresh_list(&self) {
        send(self.list, LB_RESETCONTENT, 0, 0);
        let free = self.cat.tr("(free)").to_owned();
        for &i in &self.visible {
            let (l, a) = self.assigned[i];
            let app = a.map_or(free.as_str(), |a| APPS[a]);
            let t = wide(&format!("{l}  —  {app}"));
            send(self.list, LB_ADDSTRING, 0, t.as_ptr() as isize);
        }
        let sel = self.visible.iter().position(|&i| i == self.current).map_or(-1, |p| p as isize);
        send(self.list, LB_SETCURSEL, sel as usize, 0);
        self.refresh_selection();
    }

    fn refresh_selection(&self) {
        let (letter, app) = self.assigned[self.current];
        set_text(self.combo_label, &self.cat.tr("App for “{}”").replace("{}", &letter.to_string()));
        send(self.combo, CB_SETCURSEL, app.map_or(0, |a| a + 1), 0);
        let n = self.assigned.iter().filter(|(_, a)| a.is_some()).count() as i64;
        set_text(self.status, &self.cat.trn("{n} letter assigned", "{n} letters assigned", n));
        // SAFETY: control propio.
        let _ = unsafe { EnableWindow(self.save, self.assigned != self.original) };
    }

    fn command(&mut self, id: i32, code: u32) {
        match (id, code) {
            (ID_SEARCH, c) if c == EN_CHANGE => {
                let mut buf = [0u16; 128];
                // SAFETY: buffer local.
                let len = unsafe { GetWindowTextW(self.search, &mut buf) };
                let q = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
                self.visible = (0..26)
                    .filter(|&i| settings_demo::matches(self.assigned[i].0, self.assigned[i].1, &q))
                    .collect();
                if let Some(&first) = self.visible.first() {
                    self.current = first;
                }
                self.refresh_list();
            }
            (ID_LIST, c) if c == LBN_SELCHANGE => {
                let sel = send(self.list, LB_GETCURSEL, 0, 0);
                if let Some(&i) = self.visible.get(sel.max(0) as usize) {
                    self.current = i;
                    self.refresh_selection();
                }
            }
            (ID_COMBO, c) if c == CBN_SELCHANGE => {
                let choice = send(self.combo, CB_GETCURSEL, 0, 0);
                let cur = self.current;
                self.assigned[cur].1 = if choice <= 0 { None } else { Some(choice as usize - 1) };
                self.refresh_list();
            }
            (x, _) if x == IDOK.0 || x == IDCANCEL.0 => {
                // El spike no guarda (el formato llega con el core).
                // SAFETY: ventana propia.
                let _ = unsafe { DestroyWindow(self.hwnd) };
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            UI.with(|u| {
                if let Ok(mut g) = u.try_borrow_mut()
                    && let Some(ui) = g.as_mut()
                {
                    ui.command(id, code);
                }
            });
            LRESULT(0)
        }
        WM_SIZE => {
            UI.with(|u| {
                if let Ok(g) = u.try_borrow()
                    && let Some(ui) = g.as_ref()
                {
                    ui.layout();
                }
            });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: lparam apunta al rectángulo sugerido (contrato de WM_DPICHANGED).
            let r = unsafe { &*(lparam.0 as *const RECT) };
            let dpi = (wparam.0 & 0xFFFF) as u32;
            UI.with(|u| {
                if let Ok(mut g) = u.try_borrow_mut()
                    && let Some(ui) = g.as_mut()
                {
                    ui.set_fonts(dpi);
                }
            });
            // SAFETY: ventana propia.
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            LRESULT(0)
        }
        WM_CTLCOLORSTATIC => {
            let brush = UI.with(|u| {
                u.try_borrow().ok().and_then(|g| {
                    g.as_ref().and_then(|ui| {
                        let is_mark = ui.mark.map(|m| m.0) == Some(lparam.0 as *mut c_void);
                        is_mark.then_some(ui.mark_brush)
                    })
                })
            });
            match brush {
                Some(b) => LRESULT(b.0 as isize),
                // SAFETY: procesamiento por defecto.
                None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_CLOSE => {
            // SAFETY: ventana propia.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: sin precondiciones.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: procesamiento por defecto.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Abre la ventana y bombea mensajes hasta que se cierre.
pub fn run(opts: Options) -> windows::core::Result<()> {
    let cat = if opts.lang == "en" { Catalog::default() } else { Catalog::parse(PO_ES) };
    // SAFETY: inicialización estándar de comctl32 v6 (el manifiesto la declara).
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&icc);
    }
    // SAFETY: registro y creación de la ventana principal propia.
    let hwnd = unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: w!("wm-settings-native"),
            hbrBackground: HBRUSH(
                (windows::Win32::Graphics::Gdi::COLOR_BTNFACE.0 + 1) as isize as *mut c_void,
            ),
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                None,
                windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
            )?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let t = wide(cat.tr("wm settings"));
        let (x, y) = opts.bench.unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("wm-settings-native"),
            PCWSTR(t.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            560,
            760,
            None,
            None,
            Some(instance),
            None,
        )?
    };
    // SAFETY: consulta de solo lectura.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let s = |v: i32| v * dpi as i32 / 96;
    // Tamaño en DIP, escalado al DPI del monitor donde aparece.
    let no_move = windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE | SWP_NOZORDER;
    // SAFETY: ventana propia.
    let _ = unsafe { SetWindowPos(hwnd, None, 0, 0, s(560), s(760), no_move) };

    // Estilos repetidos.
    let none = WINDOW_EX_STYLE(0);
    let stop = WS_TABSTOP.0 as i32;
    let first = (WS_TABSTOP | WS_GROUP).0 as i32;
    let label = |text: &str| child(hwnd, w!("STATIC"), text, 0, none, 0);
    let button = |text: &str, style: i32, id: i32| child(hwnd, w!("BUTTON"), text, style, none, id);

    let mark = opts.bench.map(|_| label(""));
    // Orden de creación = orden de tabulación, y cada etiqueta justo antes de
    // su control: UI Automation toma el nombre de la etiqueta anterior.
    let title = label(cat.tr("Apps and keys"));
    let search_label = label(cat.tr("Search"));
    let search = child(hwnd, w!("EDIT"), "", ES_AUTOHSCROLL | first, WS_EX_CLIENTEDGE, ID_SEARCH);
    let question = label(cat.tr("What do you want closest at hand?"));
    let radio0 = button(cat.tr("Apps first"), BS_AUTORADIOBUTTON | first, ID_APPS_FIRST);
    let radio1 = button(cat.tr("Windows first"), BS_AUTORADIOBUTTON, ID_WINDOWS_FIRST);
    let list_label = label(cat.tr("Letters"));
    let list_style = LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | (WS_VSCROLL | WS_BORDER).0 as i32 | first;
    let list = child(hwnd, w!("LISTBOX"), "", list_style, none, ID_LIST);
    let combo_label = label("");
    let combo_style = CBS_DROPDOWNLIST | WS_VSCROLL.0 as i32 | first;
    let combo = child(hwnd, w!("COMBOBOX"), "", combo_style, none, ID_COMBO);
    let group = button(cat.tr("Options"), BS_GROUPBOX, 0);
    let checks = [
        button(cat.tr("Focus border"), BS_AUTOCHECKBOX | first, ID_BORDER),
        button(cat.tr("Mouse follows focus"), BS_AUTOCHECKBOX | stop, ID_MOUSE),
        button(cat.tr("Animate windows"), BS_AUTOCHECKBOX | stop, ID_ANIMATE),
    ];
    let status = label("");
    let cancel = button(cat.tr("Cancel"), BS_PUSHBUTTON | first, IDCANCEL.0);
    let save = button(cat.tr("Save"), BS_DEFPUSHBUTTON | stop, IDOK.0);
    let ui = Ui {
        hwnd,
        title,
        search_label,
        search,
        question,
        radios: [radio0, radio1],
        list_label,
        list,
        combo_label,
        combo,
        group,
        checks,
        status,
        cancel,
        save,
        mark,
        font: HFONT::default(),
        title_font: HFONT::default(),
        // SAFETY: pincel propio (magenta), vive lo que la ventana.
        mark_brush: unsafe { CreateSolidBrush(COLORREF(0x00FF_00FF)) },
        assigned: settings_demo::initial(),
        original: settings_demo::initial(),
        visible: (0..26).collect(),
        current: 0,
        cat,
    };
    let cue = wide(ui.cat.tr("Search an app or a letter…"));
    send(ui.search, EM_SETCUEBANNER, 1, cue.as_ptr() as isize);
    let none = wide(ui.cat.tr("(none)"));
    send(ui.combo, CB_ADDSTRING, 0, none.as_ptr() as isize);
    for a in APPS {
        let t = wide(a);
        send(ui.combo, CB_ADDSTRING, 0, t.as_ptr() as isize);
    }
    send(ui.radios[0], BM_SETCHECK, 1, 0);
    send(ui.checks[0], BM_SETCHECK, 1, 0);
    send(ui.checks[2], BM_SETCHECK, 1, 0);
    let _ = send(ui.checks[1], BM_GETCHECK, 0, 0);

    let mut ui = ui;
    ui.set_fonts(dpi);
    ui.refresh_list();
    ui.layout();
    // El recuadro "Opciones" va debajo de sus casillas (si no, las tapa).
    // SAFETY: control propio.
    let _ = unsafe {
        SetWindowPos(
            ui.group,
            Some(windows::Win32::UI::WindowsAndMessaging::HWND_BOTTOM),
            0,
            0,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOSIZE
                | SWP_NOACTIVATE,
        )
    };
    // La lista se llenó con 10 px de alto: volver arriba de todo.
    send(ui.list, windows::Win32::UI::WindowsAndMessaging::LB_SETTOPINDEX, 0, 0);
    if let Some(m) = ui.mark {
        // SAFETY: control propio; la marca queda encima de sus hermanos.
        let _ = unsafe {
            let top = windows::Win32::UI::WindowsAndMessaging::HWND_TOP;
            SetWindowPos(m, Some(top), 0, 0, 48, 64, SWP_NOACTIVATE)
        };
    }
    let search = ui.search;
    UI.with(|u| *u.borrow_mut() = Some(ui));
    // SAFETY: ventana propia.
    unsafe {
        let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 1);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(search));
        let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
    }
    if opts.bench.is_some() {
        println!("SHOWN");
    }
    let mut msg = MSG::default();
    // SAFETY: bucle de mensajes estándar; IsDialogMessageW da Tab, Enter y Esc.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if IsDialogMessageW(hwnd, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    }
    UI.with(|u| {
        if let Some(ui) = u.borrow_mut().take() {
            // SAFETY: objetos GDI propios.
            unsafe {
                let _ = DeleteObject(HGDIOBJ(ui.font.0));
                let _ = DeleteObject(HGDIOBJ(ui.title_font.0));
                let _ = DeleteObject(HGDIOBJ(ui.mark_brush.0));
            }
        }
    });
    Ok(())
}
