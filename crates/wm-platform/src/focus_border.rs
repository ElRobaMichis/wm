//! Borde de foco propio (`funciones-ocultas.md` §2, "lo completo"): una
//! ventana nuestra, transparente a los clics, con una línea de 2 px en
//! `accent` justo afuera del marco de la ventana activa, puesta en el orden Z
//! justo encima de ella. A diferencia de `DWMWA_BORDER_COLOR`, se ve también
//! en las apps que dibujan su propio marco (Windows Terminal, Electron).
//!
//! Vive en el hilo `wm-ui` (usa su `Compositor`). Sigue a la ventana con
//! WinEvents **acotados a su proceso** (`EVENT_OBJECT_LOCATIONCHANGE` y
//! cloak/uncloak): ningún evento del resto del sistema despierta al hilo, y
//! con la ventana quieta no llega nada (architecture.md §4.3).

use std::cell::Cell;

use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionRoundedRectangleGeometry, Compositor, ContainerVisual, ShapeVisual,
};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_OBJECT_UNCLOAKED, GW_HWNDPREV, GetWindow, HWND_TOP, PostThreadMessageW, SW_HIDE,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WINEVENT_OUTOFCONTEXT, WM_APP,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{Interface, w};
use windows_numerics::Vector2;

use crate::windows_list::WindowId;

/// Mensaje de hilo (a `wm-ui`): la ventana seguida se movió o cambió.
pub const WM_BORDER_UPDATE: u32 = WM_APP + 0x61;

thread_local! {
    /// La ventana seguida (para filtrar los eventos de su proceso).
    static TARGET: Cell<isize> = const { Cell::new(0) };
    /// Ya hay un `WM_BORDER_UPDATE` en la cola (se coalescen).
    static POSTED: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    if object != 0 || child != 0 || hwnd.0 as isize != TARGET.with(Cell::get) {
        return;
    }
    if !POSTED.with(|p| p.replace(true)) {
        // SAFETY: mensaje de hilo al propio hilo; no bloquea.
        let _ = unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_BORDER_UPDATE, WPARAM(0), LPARAM(0))
        };
    }
}

unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, HTTRANSPARENT, MA_NOACTIVATE, WM_MOUSEACTIVATE, WM_NCHITTEST,
    };
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: procesamiento por defecto.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Grosor de la línea a 96 DPI.
const THICKNESS: f32 = 2.0;
/// Radio de las esquinas de Windows 11 a 96 DPI.
const RADIUS: f32 = 8.0;

pub struct FocusBorder {
    hwnd: HWND,
    _target: DesktopWindowTarget,
    visual: ShapeVisual,
    geom: CompositionRoundedRectangleGeometry,
    follow: Option<WindowId>,
    hooks: Vec<HWINEVENTHOOK>,
    hooked_pid: u32,
    shown: bool,
}

