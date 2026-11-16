//! Diagnóstico de la sesión (spike 9, architecture.md §10): si hay escritorio
//! interactivo, con qué privilegios corre el proceso y qué ventana tiene el
//! foco. Lo usa `wm ci-probe` para saber qué se puede probar en una máquina
//! de CI, y qué hay que dejar para una máquina propia.

use std::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::RemoteDesktop::{ProcessIdToSessionId, WTSGetActiveConsoleSessionId};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, GetProcessWindowStation,
    GetUserObjectInformationW, OpenInputDesktop, UOI_NAME,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CAPITAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetShellWindow, GetWindowTextW, GetWindowThreadProcessId,
};

use crate::windows_list::WindowId;

/// Lo que se sabe de la sesión donde corre el proceso.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Sesión del proceso (0 = la de los servicios, sin escritorio).
    pub session_id: Option<u32>,
    /// Sesión conectada a la consola física (o la que hace de consola).
    pub console_session_id: u32,
    /// Estación de ventanas ("WinSta0" = la interactiva).
    pub window_station: String,
    /// Escritorio que recibe la entrada ("Default" si se puede abrir; `None`
    /// si no: sesión bloqueada, UAC o sin escritorio interactivo).
    pub input_desktop: Option<String>,
    /// El proceso corre elevado (administrador con UAC aceptado o apagado).
    pub elevated: Option<bool>,
    /// Hay shell (Explorer u otro) con su ventana de escritorio.
    pub shell: bool,
    /// Clase y título de la ventana con el foco.
    pub foreground: Option<(String, String)>,
}

fn object_name(handle: HANDLE) -> Option<String> {
    let mut buf = [0u16; 128];
    let mut needed = 0u32;
    // SAFETY: búfer local con su tamaño en bytes; `handle` es una estación o
    // un escritorio válido.
    unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(buf.as_mut_ptr().cast::<c_void>()),
            std::mem::size_of_val(&buf) as u32,
            Some(&mut needed),
        )
        .ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn elevated() -> Option<bool> {
    // SAFETY: token propio abierto para consulta y cerrado al final; la
    // estructura de salida es local y de tamaño correcto.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast::<c_void>()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        ok.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }
}

fn class_and_title(hwnd: HWND) -> (String, String) {
    let mut class = [0u16; 128];
    let mut title = [0u16; 256];
    // SAFETY: búferes locales; si la ventana ya no existe, devuelven 0.
    let (c, t) = unsafe { (GetClassNameW(hwnd, &mut class), GetWindowTextW(hwnd, &mut title)) };
    let text = |b: &[u16], n: i32| String::from_utf16_lossy(&b[..n.max(0) as usize]);
    (text(&class, c), text(&title, t))
}

pub fn session_info() -> SessionInfo {
    let mut session = 0u32;
    // SAFETY: consultas sin efectos sobre el proceso actual, con salidas locales.
    let session_id =
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) }.ok().map(|()| session);
    // SAFETY: sin precondiciones.
    let console_session_id = unsafe { WTSGetActiveConsoleSessionId() };
    // SAFETY: la estación del proceso no se cierra (no es nuestra).
    let window_station = unsafe { GetProcessWindowStation() }
        .ok()
        .and_then(|w| object_name(HANDLE(w.0)))
        .unwrap_or_default();
    // SAFETY: el escritorio abierto se cierra enseguida.
    let input_desktop = unsafe {
        OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS).ok().map(|d| {
            let name = object_name(HANDLE(d.0)).unwrap_or_default();
            let _ = CloseDesktop(d);
            name
        })
    };
    // SAFETY: sin precondiciones.
    let shell = !unsafe { GetShellWindow() }.is_invalid();
    // SAFETY: sin precondiciones.
    let fg = unsafe { GetForegroundWindow() };
    let foreground = (!fg.is_invalid()).then(|| class_and_title(fg));
    SessionInfo {
        session_id,
        console_session_id,
        window_station,
        input_desktop,
        elevated: elevated(),
        shell,
        foreground,
    }
}

/// Proceso dueño de la ventana.
pub fn window_pid(id: WindowId) -> u32 {
    let mut pid = 0u32;
    // SAFETY: si la ventana ya no existe, `pid` queda en 0.
    unsafe { GetWindowThreadProcessId(HWND(id.0 as *mut c_void), Some(&mut pid)) };
    pid
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                // Sin nuestra marca: el hook lo trata como teclas del usuario.
                dwExtraInfo: 0,
            },
        },
    }
}

/// Inyecta Caps + `vk` como si lo apretara el usuario (sin la marca de
/// [`crate::input::INJECTED_MARKER`]). Devuelve cuántos eventos aceptó
/// `SendInput` (4 si todo bien; menos si UIPI o el escritorio lo impiden).
pub fn send_hyper_chord(vk: u8) -> u32 {
    let caps = VK_CAPITAL.0;
    let inputs = [key(caps, false), key(vk.into(), false), key(vk.into(), true), key(caps, true)];
    // SAFETY: slice válido de estructuras INPUT completas.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) }
}

/// Inyecta una secuencia de teclas (`(vk, abajo)`) como si fuera el usuario,
/// sin nuestra marca. Devuelve cuántos eventos aceptó `SendInput`.
pub fn send_keys(keys: &[(u8, bool)]) -> u32 {
    let inputs: Vec<INPUT> = keys.iter().map(|&(vk, down)| key(vk.into(), !down)).collect();
    // SAFETY: slice válido de estructuras INPUT completas.
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) }
}

/// ¿La tecla figura apretada en el estado asíncrono del sistema?
pub fn key_down(vk: u8) -> bool {
    // SAFETY: sin precondiciones.
    (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16) & 0x8000 != 0
}

/// ¿Bloq Mayús está activado?
pub fn caps_lock_on() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
    // SAFETY: sin precondiciones.
    unsafe { GetKeyState(0x14) & 1 != 0 }
}
