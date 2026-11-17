//! Fantasma para animar ventanas ajenas (architecture.md §7.4, spike 6).
//!
//! Una ventana nuestra sobre toda la pantalla virtual (sin bitmap de
//! redirección, transparente a los clics, nunca activable) muestra una
//! **miniatura en vivo** de la ventana real (`DwmRegisterThumbnail`). Se mueve
//! la miniatura cuadro a cuadro mientras la real, escondida con cloak, ya está
//! en su destino tras un solo `SetWindowPos`.
//!
//! - La miniatura es el **marco visible** (`DWMWA_EXTENDED_FRAME_BOUNDS`), sin
//!   los bordes invisibles de redimensionado ni la sombra: el destino se
//!   expresa en ese espacio, 1:1, para que el canje real ↔ fantasma no salte.
//! - Marcapasos: el reloj del compositor (`DCompositionWaitForCompositorClock`,
//!   Windows 11, resuelto en tiempo de ejecución) o `DwmFlush` como respaldo.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    DwmFlush, DwmGetWindowAttribute, DwmQueryThumbnailSourceSize, DwmRegisterThumbnail,
    DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, GetWindowRect, HTTRANSPARENT,
    LWA_ALPHA, MA_NOACTIVATE, RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, SetLayeredWindowAttributes,
    ShowWindow, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, HRESULT, s, w};
use wm_model::Rect;

use crate::windows_list::WindowId;

fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as *mut c_void)
}

