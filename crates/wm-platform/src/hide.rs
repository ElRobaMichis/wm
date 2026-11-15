//! Esconder y mostrar ventanas de otras apps (spike 2, architecture.md §6).
//!
//! Cuatro estrategias detrás de [`Hider`]:
//!
//! | Estrategia   | Documentada | Notas |
//! |--------------|-------------|-------|
//! | `cloak`      | No          | Cloaking del shell (`IApplicationView::SetCloak`). Sin parpadeo; la ventana sigue en la barra de tareas |
//! | `minimize`   | Sí          | Minimizar con las transiciones de DWM desactivadas |
//! | `offscreen`  | Sí          | Mover fuera de la pantalla virtual, sin cambiar el tamaño |
//! | `hide`       | Sí          | `SW_HIDE`: desaparece también de la barra de tareas |
//!
//! **Nunca perder ventanas** (architecture.md §9): antes de esconder, la marca
//! se guarda **en la propia ventana** con `SetPropW` (método y, si hace falta,
//! la posición original). [`restore_all`] la recupera aunque este proceso haya
//! muerto.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_TRANSITIONS_FORCEDISABLED, DwmGetWindowAttribute, DwmSetWindowAttribute,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IServiceProvider,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetPropW, GetSystemMetrics, GetWindowRect, IsIconic, IsWindowVisible,
    RemovePropW, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWMINNOACTIVE, SW_SHOWNA,
    SW_SHOWNOACTIVATE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetPropW,
    SetWindowPos, ShowWindowAsync,
};
use windows::core::{BOOL, GUID, HRESULT, Interface, PCWSTR, w};
use windows_core::{IUnknown, IUnknown_Vtbl, interface};

use crate::windows_list::WindowId;

// ---------------------------------------------------------------------------
// Interfaces COM no documentadas del shell.
//
// Definiciones tomadas de GlazeWM (https://github.com/glzr-io/glazewm,
// GPL-3.0), packages/wm-platform/src/platform_impl/windows/com.rs. Los métodos
// `m*` solo ocupan su lugar en la vtable.
// ---------------------------------------------------------------------------

const CLSID_IMMERSIVE_SHELL: GUID = GUID::from_u128(0xC2F03A33_21F5_47FA_B4BB_156362A2F239);

#[interface("1841c6d7-4f9d-42c0-af41-8747538f10e5")]
unsafe trait IApplicationViewCollection: IUnknown {
    unsafe fn m1(&self);
    unsafe fn m2(&self);
    unsafe fn m3(&self);
    unsafe fn get_view_for_hwnd(
        &self,
        window: isize,
        application_view: *mut Option<IApplicationView>,
    ) -> HRESULT;
}

#[interface("372E1D3B-38D3-42E4-A15B-8AB2B178F513")]
unsafe trait IApplicationView: IUnknown {
    unsafe fn m1(&self);
    unsafe fn m2(&self);
    unsafe fn m3(&self);
    unsafe fn m4(&self);
    unsafe fn m5(&self);
    unsafe fn m6(&self);
    unsafe fn m7(&self);
    unsafe fn m8(&self);
    unsafe fn m9(&self);
    unsafe fn set_cloak(&self, cloak_type: u32, cloak_flag: i32) -> HRESULT;
}

/// Valor de `DWMWA_CLOAKED` cuando el que esconde es el shell.
pub const DWM_CLOAKED_SHELL: u32 = 0x2;

// ---------------------------------------------------------------------------
// Estrategias
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HideMethod {
    ShellCloak,
    Minimize,
    Offscreen,
    Hide,
}

impl HideMethod {
    pub const ALL: [HideMethod; 4] =
        [HideMethod::ShellCloak, HideMethod::Minimize, HideMethod::Offscreen, HideMethod::Hide];

    pub fn name(self) -> &'static str {
        match self {
            HideMethod::ShellCloak => "cloak",
            HideMethod::Minimize => "minimize",
            HideMethod::Offscreen => "offscreen",
            HideMethod::Hide => "hide",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    fn code(self) -> usize {
        match self {
            HideMethod::ShellCloak => 1,
            HideMethod::Minimize => 2,
            HideMethod::Offscreen => 3,
            HideMethod::Hide => 4,
        }
    }

    fn from_code(code: usize) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.code() == code)
    }
}

