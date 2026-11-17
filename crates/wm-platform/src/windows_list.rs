//! Enumerar las ventanas "de usuario" y traer una al frente.

use std::ffi::c_void;
use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindow,
    GetWindowLongPtrW, GetWindowThreadProcessId, InternalGetWindowText, IsIconic, IsWindowVisible,
    SW_RESTORE, SetForegroundWindow, ShowWindowAsync, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

/// Identificador de ventana que se puede pasar entre hilos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub isize);

impl WindowId {
    pub(crate) fn hwnd(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: WindowId,
    /// Proceso dueño: con el HWND identifica la ventana aunque Windows
    /// reutilice el HWND (plan-core.md §1.1).
    pub pid: u32,
    pub title: String,
    /// Nombre del ejecutable sin extensión, en minúsculas (por ejemplo "chrome").
    pub app: String,
    /// La escondimos nosotros (tiene nuestra marca): con cloak, minimizada,
    /// fuera de pantalla u oculta. Sigue siendo del usuario.
    pub hidden: bool,
    /// Escritorio anotado en la ventana (`wm.desk`, hito 4).
    pub desk: Option<u8>,
    /// Tiene cloak (de quien sea) ahora mismo.
    pub cloaked: bool,
    pub minimized: bool,
    /// El ejecutable de su familia, si es otro (plan-apps.md §2).
    pub family: Option<String>,
    /// AUMID del proceso (apps MSIX y de la Store).
    pub aumid: Option<String>,
    /// Se puede redimensionar y maximizar: una ventana principal (no una
    /// pantalla de carga ni un diálogo). v1.1.
    pub main: bool,
}

/// Hito 4d: una ventana vista al reconciliar.
#[derive(Debug, Clone)]
pub struct SurveyWin {
    pub info: WindowInfo,
    /// Marco visible (DWM; vale también con cloak).
    pub frame: Option<wm_model::Rect>,
    /// Rectángulo de la ventana (con los bordes invisibles).
    pub outer: Option<wm_model::Rect>,
    pub maximized: bool,
    /// Escondida por nosotros fuera de pantalla u oculta: su lugar no cuenta.
    pub parked: bool,
}

/// Hito 4d: monitores y ventanas en una sola observación.
#[derive(Debug, Clone)]
pub struct Survey {
    pub monitors: Vec<wm_model::place::MonitorObs>,
    pub wins: Vec<SurveyWin>,
}

/// Hito 4d: la observación para reconciliar. Solo lecturas que no envían
/// mensajes a las apps (una colgada no la frena).
pub fn survey() -> Result<Survey, String> {
    use crate::hide::HideMethod;
    let monitors = crate::displays::topology().map_err(|e| e.message())?;
    let wins = user_windows()
        .into_iter()
        .map(|info| {
            let id = info.id;
            SurveyWin {
                frame: crate::ghost::frame_bounds(id),
                outer: crate::ghost::window_rect(id),
                maximized: crate::hide::is_maximized(id),
                parked: matches!(
                    crate::hide::hidden_with(id),
                    Some(HideMethod::Offscreen | HideMethod::Hide)
                ),
                info,
            }
        })
        .collect();
    Ok(Survey { monitors, wins })
}

/// Hito 4d: correr una ventana a `(x, y)` (rectángulo exterior) sin cambiar
/// el tamaño, el orden Z ni el foco. Asíncrono: no espera a la app.
pub fn move_async(id: WindowId, x: i32, y: i32) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    // SAFETY: HWND de otra app; asíncrono (no envía mensajes sincrónicos).
    unsafe {
        SetWindowPos(
            id.hwnd(),
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    }
    .map_err(|e| e.message())
}

/// Ventanas que un usuario consideraría "abiertas" (las del Alt+Tab), en
/// orden Z: la primera es la de más arriba. Incluye las que escondimos
/// nosotros (en otro escritorio), con `hidden`; no las que escondió otro
/// (escritorios virtuales de Windows, apps UWP suspendidas).
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