fn to_rect(r: RECT) -> Rect {
    Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: procesamiento por defecto.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// `GetWindowRect`: incluye los bordes invisibles de redimensionado. Es el
/// rectángulo que espera `SetWindowPos`.
pub fn window_rect(id: WindowId) -> Option<Rect> {
    let mut r = RECT::default();
    // SAFETY: consulta de solo lectura.
    unsafe { GetWindowRect(hwnd(id), &mut r) }.ok()?;
    Some(to_rect(r))
}

/// Marco visible de la ventana (sin bordes invisibles ni sombra).
pub fn frame_bounds(id: WindowId) -> Option<Rect> {
    let mut r = RECT::default();
    // SAFETY: estructura local con su tamaño.
    unsafe {
        DwmGetWindowAttribute(
            hwnd(id),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(to_rect(r))
}

/// ¿DWM ya la tiene escondida (cualquier motivo de cloak)?
pub fn is_cloaked(id: WindowId) -> bool {
    let mut v = 0u32;
    // SAFETY: variable local con su tamaño.
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd(id),
            DWMWA_CLOAKED,
            &mut v as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && v != 0
}

/// Ventana anfitriona de las miniaturas: una sola, sobre toda la pantalla
/// virtual (sirve para cruzar de monitor), creada oculta.
pub struct GhostHost {
    host: HWND,
    origin: (i32, i32),
    size: (i32, i32),
}

/// La pantalla virtual ahora: (x, y, ancho, alto).
fn virtual_screen() -> (i32, i32, i32, i32) {
    // SAFETY: sin precondiciones.
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

impl GhostHost {
    pub fn new() -> windows::core::Result<Self> {
        let (x, y, cx, cy) = virtual_screen();
        // SAFETY: registro y creación de una ventana propia.
        let host = unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None)?.0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: w!("wm-ghost"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let h = CreateWindowExW(
                // Mismos estilos que los overlays (overlay.rs): sin bitmap de
                // redirección (si no, sería un monitor negro), y LAYERED +
                // TRANSPARENT para que los clics pasen a otros procesos.
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                w!("wm-ghost"),
                w!("wm ghost"),
                WS_POPUP,
                x,
                y,
                cx,
                // 1 px menos: ver overlay.rs (pantalla completa para el shell).
                cy - 1,
                None,
                None,
                Some(instance),
                None,
            )?;
            SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA)?;
            h
        };
        Ok(Self { host, origin: (x, y), size: (cx, cy) })
    }

    /// ¿Sigue cubriendo la pantalla virtual (no cambiaron los monitores)?
    pub fn is_current(&self) -> bool {
        let (x, y, cx, cy) = virtual_screen();
        // 1 px menos de alto, como al crearlo (`new`).
        let want = Rect::new(x, y, x + cx, y + cy - 1);
        self.origin == (x, y)
            && self.size == (cx, cy)
            && window_rect(crate::windows_list::WindowId(self.host.0 as isize)) == Some(want)
    }

    /// El HWND del anfitrión, para esconderlo desde otro hilo
    /// ([`cloak_host`]).
    pub fn raw(&self) -> isize {
        self.host.0 as isize
    }

    /// Muestra el anfitrión justo **debajo** de `above` (el mapa de teclado,
    /// si se ve): los dos están encima de todo y, sin esto, el fantasma de un
    /// vuelo tapaba el mapa. Sin `above`, como [`GhostHost::show`].
    pub fn show_below(&self, above: Option<isize>) {
        use windows::Win32::UI::WindowsAndMessaging::{
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        };
        let Some(raw) = above else { return self.show() };
        // SAFETY: ventanas propias del proceso.
        let ok = unsafe {
            SetWindowPos(
                self.host,
                Some(HWND(raw as *mut c_void)),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        if ok.is_err() {
            self.show();
        }
    }

    pub fn show(&self) {
        // SAFETY: ventana propia.
        let _ = unsafe { ShowWindow(self.host, SW_SHOWNOACTIVATE) };
    }

    pub fn hide(&self) {
        // SAFETY: ventana propia.
        let _ = unsafe { ShowWindow(self.host, SW_HIDE) };
    }

    /// Registra la miniatura de `source` (invisible hasta el primer `place`).
    pub fn ghost(&self, source: WindowId) -> windows::core::Result<Ghost> {
        // SAFETY: ventanas válidas; la miniatura se libera en Drop.
        let thumb = unsafe { DwmRegisterThumbnail(self.host, hwnd(source)) }?;
        Ok(Ghost { thumb, origin: self.origin, shown: false })
    }
}

impl Drop for GhostHost {
    fn drop(&mut self) {
        // SAFETY: ventana propia.
        let _ = unsafe { DestroyWindow(self.host) };
    }
}

pub struct Ghost {
    thumb: isize,
    origin: (i32, i32),
    shown: bool,
}

impl Ghost {
    /// Tamaño de la fuente según DWM (se espera igual al marco visible).
    pub fn source_size(&self) -> Option<(i32, i32)> {
        // SAFETY: miniatura propia.
        unsafe { DwmQueryThumbnailSourceSize(self.thumb) }.ok().map(|s| (s.cx, s.cy))
    }

    /// Pone la miniatura en `frame` (marco visible, coordenadas de pantalla).
    /// La primera vez la hace visible; después solo cambia el destino.
    pub fn place(&mut self, frame: Rect) -> windows::core::Result<()> {
        let (ox, oy) = self.origin;
        let mut props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION,
            rcDestination: RECT {
                left: frame.left - ox,
                top: frame.top - oy,
                right: frame.right - ox,
                bottom: frame.bottom - oy,
            },
            ..Default::default()
        };
        if !self.shown {
            props.dwFlags |= DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY;
            props.opacity = 255;
            props.fVisible = true.into();
            props.fSourceClientAreaOnly = false.into();
            self.shown = true;
        }
        // SAFETY: miniatura propia y estructura local.
        unsafe { DwmUpdateThumbnailProperties(self.thumb, &props) }
    }

    /// Como [`Ghost::place`], pero mostrando solo `w` × `h` píxeles de la
    /// fuente desde `(x, y)`, 1:1 (mini-spike 6b: recortar en vez de estirar
    /// al cambiar de tamaño). Medido: el origen de la fuente es el rectángulo
    /// de la ventana, con los bordes invisibles (sin correrlo, faltaban 7 px).
    pub fn place_crop(
        &mut self,
        frame: Rect,
        (x, y): (i32, i32),
        (w, h): (i32, i32),
    ) -> windows::core::Result<()> {
        use windows::Win32::Graphics::Dwm::DWM_TNP_RECTSOURCE;
        let (ox, oy) = self.origin;
        let mut props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_RECTSOURCE,
            rcDestination: RECT {
                left: frame.left - ox,
                top: frame.top - oy,
                right: frame.right - ox,
                bottom: frame.bottom - oy,
            },
            rcSource: RECT { left: x, top: y, right: x + w, bottom: y + h },
            ..Default::default()
        };
        if !self.shown {
            props.dwFlags |= DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY;
            props.opacity = 255;
            props.fVisible = true.into();
            props.fSourceClientAreaOnly = false.into();
            self.shown = true;
        }
        // SAFETY: miniatura propia y estructura local.
        unsafe { DwmUpdateThumbnailProperties(self.thumb, &props) }
    }

    pub fn conceal(&mut self) {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_VISIBLE,
            fVisible: false.into(),
            ..Default::default()
        };
        // SAFETY: miniatura propia y estructura local.
        let _ = unsafe { DwmUpdateThumbnailProperties(self.thumb, &props) };
        self.shown = false;
    }
}

impl Drop for Ghost {
    fn drop(&mut self) {
        // SAFETY: miniatura propia.
        let _ = unsafe { DwmUnregisterThumbnail(self.thumb) };
    }
}

/// Esconde el anfitrión (cloak de DWM) desde cualquier hilo de este proceso:
/// si el hilo `anim` se traba, el fantasma no queda pegado en pantalla.
pub fn cloak_host(raw: isize) {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    let on: BOOL = true.into();
    // SAFETY: ventana propia (o ya destruida: falla sin efecto); valor local.
    let _ = unsafe {
        DwmSetWindowAttribute(
            HWND(raw as *mut c_void),
            DWMWA_CLOAK,
            &on as *const BOOL as *const c_void,
            size_of::<BOOL>() as u32,
        )
    };
}

type WaitClock = unsafe extern "system" fn(u32, *const c_void, u32) -> u32;
type BoostClock = unsafe extern "system" fn(BOOL) -> HRESULT;

fn compositor_clock() -> Option<(WaitClock, BoostClock)> {
    static CLOCK: OnceLock<Option<(usize, usize)>> = OnceLock::new();
    let (wait, boost) = (*CLOCK.get_or_init(|| {
        let wait = crate::dynlib::proc(w!("dcomp.dll"), s!("DCompositionWaitForCompositorClock"));
        let boost = crate::dynlib::proc(w!("dcomp.dll"), s!("DCompositionBoostCompositorClock"));
        match (wait, boost) {
            (Ok(Some(w)), Ok(Some(b))) => Some((w as usize, b as usize)),
            _ => None,
        }
    }))?;
    // SAFETY: firmas documentadas (Windows 11, build 22000+).
    Some(unsafe {
        (
            std::mem::transmute::<usize, WaitClock>(wait),
            std::mem::transmute::<usize, BoostClock>(boost),
        )
    })
}

/// ¿Hay reloj del compositor (Windows 11)? Si no, se usa `DwmFlush`.
pub fn has_compositor_clock() -> bool {
    compositor_clock().is_some()
}

/// Pide al compositor que no baje su ritmo durante una animación.
pub fn boost_clock(on: bool) {
    if let Some((_, boost)) = compositor_clock() {
        // SAFETY: firma documentada.
        let _ = unsafe { boost(on.into()) };
    }
}

/// Despacha los mensajes pendientes del hilo (las ventanas de prueba, como el
/// telón, tienen que pintarse).
fn pump() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };
    let mut m = MSG::default();
    // SAFETY: bucle de mensajes estándar con un MSG válido.
    while unsafe { PeekMessageW(&mut m, None, 0, 0, PM_REMOVE) }.as_bool() {
        // SAFETY: mensaje recién obtenido.
        unsafe {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}

/// Espera el próximo cuadro del compositor (y despacha mensajes).
pub fn wait_frame() {
    pump();
    match compositor_clock() {
        // SAFETY: sin handles extra; timeout de 100 ms.
        Some((wait, _)) => unsafe {
            wait(0, std::ptr::null(), 100);
        },
        // SAFETY: sin precondiciones.
        None => unsafe {
            let _ = DwmFlush();
        },
    }
}
