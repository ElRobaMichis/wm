//! Overlays con Windows.UI.Composition (architecture.md §3, §7.2 y spike 5).
//!
//! - Una ventana por monitor, del tamaño del monitor, transparente a los
//!   clics, que **nunca se activa** (el teclado llega por el hook).
//! - Sin bitmap de redirección (`WS_EX_NOREDIRECTIONBITMAP`): lo que se ve es
//!   el árbol de visuales de Composition, que compone DWM.
//! - Las animaciones (`KeyFrameAnimation`) las corre DWM: nuestro hilo las
//!   emite y se duerme.
//!
//! Todo se crea en el hilo que llama a [`Overlays::new`], que tiene que
//! bombear mensajes (el `DispatcherQueue` de Composition vive en su cola).

use std::mem::size_of;

use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{Compositor, ContainerVisual};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::{DQTAT_COM_STA, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HTTRANSPARENT, MA_NOACTIVATE, RegisterClassW,
    SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Interface, w};
use wm_model::Rect;

/// Un monitor con su ventana overlay.
pub struct Screen {
    pub hwnd: HWND,
    /// Rectángulo del monitor en píxeles físicos.
    pub bounds: Rect,
    pub primary: bool,
    pub refresh_hz: Option<u32>,
    /// DPI efectivo del monitor (96 = 100 %).
    pub dpi: u32,
    /// Raíz del árbol de visuales de este monitor (coordenadas relativas).
    pub root: ContainerVisual,
    _target: DesktopWindowTarget,
}

pub struct Overlays {
    compositor: Compositor,
    screens: Vec<Screen>,
    _queue: windows::System::DispatcherQueueController,
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // Los clics de otros procesos atraviesan la ventana por
        // WS_EX_LAYERED | WS_EX_TRANSPARENT; esto cubre los del propio hilo.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: procesamiento por defecto.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl Overlays {
    /// Crea el `DispatcherQueue` del hilo actual, el `Compositor` y una
    /// ventana overlay oculta por monitor.
    pub fn new() -> windows::core::Result<Self> {
        let options = DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        };
        let queue = create_dispatcher_queue(options)?;
        let compositor = Compositor::new()?;
        register_class()?;
        let screens = create_screens(&compositor)?;
        Ok(Self { compositor, screens, _queue: queue })
    }

    /// ¿Las ventanas siguen correspondiendo a los monitores de ahora (mismos
    /// rectángulos y escalas)?
    pub fn is_current(&self) -> bool {
        let now = crate::monitors();
        now.len() == self.screens.len()
            && now.iter().zip(&self.screens).all(|(m, s)| m.bounds == s.bounds && m.dpi == s.dpi)
    }

    /// Rearma las ventanas para los monitores de ahora (se conectó o
    /// desconectó uno, o cambió la resolución o la escala). Mismo compositor.
    pub fn rebuild_screens(&mut self) -> windows::core::Result<()> {
        let new = create_screens(&self.compositor)?;
        for s in std::mem::replace(&mut self.screens, new) {
            // SAFETY: ventanas propias, creadas en este hilo.
            let _ = unsafe { DestroyWindow(s.hwnd) };
        }
        Ok(())
    }

    pub fn compositor(&self) -> &Compositor {
        &self.compositor
    }

    pub fn screens(&self) -> &[Screen] {
        &self.screens
    }

    /// Muestra las ventanas overlay sin activarlas.
    pub fn show(&self) {
        for s in &self.screens {
            // SAFETY: ventanas propias.
            let _ = unsafe { ShowWindow(s.hwnd, SW_SHOWNOACTIVATE) };
        }
    }

    /// Muestra solo la ventana del monitor `i` (encima de todo, sin
    /// activarla) y esconde las demás.
    pub fn show_only(&self, i: usize) {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        };
        for (k, s) in self.screens.iter().enumerate() {
            if k == i {
                // SAFETY: ventana propia.
                let _ = unsafe {
                    SetWindowPos(
                        s.hwnd,
                        Some(HWND_TOPMOST),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                };
            } else {
                // SAFETY: ventana propia.
                let _ = unsafe { ShowWindow(s.hwnd, SW_HIDE) };
            }
        }
    }

    /// Muestra también la ventana del monitor `i` (encima de todo, sin
    /// activarla) sin tocar las demás: para un aviso con el mapa visible en
    /// otro monitor.
    pub fn show_also(&self, i: usize) {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        };
        if let Some(s) = self.screens.get(i) {
            // SAFETY: ventana propia.
            let _ = unsafe {
                SetWindowPos(
                    s.hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
        }
    }

    /// Esconde la ventana del monitor `i`.
    pub fn hide_one(&self, i: usize) {
        if let Some(s) = self.screens.get(i) {
            // SAFETY: ventana propia.
            let _ = unsafe { ShowWindow(s.hwnd, SW_HIDE) };
        }
    }

    pub fn hide(&self) {
        for s in &self.screens {
            // SAFETY: ventanas propias.
            let _ = unsafe { ShowWindow(s.hwnd, SW_HIDE) };
        }
    }
}

impl Drop for Overlays {
    fn drop(&mut self) {
        for s in &self.screens {
            // SAFETY: ventanas propias, creadas en este hilo.
            let _ = unsafe { DestroyWindow(s.hwnd) };
        }
    }
}