    // Primero se descartan las ventanas propias: leer algo de una ventana
    // nuestra desde otro hilo puede esperar a ese hilo.
    let own_pid = std::process::id();
    let mut list: Vec<WindowInfo> = handles
        .into_iter()
        .filter_map(|h| {
            let mut pid = 0u32;
            // SAFETY: `h` es un HWND de EnumWindows; `pid` es una variable local.
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
            if pid == own_pid {
                return None;
            }
            let id = WindowId(h.0 as isize);
            let hidden = crate::hide::hidden_with(id).is_some();
            if !hidden && !is_user_window(h) {
                return None;
            }
            let title = title(h);
            // Una escondida por nosotros nunca se descarta (quedaría fuera
            // del modelo y no volvería a su escritorio).
            if title.is_empty() && !hidden {
                return None;
            }
            let app = match process_name(pid) {
                Some(a) => a,
                None if hidden => String::new(),
                None => return None,
            };
            let v = crate::hide::visibility(id);
            Some(WindowInfo {
                id,
                pid,
                title,
                app,
                hidden,
                desk: crate::hide::desk_of(id),
                cloaked: v.cloaked != 0,
                minimized: v.minimized,
                family: None,
                aumid: None,
                main: is_main_window(h),
            })
        })
        .collect();
    // Familia y AUMID, con una sola foto de procesos (y solo si hay alguno
    // nuevo: el resto sale de la caché).
    let mut pids: Vec<u32> = list.iter().map(|w| w.pid).collect();
    pids.sort_unstable();
    pids.dedup();
    let infos = crate::process_info::infos(&pids);
    for w in &mut list {
        if let Some(i) = infos.get(&w.pid) {
            w.family = i.family.clone();
            w.aumid = i.aumid.clone();
        }
    }
    list
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
        // Las ventanas fantasma ("No responde") son de dwm.exe y tapan a la
        // de la app colgada: se ignoran (architecture.md §4.4).
        let mut class = [0u16; 8];
        let len = GetClassNameW(h, &mut class);
        if String::from_utf16_lossy(&class[..len.max(0) as usize]) == "Ghost" {
            return false;
        }
        let mut cloaked = 0u32;
        let ok = DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        );
        !(ok.is_ok() && cloaked != 0)
    }
}

/// ¿Ventana principal? Borde para cambiar el tamaño y botón de maximizar
/// (las pantallas de carga y los diálogos no los tienen). Solo lee el estilo.
fn is_main_window(h: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GWL_STYLE, WS_MAXIMIZEBOX, WS_THICKFRAME};
    // SAFETY: lectura del estilo de un HWND; no envía mensajes.
    let style = unsafe { GetWindowLongPtrW(h, GWL_STYLE) } as u32;
    style & WS_THICKFRAME.0 != 0 && style & WS_MAXIMIZEBOX.0 != 0
}

/// ¿Es una ventana "de usuario" (las del Alt+Tab)?
pub fn is_user(id: WindowId) -> bool {
    is_user_window(id.hwnd())
}

/// "Borde de foco": color del borde de la ventana (`DWMWA_BORDER_COLOR`,
/// Windows 11; en Windows 10 falla sin efecto). `None`: el de Windows.
pub fn set_border(id: WindowId, color: Option<u32>) {
    use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DwmSetWindowAttribute};
    const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;
    let value = color.unwrap_or(DWMWA_COLOR_DEFAULT);
    // SAFETY: un u32 local; atributo documentado (no envía mensajes a la app).
    let _ = unsafe {
        DwmSetWindowAttribute(
            id.hwnd(),
            DWMWA_BORDER_COLOR,
            &value as *const u32 as *const c_void,
            size_of::<u32>() as u32,
        )
    };
}

/// El color del borde (`DWMWA_BORDER_COLOR`), si Windows lo informa.
pub fn border_color(id: WindowId) -> Option<u32> {
    use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DwmGetWindowAttribute};
    let mut v = 0u32;
    // SAFETY: variable local con su tamaño.
    unsafe {
        DwmGetWindowAttribute(
            id.hwnd(),
            DWMWA_BORDER_COLOR,
            &mut v as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    }
    .ok()
    .map(|()| v)
}

