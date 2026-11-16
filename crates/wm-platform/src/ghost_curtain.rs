//! Telón magenta para medir el fantasma con apps reales (spike 6): una
//! ventana del tamaño del monitor, justo **debajo** de la ventana animada. Así
//! el medidor cuenta "lo que no es magenta" en una fila y sabe dónde está la
//! ventana (o el fantasma) sin conocer sus colores. Solo para pruebas.

use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND};
use windows::Win32::Graphics::Gdi::CreateSolidBrush;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;
use wm_model::Rect;

use crate::windows_list::WindowId;

pub struct Curtain(HWND);

unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    // SAFETY: procesamiento por defecto (pinta el fondo con el pincel de la clase).
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Curtain {
    /// Crea el telón sobre `area` (1 px menos de alto) y lo pone justo debajo
    /// de `above`. Si falla, devuelve un telón vacío (la medición lo notará).
    pub fn new(area: Rect, above: WindowId) -> Self {
        // SAFETY: registro y creación de una ventana propia; el pincel vive
        // lo que dure el proceso.
        let h = unsafe {
            let Ok(module) = GetModuleHandleW(None) else { return Self(HWND::default()) };
            let instance = HINSTANCE(module.0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: w!("wm-curtain"),
                hbrBackground: CreateSolidBrush(COLORREF(0x00FF_00FF)),
                ..Default::default()
            };
            RegisterClassW(&class);
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("wm-curtain"),
                w!("wm curtain"),
                WS_POPUP,
                area.left,
                area.top,
                area.width(),
                area.height() - 1,
                None,
                None,
                Some(instance),
                None,
            )
            .unwrap_or_default()
        };
        // SAFETY: ventanas válidas; insertar después de `above` = justo debajo.
        unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                h,
                Some(HWND(above.0 as *mut c_void)),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        Self(h)
    }
}

impl Drop for Curtain {
    fn drop(&mut self) {
        // SAFETY: ventana propia.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}