fn register_class() -> windows::core::Result<()> {
    // SAFETY: registro de clase propia; el procedimiento vive siempre.
    let instance = unsafe { HINSTANCE(GetModuleHandleW(None)?.0) };
    let class = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: instance,
        lpszClassName: w!("wm-overlay"),
        ..Default::default()
    };
    // SAFETY: estructura local válida (si ya estaba registrada, falla sin efectos).
    unsafe { RegisterClassW(&class) };
    Ok(())
}

/// Una ventana overlay oculta por monitor, con su raíz de Composition.
fn create_screens(compositor: &Compositor) -> windows::core::Result<Vec<Screen>> {
    let interop: ICompositorDesktopInterop = compositor.cast()?;
    // SAFETY: módulo actual.
    let instance = unsafe { HINSTANCE(GetModuleHandleW(None)?.0) };
    let mut screens = Vec::new();
    for m in crate::monitors() {
        let b = m.bounds;
        // SAFETY: ventana propia; los parámetros son válidos.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT
                    | WS_EX_LAYERED,
                w!("wm-overlay"),
                w!("wm overlay"),
                WS_POPUP,
                b.left,
                b.top,
                b.width(),
                // 1 px menos que el monitor: si la ventana lo cubre entero,
                // el shell cree que hay una app a pantalla completa y, por
                // ejemplo, silencia las notificaciones (spike 5).
                b.height() - 1,
                None,
                None,
                Some(instance),
                None,
            )
        }?;
        // Una ventana layered no se muestra hasta fijarle atributos.
        // SAFETY: ventana propia recién creada.
        unsafe {
            use windows::Win32::Foundation::COLORREF;
            use windows::Win32::UI::WindowsAndMessaging::{LWA_ALPHA, SetLayeredWindowAttributes};
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        }
        // SAFETY: interfaz válida y ventana propia de este hilo.
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, true) }?;
        let root = compositor.CreateContainerVisual()?;
        root.SetSize(windows_numerics::Vector2 { X: b.width() as f32, Y: b.height() as f32 })?;
        target.SetRoot(&root)?;
        screens.push(Screen {
            hwnd,
            bounds: b,
            primary: m.primary,
            refresh_hz: m.refresh_hz,
            dpi: m.dpi,
            root,
            _target: target,
        });
    }
    Ok(screens)
}

/// `CreateDispatcherQueueController` resuelta en tiempo de ejecución, para
/// que `wm.exe` no importe `CoreMessaging.dll` en todos sus roles.
fn create_dispatcher_queue(
    options: DispatcherQueueOptions,
) -> windows::core::Result<windows::System::DispatcherQueueController> {
    use std::ffi::c_void;
    use windows::core::{HRESULT, s, w};
    type Create = unsafe extern "system" fn(DispatcherQueueOptions, *mut *mut c_void) -> HRESULT;
    let f = crate::dynlib::proc(w!("CoreMessaging.dll"), s!("CreateDispatcherQueueController"))?;
    // SAFETY: firma documentada de CreateDispatcherQueueController.
    let create: Create = unsafe { std::mem::transmute(f) };
    let mut raw = std::ptr::null_mut();
    // SAFETY: estructura local válida; `raw` recibe una referencia propia.
    unsafe { create(options, &mut raw) }.ok()?;
    // SAFETY: `raw` es un DispatcherQueueController recién creado (referencia propia).
    Ok(unsafe { windows::System::DispatcherQueueController::from_raw(raw) })
}

/// Memoria del proceso actual: (working set, bytes privados comprometidos).
/// Usa la variante de `kernel32` (`K32…`) para no importar `psapi.dll`.
pub fn process_memory() -> (usize, usize) {
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: estructura local con su tamaño; el proceso actual siempre es válido.
    let _ =
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut _, c.cb) };
    (c.WorkingSetSize, c.PrivateUsage)
}

/// Que Windows no reemplace nuestras ventanas por una "fantasma" si el hilo
/// de UI deja de responder: un fantasma de un overlay a pantalla completa
/// taparía todo (Fable, spike 5).
pub fn disable_ghosting() {
    use windows::Win32::UI::WindowsAndMessaging::DisableProcessWindowsGhosting;
    // SAFETY: sin precondiciones.
    unsafe { DisableProcessWindowsGhosting() };
}

/// Postea `msg` (con `wparam`) a la cola del hilo `thread`.
pub fn post_to_thread(thread: u32, msg: u32, wparam: usize) {
    use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
    // SAFETY: PostThreadMessageW no bloquea.
    let _ = unsafe { PostThreadMessageW(thread, msg, WPARAM(wparam), LPARAM(0)) };
}

/// Bombea los mensajes del hilo hasta recibir un mensaje de hilo `msg`
/// (devuelve su `wParam`) o `WM_QUIT` (`None`). Así Composition sigue
/// recibiendo su cola mientras se espera.
pub fn next_thread_message(msg: u32) -> Option<usize> {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, TranslateMessage,
    };
    let mut m = MSG::default();
    // SAFETY: bucle de mensajes estándar con un MSG válido.
    while unsafe { GetMessageW(&mut m, None, 0, 0) }.as_bool() {
        if m.hwnd.is_invalid() && m.message == msg {
            return Some(m.wParam.0);
        }
        // SAFETY: mensaje recién obtenido de GetMessageW.
        unsafe {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    None
}