const PROP_METHOD: PCWSTR = w!("wm.hidden");
const PROP_X: PCWSTR = w!("wm.x");
const PROP_Y: PCWSTR = w!("wm.y");

/// Bit que se suma a las coordenadas guardadas para que un valor 0 no se
/// confunda con "propiedad inexistente" (`GetPropW` devuelve 0 en ese caso).
/// Bug encontrado en la prueba: una ventana en (0, 0) quedaba afuera.
const PRESENT: usize = 1 << 32;

/// Dónde dejar una ventana si se perdió su posición original: dentro del
/// monitor principal, que siempre empieza en (0, 0).
const FALLBACK_POS: (i32, i32) = (50, 50);

/// Esconde y muestra ventanas con un método fijo.
pub struct Hider {
    method: HideMethod,
    views: Option<IApplicationViewCollection>,
}

impl Hider {
    /// Prepara el método. Para `cloak` inicializa COM en este hilo y obtiene la
    /// interfaz del shell; si no existe (otra versión de Windows), falla.
    pub fn new(method: HideMethod) -> Result<Self, String> {
        let views = if method == HideMethod::ShellCloak { Some(view_collection()?) } else { None };
        Ok(Self { method, views })
    }

    pub fn method(&self) -> HideMethod {
        self.method
    }

    pub fn hide(&self, id: WindowId) -> Result<(), String> {
        let hwnd = hwnd(id);
        // Si ya estaba escondida (por ejemplo, por una prueba anterior con otro
        // método), primero se muestra con su método original: pisar la marca
        // perdería la información para recuperarla.
        if let Some(previous) = marked_method(hwnd) {
            if previous == self.method {
                return Ok(());
            }
            show_marked(hwnd, self.views.as_ref())?;
        }
        let mut pos = None;
        if self.method == HideMethod::Offscreen {
            let rect = window_rect(hwnd).ok_or("no se pudo leer la posición")?;
            pos = Some((rect.left, rect.top));
        }
        mark(hwnd, self.method, pos)?;
        let result = match self.method {
            HideMethod::ShellCloak => self.cloak(hwnd, true),
            HideMethod::Minimize => {
                disable_transitions(hwnd);
                show_async(hwnd, SW_SHOWMINNOACTIVE.0)
            }
            HideMethod::Offscreen => {
                // Más allá del borde superior izquierdo de la pantalla virtual.
                // SAFETY: sin precondiciones.
                let (vx, vy) = unsafe {
                    (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN))
                };
                let rect = window_rect(hwnd).unwrap_or_default();
                let x = vx - (rect.right - rect.left) - 100;
                let y = vy - (rect.bottom - rect.top) - 100;
                move_async(hwnd, x, y)
            }
            HideMethod::Hide => show_async(hwnd, SW_HIDE.0),
        };
        if result.is_err() {
            unmark(hwnd);
        }
        result
    }

    pub fn show(&self, id: WindowId) -> Result<(), String> {
        show_marked(hwnd(id), self.views.as_ref())
    }

    fn cloak(&self, hwnd: HWND, cloaked: bool) -> Result<(), String> {
        let views = self.views.as_ref().ok_or("el cloaking del shell no está disponible")?;
        set_cloak(views, hwnd, cloaked)
    }
}

fn view_collection() -> Result<IApplicationViewCollection, String> {
    // SAFETY: inicializa COM en este hilo (STA, requerido por el shell). Si ya
    // estaba inicializado con el mismo modelo, devuelve S_FALSE y está bien.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    // SAFETY: CLSID del shell; la interfaz pedida es IServiceProvider.
    let provider: IServiceProvider =
        unsafe { CoCreateInstance(&CLSID_IMMERSIVE_SHELL, None, CLSCTX_ALL) }
            .map_err(|e| format!("no se pudo crear el ImmersiveShell: {e}"))?;
    // SAFETY: llamada COM estándar sobre una interfaz válida.
    unsafe { provider.QueryService(&IApplicationViewCollection::IID) }
        .map_err(|e| format!("Windows no expone IApplicationViewCollection: {e}"))
}