impl FocusBorder {
    /// Crea la ventana (oculta). En el hilo del `Compositor`.
    pub fn new(compositor: &Compositor) -> windows::core::Result<Self> {
        // Clase propia (no `wm-overlay`: el `ci-probe` cuenta esas como mapa):
        // transparente a los clics y que nunca se activa.
        // SAFETY: módulo actual.
        let instance = unsafe { HINSTANCE(GetModuleHandleW(None)?.0) };
        let class = windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: w!("wm-border"),
            ..Default::default()
        };
        // SAFETY: estructura local válida (si ya existe, falla sin efectos).
        unsafe { windows::Win32::UI::WindowsAndMessaging::RegisterClassW(&class) };
        // SAFETY: ventana propia; parámetros válidos.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT
                    | WS_EX_LAYERED,
                w!("wm-border"),
                w!("wm borde de foco"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )
        }?;
        // SAFETY: ventana propia recién creada (una layered no se ve sin esto).
        unsafe {
            use windows::Win32::Foundation::COLORREF;
            use windows::Win32::UI::WindowsAndMessaging::{LWA_ALPHA, SetLayeredWindowAttributes};
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        }
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        // SAFETY: interfaz válida y ventana propia de este hilo.
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, true) }?;
        let root: ContainerVisual = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let visual = compositor.CreateShapeVisual()?;
        let geom = compositor.CreateRoundedRectangleGeometry()?;
        let shape = compositor.CreateSpriteShapeWithGeometry(&geom)?;
        shape.SetStrokeBrush(
            &compositor.CreateColorBrushWithColor(crate::overlay_text::tokens::ACCENT)?,
        )?;
        visual.Shapes()?.Append(&shape)?;
        root.Children()?.InsertAtTop(&visual)?;
        // El grosor se fija en `update` (depende del DPI).
        Ok(Self {
            hwnd,
            _target: target,
            visual,
            geom,
            follow: None,
            hooks: Vec::new(),
            hooked_pid: 0,
            shown: false,
        })
    }

    /// Seguir a `win` (o a ninguna).
    pub fn follow(&mut self, win: Option<WindowId>) {
        self.follow = win;
        TARGET.with(|t| t.set(win.map_or(0, |w| w.0)));
        let pid = win.map_or(0, crate::windows_list::window_pid);
        if pid != self.hooked_pid {
            self.unhook();
            if pid != 0 {
                for (a, b) in [
                    (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
                    (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
                ] {
                    // SAFETY: callback con la firma de WINEVENTPROC; fuera de
                    // contexto, acotado al proceso de la ventana.
                    let h = unsafe {
                        SetWinEventHook(a, b, None, Some(on_event), pid, 0, WINEVENT_OUTOFCONTEXT)
                    };
                    if !h.is_invalid() {
                        self.hooks.push(h);
                    }
                }
            }
            self.hooked_pid = pid;
        }
        self.update();
    }

    fn unhook(&mut self) {
        for h in self.hooks.drain(..) {
            // SAFETY: hooks propios de este hilo.
            let _ = unsafe { UnhookWinEvent(h) };
        }
        self.hooked_pid = 0;
    }

    fn hide(&mut self) {
        if self.shown {
            // SAFETY: ventana propia.
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.shown = false;
        }
    }

    /// Reubica el borde (o lo esconde: minimizada, maximizada, a pantalla
    /// completa, con cloak, o ya cerrada).
    pub fn update(&mut self) {
        POSTED.with(|p| p.set(false));
        let Some(win) = self.follow else { return self.hide() };
        let v = crate::hide::visibility(win);
        let frame = crate::ghost::frame_bounds(win);
        let Some(f) = frame.filter(|f| f.width() > 0 && f.height() > 0) else { return self.hide() };
        let fullscreen = crate::monitors().iter().any(|m| m.bounds == f);
        if !v.visible
            || v.minimized
            || v.cloaked != 0
            || crate::hide::is_maximized(win)
            || fullscreen
        {
            return self.hide();
        }
        let scale = crate::geometry::window_dpi(win).max(96) as f32 / 96.0;
        let t = (THICKNESS * scale).round().max(1.0);
        let r = RADIUS * scale;
        let ti = t as i32;
        let (x, y, w, h) = (f.left - ti, f.top - ti, f.width() + 2 * ti, f.height() + 2 * ti);
        // Justo encima de la ventana en el orden Z (si otra la tapa, también
        // tapa al borde). Si la de arriba ya es el borde, no se toca el orden.
        // SAFETY: lecturas sobre HWND; si no existen, fallan sin efectos.
        let above = unsafe { GetWindow(win.hwnd(), GW_HWNDPREV) }.ok().filter(|h| !h.is_invalid());
        let insert_after = match above {
            Some(a) if a == self.hwnd => None,
            Some(a) => Some(a),
            None => Some(HWND_TOP),
        };
        let mut flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
        if insert_after.is_none() {
            flags |= windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER;
        }
        // SAFETY: ventana propia.
        let _ = unsafe { SetWindowPos(self.hwnd, insert_after, x, y, w, h, flags) };
        self.shown = true;
        let _ = self.visual.SetSize(Vector2 { X: w as f32, Y: h as f32 });
        // La línea centrada en la franja de `t` px de afuera del marco.
        let _ = self.geom.SetOffset(Vector2 { X: t / 2.0, Y: t / 2.0 });
        let _ = self.geom.SetSize(Vector2 { X: w as f32 - t, Y: h as f32 - t });
        let _ = self.geom.SetCornerRadius(Vector2 { X: r + t / 2.0, Y: r + t / 2.0 });
        if let Ok(shapes) = self.visual.Shapes()
            && let Ok(shape) = shapes.GetAt(0)
            && let Ok(sprite) = shape.cast::<windows::UI::Composition::CompositionSpriteShape>()
        {
            let _ = sprite.SetStrokeThickness(t);
        }
    }
}

impl Drop for FocusBorder {
    fn drop(&mut self) {
        self.unhook();
        TARGET.with(|t| t.set(0));
        // SAFETY: ventana propia de este hilo.
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// Para `ci-probe`: el rectángulo del borde de foco del proceso `pid`, si se
/// ve.
pub fn shown_rect(pid: u32) -> Option<wm_model::Rect> {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, IsWindowVisible};
    let mut after: Option<HWND> = None;
    loop {
        // SAFETY: búsqueda por clase y título; sin efectos.
        let h =
            unsafe { FindWindowExW(None, after, w!("wm-border"), w!("wm borde de foco")) }.ok()?;
        let id = WindowId(h.0 as isize);
        // SAFETY: lectura sin efectos.
        if crate::windows_list::window_pid(id) == pid && unsafe { IsWindowVisible(h) }.as_bool() {
            return crate::ghost::window_rect(id);
        }
        after = Some(h);
    }
}