/// "El puntero sigue al foco": lleva el puntero al centro de la ventana, si
/// no está ya adentro.
pub fn pointer_to(id: WindowId) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
    let Some(f) = crate::ghost::frame_bounds(id) else { return };
    let mut p = POINT::default();
    // SAFETY: variable local.
    if unsafe { GetCursorPos(&mut p) }.is_ok()
        && p.x >= f.left
        && p.x < f.right
        && p.y >= f.top
        && p.y < f.bottom
    {
        return;
    }
    // SAFETY: sin precondiciones.
    let _ = unsafe { SetCursorPos((f.left + f.right) / 2, (f.top + f.bottom) / 2) };
}

/// El dueño raíz (`GA_ROOTOWNER`): para un diálogo, su ventana principal.
pub fn root_owner(id: WindowId) -> WindowId {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOTOWNER, GetAncestor};
    // SAFETY: lectura sobre un HWND; si no existe, devuelve nulo.
    let h = unsafe { GetAncestor(id.hwnd(), GA_ROOTOWNER) };
    if h.is_invalid() { id } else { WindowId(h.0 as isize) }
}

/// La ventana del escritorio de Windows (`Progman`), para dejar el foco ahí
/// cuando el escritorio de destino está vacío.
pub fn shell_window() -> Option<WindowId> {
    use windows::Win32::UI::WindowsAndMessaging::GetShellWindow;
    // SAFETY: sin precondiciones.
    let h = unsafe { GetShellWindow() };
    (!h.is_invalid()).then_some(WindowId(h.0 as isize))
}

/// Título de una ventana (vacío si no tiene).
pub fn window_title(id: WindowId) -> String {
    title(HWND(id.0 as *mut core::ffi::c_void))
}

fn title(h: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: buffer local válido. InternalGetWindowText lee el texto que
    // guarda el sistema y nunca envía WM_GETTEXT, ni siquiera a ventanas
    // propias (GetWindowTextW sí lo hace con las del mismo proceso).
    let len = unsafe { InternalGetWindowText(h, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Como [`place_async`], pero además la sube al frente del orden Z (sin
/// activarla), en la misma llamada. Lo usa la animación con fantasma: el
/// fantasma se ve encima de todo, así que la real tiene que quedar arriba al
/// terminar (spike 6).
pub fn place_async_top(id: WindowId, rect: wm_model::Rect) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOP, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SetWindowPos,
    };
    let flags = SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
    // SAFETY: con SWP_ASYNCWINDOWPOS el pedido se postea al hilo dueño.
    unsafe {
        SetWindowPos(
            id.hwnd(),
            Some(HWND_TOP),
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            flags,
        )
    }
    .map_err(|e| format!("SetWindowPos: {e}"))
}

/// Le pide a la ventana que se cierre (`WM_CLOSE`, sin esperar).
pub fn close(id: WindowId) {
    use windows::Win32::Foundation::WPARAM;
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    // SAFETY: PostMessageW no bloquea; si la ventana ya no existe, falla.
    let _ = unsafe { PostMessageW(Some(id.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0)) };
}

/// Mueve y redimensiona sin esperar a la app (`SWP_ASYNCWINDOWPOS`).
pub fn place_async(id: WindowId, rect: wm_model::Rect) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    };
    let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
    // SAFETY: con SWP_ASYNCWINDOWPOS el pedido se postea al hilo dueño.
    unsafe {
        SetWindowPos(id.hwnd(), None, rect.left, rect.top, rect.width(), rect.height(), flags)
    }
    .map_err(|e| format!("SetWindowPos: {e}"))
}