fn set_cloak(views: &IApplicationViewCollection, hwnd: HWND, cloaked: bool) -> Result<(), String> {
    let mut view: Option<IApplicationView> = None;
    // SAFETY: interfaz válida y puntero a una variable local. Es una llamada
    // COM bloqueante a explorer.exe (architecture.md §6.2).
    unsafe { views.get_view_for_hwnd(hwnd.0 as isize, &mut view) }
        .ok()
        .map_err(|e| format!("la ventana no tiene vista del shell: {e}"))?;
    let view = view.ok_or("la ventana no tiene vista del shell")?;
    // SAFETY: interfaz válida; argumentos (1, 2) = esconder, (1, 0) = mostrar.
    unsafe { view.set_cloak(1, if cloaked { 2 } else { 0 }) }
        .ok()
        .map_err(|e| format!("SetCloak falló: {e}"))
}

// ---------------------------------------------------------------------------
// Marcas en la ventana y recuperación
// ---------------------------------------------------------------------------

fn mark(hwnd: HWND, method: HideMethod, pos: Option<(i32, i32)>) -> Result<(), String> {
    // SAFETY: SetPropW funciona entre procesos y con la ventana colgada
    // (architecture.md §0). El valor es un entero guardado como HANDLE.
    unsafe {
        if let Some((x, y)) = pos {
            for (name, value) in [(PROP_X, x), (PROP_Y, y)] {
                let stored = (value as u32 as usize) | PRESENT;
                SetPropW(hwnd, name, Some(HANDLE(stored as *mut c_void)))
                    .map_err(|e| format!("SetPropW: {e}"))?;
            }
        }
        SetPropW(hwnd, PROP_METHOD, Some(HANDLE(method.code() as *mut c_void)))
            .map_err(|e| format!("SetPropW: {e}"))
    }
}

fn unmark(hwnd: HWND) {
    // SAFETY: quitar propiedades propias; si no existen, falla sin efectos.
    unsafe {
        let _ = RemovePropW(hwnd, PROP_METHOD);
        let _ = RemovePropW(hwnd, PROP_X);
        let _ = RemovePropW(hwnd, PROP_Y);
    }
}

fn marked_method(hwnd: HWND) -> Option<HideMethod> {
    // SAFETY: lectura de una propiedad; 0 si no existe.
    let value = unsafe { GetPropW(hwnd, PROP_METHOD) };
    HideMethod::from_code(value.0 as usize)
}

fn marked_pos(hwnd: HWND) -> Option<(i32, i32)> {
    let read = |name: PCWSTR| -> Option<i32> {
        // SAFETY: lectura de una propiedad; 0 si no existe.
        let value = unsafe { GetPropW(hwnd, name) }.0 as usize;
        (value & PRESENT != 0).then_some(value as u32 as i32)
    };
    Some((read(PROP_X)?, read(PROP_Y)?))
}

/// Muestra una ventana según el método con que se escondió (leído de su
/// marca) y quita la marca. Sin marca, no hace nada.
fn show_marked(hwnd: HWND, views: Option<&IApplicationViewCollection>) -> Result<(), String> {
    let Some(method) = marked_method(hwnd) else { return Ok(()) };
    let result = match method {
        HideMethod::ShellCloak => match views {
            Some(v) => set_cloak(v, hwnd, false),
            None => set_cloak(&view_collection()?, hwnd, false),
        },
        HideMethod::Minimize => show_async(hwnd, SW_SHOWNOACTIVATE.0),
        HideMethod::Offscreen => {
            // Sin la posición original, igual se trae la ventana a la
            // pantalla: nunca dejarla afuera (architecture.md §9).
            let (x, y) = marked_pos(hwnd).unwrap_or(FALLBACK_POS);
            move_async(hwnd, x, y)
        }
        HideMethod::Hide => show_async(hwnd, SW_SHOWNA.0),
    };
    if result.is_ok() {
        unmark(hwnd);
    }
    result
}

