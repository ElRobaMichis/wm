//! Enumerar las ventanas "de usuario" y traer una al frente.

use std::ffi::c_void;
use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetForegroundWindow, GetWindow, GetWindowLongPtrW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SW_RESTORE, SetForegroundWindow, ShowWindowAsync, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

/// Identificador de ventana que se puede pasar entre hilos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub isize);

impl WindowId {
    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    /// Nombre del ejecutable sin extensión, en minúsculas (por ejemplo "chrome").
    pub app: String,
}

/// Ventanas que un usuario consideraría "abiertas" (las del Alt+Tab), en
/// orden Z: la primera es la de más arriba.
pub fn user_windows() -> Vec<WindowInfo> {
    let mut handles: Vec<HWND> = Vec::new();

    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` es el `&mut Vec` que pasamos abajo; EnumWindows llama
        // al callback de forma síncrona en este mismo hilo.
        let handles = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        handles.push(hwnd);
        true.into()
    }

    // SAFETY: el callback solo usa `data` como el `&mut Vec` que le pasamos.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut handles as *mut Vec<HWND> as isize)) };

    let own_pid = std::process::id();
    handles
        .into_iter()
        .filter(|&h| is_user_window(h))
        .filter_map(|h| {
            let mut pid = 0u32;
            // SAFETY: `h` es un HWND de EnumWindows; `pid` es una variable local.
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
            if pid == own_pid {
                return None;
            }
            let app = process_name(pid)?;
            Some(WindowInfo { id: WindowId(h.0 as isize), title: title(h), app })
        })
        .collect()
}

fn is_user_window(h: HWND) -> bool {
    // SAFETY: consultas de solo lectura sobre un HWND; no bloquean aunque la
    // app esté colgada (architecture.md §0).
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            return false;
        }
        if GetWindow(h, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return false;
        }
        if GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut cloaked = 0u32;
        let ok = DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        if ok.is_ok() && cloaked != 0 {
            return false;
        }
        GetWindowTextLengthW(h) > 0
    }
}

/// Título de una ventana (vacío si no tiene).
pub fn window_title(id: WindowId) -> String {
    title(HWND(id.0 as *mut core::ffi::c_void))
}

fn title(h: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: buffer local válido; GetWindowTextW no bloquea con ventanas
    // de otros procesos (lee el texto que guarda el sistema).
    let len = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: OpenProcess con el permiso mínimo; el handle se cierra abajo.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: buffer local válido y `len` con su tamaño en caracteres.
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    };
    // SAFETY: handle propio, abierto arriba.
    let _ = unsafe { CloseHandle(process) };
    ok.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    let file = path.rsplit(['\\', '/']).next()?;
    let stem = file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file);
    Some(stem.to_lowercase())
}

pub fn foreground() -> Option<WindowId> {
    // SAFETY: sin precondiciones.
    let h = unsafe { GetForegroundWindow() };
    (!h.is_invalid()).then_some(WindowId(h.0 as isize))
}

/// Resultado de intentar traer una ventana al frente.
#[derive(Debug, Clone, Copy)]
pub struct Activation {
    pub ok: bool,
    /// Hizo falta inyectar la tecla neutra para obtener permiso de foco.
    pub used_neutral_key: bool,
    pub elapsed: Duration,
}

/// Trae `id` al frente (restaurándola si está minimizada).
///
/// Windows solo deja cambiar el foco a procesos que cumplen ciertas
/// condiciones (architecture.md §5.6). Primero se intenta directo; si no
/// alcanza, se inyecta una tecla neutra marcada y se reintenta.
pub fn activate(id: WindowId) -> Activation {
    let start = Instant::now();
    let hwnd = id.hwnd();
    // SAFETY: llamadas asíncronas o de solo lectura sobre un HWND; si la
    // ventana ya no existe, fallan sin efectos.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() && wait_foreground(id) {
            return Activation { ok: true, used_neutral_key: false, elapsed: start.elapsed() };
        }
        crate::input::send_neutral_key();
        let ok = SetForegroundWindow(hwnd).as_bool() && wait_foreground(id);
        Activation { ok, used_neutral_key: true, elapsed: start.elapsed() }
    }
}

/// Espera hasta 50 ms a que `id` figure como ventana de primer plano.
fn wait_foreground(id: WindowId) -> bool {
    let deadline = Instant::now() + Duration::from_millis(50);
    loop {
        if foreground() == Some(id) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