pub(crate) fn process_name(pid: u32) -> Option<String> {
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

/// Proceso dueño de la ventana (0 si ya no existe).
pub fn window_pid(id: WindowId) -> u32 {
    let mut pid = 0u32;
    // SAFETY: si la ventana ya no existe, `pid` queda en 0.
    unsafe { GetWindowThreadProcessId(id.hwnd(), Some(&mut pid)) };
    pid
}

/// Lo que el core necesita saber de una ventana para acomodarla
/// (`wm_model::world::Obs`).
#[derive(Debug, Clone)]
pub struct Inspection {
    pub id: WindowId,
    pub pid: u32,
    pub app: String,
    pub title: String,
    /// Marco visible (`DWMWA_EXTENDED_FRAME_BOUNDS`).
    pub frame: wm_model::Rect,
    /// `GetWindowRect`, con los bordes invisibles.
    pub outer: wm_model::Rect,
    pub dpi: u32,
    pub maximized: bool,
    pub minimized: bool,
    pub monitors: Vec<crate::MonitorInfo>,
    /// Por qué no se puede animar (`None`: se puede).
    pub no_fly: Option<wm_model::world::NoFly>,
    /// v1.1: dónde está el puntero (`None`: no se pudo leer).
    pub pointer: Option<(i32, i32)>,
}

/// Por qué no animar esta ventana (hito 1e): lecturas que no envían
/// mensajes a la app, así que no bloquean aunque esté colgada.
fn no_fly(id: WindowId) -> Option<wm_model::world::NoFly> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowDisplayAffinity, IsHungAppWindow};
    use wm_model::world::NoFly;
    // El vuelo sube la ventana al frente: solo se anima la que ya lo está.
    if foreground() != Some(id) {
        return Some(NoFly::NotFront);
    }
    // SAFETY: lectura sin efectos sobre un HWND.
    if unsafe { IsHungAppWindow(id.hwnd()) }.as_bool() {
        return Some(NoFly::Hung);
    }
    let mut affinity = 0u32;
    // SAFETY: variable local válida.
    if unsafe { GetWindowDisplayAffinity(id.hwnd(), &mut affinity) }.is_ok() && affinity != 0 {
        return Some(NoFly::Protected);
    }
    // Se lee en cada observación: no depende de WM_SETTINGCHANGE.
    if reduced_motion() {
        return Some(NoFly::ReducedMotion);
    }
    None
}

/// ¿El sistema pide reducir animaciones? (Configuración > Accesibilidad >
/// Efectos de animación; en Windows Server viene así por defecto.)
pub fn reduced_motion() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut animate = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION escribe un BOOL en la variable local.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut animate as *mut BOOL as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && !animate.as_bool()
}

/// Observa una ventana. `None` = la que tiene el foco, si es una ventana de
/// usuario de otro proceso (si no, error: no se acomoda). Con `(id, pid)`,
/// falla si el HWND ahora es de otro proceso. Solo lecturas que no se traban
/// con apps colgadas (architecture.md §13.6).
pub fn inspect(target: Option<(WindowId, u32)>) -> Result<Inspection, String> {
    let id = match target {
        Some((id, pid)) => {
            if window_pid(id) != pid {
                return Err("la ventana ya no existe".to_owned());
            }
            id
        }
        None => {
            let id = foreground().ok_or("no hay ninguna ventana al frente")?;
            let pid = window_pid(id);
            // Con nuestra marca (por ejemplo, en vuelo: con cloak) sigue
            // siendo una ventana de usuario.
            let ours = crate::hide::hidden_with(id).is_some();
            if pid == std::process::id() || (!ours && !is_user_window(id.hwnd())) {
                return Err("la ventana al frente no se acomoda".to_owned());
            }
            id
        }
    };
    let pid = window_pid(id);
    let frame = crate::ghost::frame_bounds(id).ok_or("no se pudo leer el marco")?;
    let outer = crate::ghost::window_rect(id).ok_or("no se pudo leer la ventana")?;
    // SAFETY: lectura sin efectos sobre un HWND (si ya no existe, falso).
    let minimized = unsafe { IsIconic(id.hwnd()) }.as_bool();
    Ok(Inspection {
        id,
        pid,
        app: process_name(pid).unwrap_or_default(),
        title: title(id.hwnd()),
        frame,
        outer,
        dpi: crate::geometry::window_dpi(id),
        maximized: crate::hide::is_maximized(id),
        minimized,
        monitors: crate::monitors(),
        no_fly: no_fly(id),
        pointer: cursor(),
    })
}

/// La posición del puntero (`None` en el escritorio seguro, por ejemplo).
fn cursor() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    // SAFETY: escribe en una variable local.
    unsafe { GetCursorPos(&mut p) }.ok().map(|()| (p.x, p.y))
}

/// La ventana top-level visible bajo el punto (para pruebas).
pub fn root_at(x: i32, y: i32) -> Option<WindowId> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, WindowFromPoint};
    // SAFETY: lecturas sin efectos.
    let h = unsafe { WindowFromPoint(POINT { x, y }) };
    if h.is_invalid() {
        return None;
    }
    // SAFETY: ídem.
    let root = unsafe { GetAncestor(h, GA_ROOT) };
    Some(WindowId(if root.is_invalid() { h.0 as isize } else { root.0 as isize }))
}

