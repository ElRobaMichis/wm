//! Ventanas "títere" para pruebas (architecture.md §10): ventanas comunes de
//! otro proceso, de colores y numeradas, para probar esconder y mostrar sin
//! tocar las apps reales del usuario.
//!
//! El títere "colgable" (spike 4) vive solo en su propio proceso y deja de
//! atender mensajes cuando se le pide, como una app colgada de verdad.

use std::mem::size_of;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject,
    DrawTextW, EndPaint, FillRect, GetMonitorInfoW, HGDIOBJ, MONITOR_DEFAULTTOPRIMARY,
    MONITORINFO, MonitorFromWindow, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows,
    GWLP_USERDATA, GetClassNameW, GetClientRect, GetMessageW, GetWindowLongPtrW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SetWindowLongPtrW, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_DESTROY, WM_PAINT, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::windows_list::WindowId;

const CLASS: PCWSTR = w!("wm-puppet");

/// Índice del títere colgable (los comunes van de 0 a 19).
pub const HANG_INDEX: isize = 99;

/// Pedido al títere colgable: colgarse `wparam` ms. `lparam` = 0 dormido
/// (no atiende mensajes), 1 ocupado (CPU al 100 % en un núcleo).
const WM_PUPPET_HANG: u32 = WM_APP + 0x10;

/// Color del títere `n` (desde 1) como (azul, verde, rojo).
pub fn color_bgr(n: usize) -> (u8, u8, u8) {
    let c = COLORS[(n.max(1) - 1) % COLORS.len()];
    ((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Colores bien distintos para reconocer cada títere a simple vista.
const COLORS: [u32; 10] = [
    0x003C_14DC, 0x0000_A5FF, 0x0000_D7FF, 0x0032_CD32, 0x00D1_CE00, 0x00E1_6941, 0x00D3_00C7,
    0x0013_458B, 0x0080_8080, 0x0099_3366,
];

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            // SAFETY: pintado estándar dentro de WM_PAINT con objetos propios
            // que se liberan antes de salir.
            unsafe {
                let index = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(COLORREF(COLORS[index % COLORS.len()]));
                FillRect(hdc, &rect, brush);
                let font = CreateFontW(
                    160, 0, 0, 0, 700, 0, 0, 0, Default::default(), Default::default(),
                    Default::default(), Default::default(), 0, w!("Segoe UI"),
                );
                let old = SelectObject(hdc, HGDIOBJ(font.0));
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00FF_FFFF));
                let mut text: Vec<u16> = (index + 1).to_string().encode_utf16().collect();
                DrawTextW(hdc, &mut text, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                SelectObject(hdc, old);
                let _ = DeleteObject(HGDIOBJ(font.0));
                let _ = DeleteObject(HGDIOBJ(brush.0));
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_PUPPET_HANG => {
            let duration = std::time::Duration::from_millis(wparam.0 as u64);
            if lparam.0 == 0 {
                std::thread::sleep(duration);
            } else {
                let start = std::time::Instant::now();
                while start.elapsed() < duration {
                    std::hint::spin_loop();
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // Cerrar cualquier títere termina el proceso de títeres.
            // SAFETY: sin precondiciones.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: procesamiento por defecto.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Abre `count` ventanas títere en grilla sobre el monitor principal y
/// atiende sus mensajes hasta que se cierre alguna. Bloquea.
pub fn run_puppets(count: usize) -> windows::core::Result<()> {
    // SAFETY: registro de clase y creación de ventanas propias; la clase y
    // el procedimiento viven mientras dura el proceso.
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);

        let monitor = MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info =
            MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let work = info.rcWork;
        let cols = 5.min(count.max(1)) as i32;
        let rows = count.div_ceil(cols as usize).max(1) as i32;
        let width = (work.right - work.left) / cols;
        let height = (work.bottom - work.top) / rows;

        for i in 0..count {
            let (col, row) = ((i as i32) % cols, (i as i32) / cols);
            let title: Vec<u16> =
                format!("Títere {}\0", i + 1).encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                work.left + col * width,
                work.top + row * height,
                width,
                height,
                None,
                None,
                Some(instance),
                None,
            )?;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, i as isize);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Abre el títere colgable: una sola ventana en su propio proceso, en el
/// centro del monitor principal. Bloquea hasta que se cierre.
pub fn run_hang_puppet() -> windows::core::Result<()> {
    // SAFETY: igual que `run_puppets`.
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        let monitor = MonitorFromWindow(HWND::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info =
            MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let work = info.rcWork;
        let (w, h) = ((work.right - work.left) / 4, (work.bottom - work.top) / 4);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("Títere colgable"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            work.left + (work.right - work.left - w) / 2,
            work.top + (work.bottom - work.top - h) / 2,
            w,
            h,
            None,
            None,
            Some(instance),
            None,
        )?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, HANG_INDEX);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Le pide al títere colgable que se cuelgue `ms` milisegundos (no bloquea).
pub fn hang(id: WindowId, ms: u32, busy: bool) -> windows::core::Result<()> {
    let hwnd = HWND(id.0 as *mut core::ffi::c_void);
    // SAFETY: PostMessageW no espera a la ventana destino.
    unsafe { PostMessageW(Some(hwnd), WM_PUPPET_HANG, WPARAM(ms as usize), LPARAM(busy.into())) }
}

/// El títere colgable, si está abierto.
pub fn find_hang_puppet() -> Option<WindowId> {
    find_all().into_iter().find(|(i, _)| *i == HANG_INDEX).map(|(_, id)| id)
}

/// Ventanas títere abiertas (de cualquier proceso), visibles o escondidas,
/// ordenadas por número. No incluye al colgable.
pub fn find_puppets() -> Vec<WindowId> {
    find_all().into_iter().filter(|(i, _)| *i != HANG_INDEX).map(|(_, id)| id).collect()
}

fn find_all() -> Vec<(isize, WindowId)> {
    let mut found: Vec<(isize, WindowId)> = Vec::new();

    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` es el `&mut Vec` que pasamos abajo.
        let found = unsafe { &mut *(data.0 as *mut Vec<(isize, WindowId)>) };
        let mut name = [0u16; 32];
        // SAFETY: buffer local válido.
        let len = unsafe { GetClassNameW(hwnd, &mut name) };
        if String::from_utf16_lossy(&name[..len.max(0) as usize]) == "wm-puppet" {
            // SAFETY: lectura del índice que guardó el proceso de títeres.
            let index = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            found.push((index, WindowId(hwnd.0 as isize)));
        }
        true.into()
    }

    // SAFETY: el callback solo usa `data` como el `&mut Vec` que le pasamos.
    let _ = unsafe {
        EnumWindows(Some(collect), LPARAM(&mut found as *mut Vec<(isize, WindowId)> as isize))
    };
    found.sort_by_key(|(index, _)| *index);
    found
}
