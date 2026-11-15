//! Llamadas sueltas para la sonda del spike 4 (apps colgadas). Algunas son
//! justamente las prohibidas por architecture.md §4.4: están acá para medir
//! que de verdad se bloquean, no para usarlas en el core.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowThreadProcessId, IsHungAppWindow,
    SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SendMessageTimeoutW,
    SetWindowPos, WM_NULL,
};
use windows::core::BOOL;

use crate::windows_list::WindowId;

fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as *mut c_void)
}

/// `SetWindowPos` **síncrono** a la misma posición (control: debería
/// bloquearse con la ventana colgada).
pub fn set_window_pos_sync(id: WindowId) {
    let h = hwnd(id);
    let mut r = Default::default();
    // SAFETY: llamadas Win32 sobre un HWND (si ya no existe, fallan sin más).
    unsafe {
        let _ = GetWindowRect(h, &mut r);
        let _ = SetWindowPos(
            h,
            None,
            r.left,
            r.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// `SendMessageTimeout(WM_NULL, SMTO_ABORTIFHUNG)`: ¿responde en `ms`?
pub fn responds(id: WindowId, ms: u32) -> bool {
    let mut result = 0usize;
    // SAFETY: WM_NULL no tiene efectos; el resultado va a una variable local.
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd(id),
            WM_NULL,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            ms,
            Some(&mut result),
        )
    };
    ok.0 != 0
}

/// `IsHungAppWindow` (documentado como "no de uso general").
pub fn is_hung(id: WindowId) -> bool {
    // SAFETY: consulta de solo lectura.
    unsafe { IsHungAppWindow(hwnd(id)) }.as_bool()
}

/// Ventanas fantasma (clase `Ghost`) que crea Windows para apps colgadas:
/// `(id, pid del proceso dueño del fantasma)`.
pub fn ghost_windows() -> Vec<(WindowId, u32)> {
    let mut found: Vec<(WindowId, u32)> = Vec::new();

    unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` es el `&mut Vec` que pasamos abajo.
        let found = unsafe { &mut *(data.0 as *mut Vec<(WindowId, u32)>) };
        let mut name = [0u16; 32];
        // SAFETY: buffer local válido.
        let len = unsafe { GetClassNameW(h, &mut name) };
        if String::from_utf16_lossy(&name[..len.max(0) as usize]) == "Ghost" {
            let mut pid = 0u32;
            // SAFETY: puntero a variable local.
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
            found.push((WindowId(h.0 as isize), pid));
        }
        true.into()
    }

    // SAFETY: el callback solo usa `data` como el `&mut Vec` que le pasamos.
    let _ = unsafe {
        EnumWindows(Some(collect), LPARAM(&mut found as *mut Vec<(WindowId, u32)> as isize))
    };
    found
}