/// Hito 5: la ventana que se agarra con `Caps` + clic en `(x, y)`.
#[derive(Debug, Clone)]
pub struct GrabInfo {
    pub inspection: Inspection,
    /// Si está maximizada: su tamaño normal (exterior), para restaurarla bajo
    /// el cursor al arrastrarla.
    pub restore: Option<(i32, i32)>,
    /// Tiene borde para cambiar el tamaño (`WS_THICKFRAME`).
    pub resizable: bool,
}

/// La ventana top-level bajo el punto (no el dueño: si se agarra un diálogo,
/// se mueve el diálogo). Rechaza las nuestras, las escondidas, la barra, el
/// escritorio y las de herramienta. Solo lecturas que no se traban.
pub fn inspect_at(x: i32, y: i32) -> Result<GrabInfo, String> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GA_ROOT, GWL_STYLE, GetAncestor, GetWindowPlacement, WINDOWPLACEMENT, WS_THICKFRAME,
        WindowFromPoint,
    };
    // SAFETY: lectura sin efectos.
    let h = unsafe { WindowFromPoint(POINT { x, y }) };
    if h.is_invalid() {
        return Err("no hay ninguna ventana ahí".to_owned());
    }
    // SAFETY: lectura sobre un HWND válido.
    let root = unsafe { GetAncestor(h, GA_ROOT) };
    let root = if root.is_invalid() { h } else { root };
    let id = WindowId(root.0 as isize);
    let pid = window_pid(id);
    if pid == std::process::id() {
        return Err("es una ventana de wm".to_owned());
    }
    let mut class = [0u16; 32];
    // SAFETY: buffer local válido.
    let len = unsafe { GetClassNameW(root, &mut class) };
    let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
    if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    {
        return Err("ahí no hay una ventana (escritorio o barra de tareas)".to_owned());
    }
    // SAFETY: lectura sin efectos.
    let ex = unsafe { GetWindowLongPtrW(root, GWL_EXSTYLE) } as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 || class == "Ghost" {
        return Err("esa ventana no se acomoda".to_owned());
    }
    if crate::hide::visibility(id).cloaked != 0 {
        return Err("la ventana está escondida".to_owned());
    }
    let inspection = inspect(Some((id, pid)))?;
    // SAFETY: lectura sin efectos.
    let style = unsafe { GetWindowLongPtrW(root, GWL_STYLE) } as u32;
    let resizable = style & WS_THICKFRAME.0 != 0;
    let restore = if inspection.maximized {
        let mut wp =
            WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        // SAFETY: estructura local con su tamaño.
        unsafe { GetWindowPlacement(root, &mut wp) }.ok().map(|()| {
            let r = wp.rcNormalPosition;
            (r.right - r.left, r.bottom - r.top)
        })
    } else {
        None
    };
    Ok(GrabInfo { inspection, restore, resizable })
}

/// Saca una ventana de maximizada o minimizada sin activarla (asíncrono: se
/// postea al hilo de la app, que lo aplica antes que un `place_async`
/// posterior).
pub fn normalize(id: WindowId) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{SW_SHOWNOACTIVATE, ShowWindowAsync};
    // SAFETY: pedido asíncrono a un HWND; si no existe, falla sin efectos.
    let ok = unsafe { ShowWindowAsync(id.hwnd(), SW_SHOWNOACTIVATE) }.as_bool();
    // Devuelve si antes era visible, no si funcionó: solo es error si la
    // ventana ya no existe.
    if !ok && window_pid(id) == 0 { Err("la ventana ya no existe".to_owned()) } else { Ok(()) }
}

/// Maximiza una ventana, como su botón (asíncrono: Windows recuerda el
/// tamaño anterior para restaurarla).
pub fn maximize(id: WindowId) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{SW_MAXIMIZE, ShowWindowAsync};
    // SAFETY: pedido asíncrono a un HWND; si no existe, falla sin efectos.
    let ok = unsafe { ShowWindowAsync(id.hwnd(), SW_MAXIMIZE) }.as_bool();
    if !ok && window_pid(id) == 0 { Err("la ventana ya no existe".to_owned()) } else { Ok(()) }
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