/// Resultado de [`restore_all`].
#[derive(Debug, Default)]
pub struct RestoreReport {
    pub restored: Vec<(WindowId, HideMethod)>,
    pub failed: Vec<(WindowId, String)>,
}

/// Busca en **todas** las ventanas top-level (también las escondidas) las que
/// tienen nuestra marca y las vuelve a mostrar. No depende de la memoria de
/// ningún proceso: funciona aunque el gestor se haya caído.
pub fn restore_all() -> RestoreReport {
    let mut handles: Vec<HWND> = Vec::new();

    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` es el `&mut Vec` que pasamos abajo; EnumWindows llama
        // de forma síncrona en este hilo.
        let handles = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        handles.push(hwnd);
        true.into()
    }

    // SAFETY: el callback solo usa `data` como el `&mut Vec` que le pasamos.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut handles as *mut Vec<HWND> as isize)) };

    let mut views: Option<IApplicationViewCollection> = None;
    let mut report = RestoreReport::default();
    for hwnd in handles {
        let Some(method) = marked_method(hwnd) else { continue };
        let id = WindowId(hwnd.0 as isize);
        if method == HideMethod::ShellCloak && views.is_none() {
            views = view_collection().ok();
        }
        match show_marked(hwnd, views.as_ref()) {
            Ok(()) => report.restored.push((id, method)),
            Err(e) => report.failed.push((id, e)),
        }
    }
    report
}

// ---------------------------------------------------------------------------
// Estado observable (para verificar en las pruebas)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub visible: bool,
    pub minimized: bool,
    /// Valor de `DWMWA_CLOAKED` (0 = no; 2 = shell).
    pub cloaked: u32,
    pub left: i32,
    pub top: i32,
}

pub fn visibility(id: WindowId) -> Visibility {
    let hwnd = hwnd(id);
    let mut cloaked = 0u32;
    // SAFETY: consultas de solo lectura; no bloquean con apps colgadas.
    unsafe {
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        let rect = window_rect(hwnd).unwrap_or_default();
        Visibility {
            visible: IsWindowVisible(hwnd).as_bool(),
            minimized: IsIconic(hwnd).as_bool(),
            cloaked,
            left: rect.left,
            top: rect.top,
        }
    }
}

/// ¿El estado observado corresponde a "escondida" con este método?
pub fn looks_hidden(method: HideMethod, v: Visibility) -> bool {
    match method {
        HideMethod::ShellCloak => v.cloaked == DWM_CLOAKED_SHELL,
        HideMethod::Minimize => v.minimized,
        HideMethod::Offscreen => {
            // SAFETY: sin precondiciones.
            let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
            // SAFETY: sin precondiciones.
            let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
            v.left < vx || v.top < vy
        }
        HideMethod::Hide => !v.visible,
    }
}

// ---------------------------------------------------------------------------
// Utilidades
// ---------------------------------------------------------------------------

fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as *mut c_void)
}

fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    // SAFETY: puntero a una variable local válida.
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok().map(|()| rect)
}

fn show_async(hwnd: HWND, cmd: i32) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD;
    // SAFETY: ShowWindowAsync postea el pedido al hilo dueño: no bloquea
    // aunque la app esté colgada (architecture.md §4.4). El valor de retorno
    // indica si la ventana estaba visible antes, no un error.
    let _ = unsafe { ShowWindowAsync(hwnd, SHOW_WINDOW_CMD(cmd)) };
    Ok(())
}

fn move_async(hwnd: HWND, x: i32, y: i32) -> Result<(), String> {
    let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
    // SAFETY: SWP_ASYNCWINDOWPOS evita bloquearnos con apps colgadas.
    unsafe { SetWindowPos(hwnd, None, x, y, 0, 0, flags) }.map_err(|e| format!("SetWindowPos: {e}"))
}

fn disable_transitions(hwnd: HWND) {
    let disabled = BOOL::from(true);
    // SAFETY: atributo aceptado entre procesos (architecture.md §0, sonda).
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &disabled as *const BOOL as *const c_void,
            size_of::<BOOL>() as u32,
        )
    };
}
