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

/// Suspende (o reanuda) todos los hilos de un proceso. Solo pruebas: usa
/// `NtSuspendProcess`/`NtResumeProcess` de ntdll (no documentadas).
pub fn suspend_process(handle: isize, suspend: bool) -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::core::{s, w};
    type NtFn = unsafe extern "system" fn(HANDLE) -> i32;
    // SAFETY: ntdll siempre está cargada; la firma es la conocida de estas
    // funciones (un HANDLE, devuelven NTSTATUS).
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(w!("ntdll.dll")) else { return false };
        let name = if suspend { s!("NtSuspendProcess") } else { s!("NtResumeProcess") };
        let Some(f) = GetProcAddress(ntdll, name) else { return false };
        let f: NtFn = std::mem::transmute(f);
        f(HANDLE(handle as *mut c_void)) >= 0
    }
}

/// Tiempo de CPU (usuario + núcleo) consumido por un proceso, en ms.
pub fn process_cpu_ms(handle: isize) -> f64 {
    use windows::Win32::Foundation::{FILETIME, HANDLE};
    use windows::Win32::System::Threading::GetProcessTimes;
    let [mut c, mut e, mut k, mut u] = [FILETIME::default(); 4];
    let h = HANDLE(handle as *mut c_void);
    // SAFETY: punteros a variables locales; handle con permiso de consulta.
    let ok = unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) };
    if ok.is_err() {
        return f64::NAN;
    }
    let t = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32 | u64::from(f.dwLowDateTime)) as f64;
    (t(k) + t(u)) / 10_000.0
}

/// Clase de la ventana que recibiría un clic en (`x`, `y`) de pantalla.
pub fn class_at(x: i32, y: i32) -> String {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::WindowFromPoint;
    // SAFETY: consulta de solo lectura.
    let h = unsafe { WindowFromPoint(POINT { x, y }) };
    let mut name = [0u16; 64];
    // SAFETY: buffer local válido.
    let len = unsafe { GetClassNameW(h, &mut name) };
    String::from_utf16_lossy(&name[..len.max(0) as usize])
}
