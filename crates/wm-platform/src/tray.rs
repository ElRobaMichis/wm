//! Ícono de la bandeja y la ventana `wm-ui` (hito 3b, plan-core.md §6).
//!
//! Un hilo propio (`wm-tray`), dueño de una ventana top-level oculta
//! `wm-ui`, porque:
//! - `Shell_NotifyIconW` puede tardar segundos con Explorer colgado: en el
//!   hilo de entrada (TIME_CRITICAL) Windows quitaría el hook;
//! - el menú es un bucle modal: en el hilo del mapa trabaría al mapa;
//! - "Salir" no puede depender del mapa (que puede estar apagado).
//!
//! El brain le habla con `PostMessageW` a la ventana; el hilo responde por
//! canal + `WM_OPS` (como `ops`, `anim` y `ui`). Abrir los ajustes e iniciar
//! con Windows los resuelve el propio hilo (son E/S).
//!
//! También recibe las órdenes de afuera (`wm quit|pause|resume|undo|ajustes`
//! y una segunda instancia que pide abrir los ajustes) con el mensaje
//! registrado `wm.command`, y `TaskbarCreated` (Explorer se reinició: el
//! ícono se vuelve a agregar).

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIIF_NOSOUND, NIIF_RESPECT_QUIET_TIME,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_DATA_FLAGS, NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW,
    GetSystemMetrics, HICON, ICONINFO, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SM_CXSMICON,
    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, TranslateMessage,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_ENDSESSION, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, s, w};
use wm_model::world::Command;

use crate::ops;

/// Mensaje del ícono (clics).
const WM_TRAY_ICON: u32 = WM_APP + 0x40;
/// El brain cambió el estado (pausa, deshacer): refrescar ícono y menú.
const WM_TRAY_SYNC: u32 = WM_APP + 0x41;
/// Cambió la ventana al frente (lParam: su HWND).
const WM_TRAY_FOREGROUND: u32 = WM_APP + 0x42;

/// Órdenes por `wm.command` (wParam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Order {
    Quit = 1,
    Pause = 2,
    Resume = 3,
    Undo = 4,
    Settings = 5,
    ToggleAutostart = 6,
    /// Hito 4c: todas las ventanas al escritorio activo (el botón de pánico).
    ShowAll = 7,
    /// Hito U: `wm update run` dejó una versión lista para colocar.
    UpdateReady = 8,
    /// Hito U: colocar la versión lista ahora (`wm actualizar`).
    UpdateNow = 9,
    /// Hito U: buscar actualizaciones ya.
    UpdateCheck = 10,
}

impl Order {
    fn from_code(c: usize) -> Option<Self> {
        Some(match c {
            1 => Self::Quit,
            2 => Self::Pause,
            3 => Self::Resume,
            4 => Self::Undo,
            5 => Self::Settings,
            6 => Self::ToggleAutostart,
            7 => Self::ShowAll,
            8 => Self::UpdateReady,
            9 => Self::UpdateNow,
            10 => Self::UpdateCheck,
            _ => return None,
        })
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "quit" | "salir" => Self::Quit,
            "pause" | "pausar" => Self::Pause,
            "resume" | "reanudar" => Self::Resume,
            "undo" | "deshacer" => Self::Undo,
            "settings" | "ajustes" => Self::Settings,
            "showall" | "mostrar" => Self::ShowAll,
            "actualizar" => Self::UpdateNow,
            _ => return None,
        })
    }
}

#[derive(Debug)]
pub enum TrayEvent {
    Ready(Result<(), String>),
    Command(Command),
    Log(String),
    /// Hito U: hay una versión lista (`wm update run` la avisó).
    UpdateReady,
    /// Hito U: colocarla ya (menú, globo o `wm actualizar`).
    UpdateNow,
    /// Hito U: buscar actualizaciones (temporizador o el menú; `forced`: el
    /// menú, sin el mínimo de 20 h).
    CheckUpdates {
        forced: bool,
    },
}

/// Hito U: la versión lista para colocar (texto para el menú) y un globo
/// pendiente.
static UPDATE_READY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static BALLOON: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);
/// Mostrar un globo (título y texto en `BALLOON`).
const WM_TRAY_BALLOON: u32 = WM_APP + 0x43;
/// Temporizador de búsqueda de actualizaciones (3 min y después cada 24 h).
const TIMER_UPDATE: usize = 7;
const NIN_BALLOONUSERCLICK: u32 = 0x0405;
use windows::Win32::UI::WindowsAndMessaging::{SetTimer, WM_TIMER};

/// Estado que muestra la bandeja (lo actualiza el brain).
static PAUSED: AtomicBool = AtomicBool::new(false);
static CAN_UNDO: AtomicBool = AtomicBool::new(false);
/// Escritorio activo (1-4) y ventanas de cada uno (8 bits cada uno), para
/// el número del ícono y el menú (hito 5e).
static DESKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn pack_desks(active: u8, counts: [usize; 4]) -> u64 {
    counts
        .iter()
        .enumerate()
        .fold(u64::from(active), |acc, (i, c)| acc | ((*c).min(255) as u64) << (8 * (i + 1)))
}

fn unpack_desks(v: u64) -> (u8, [usize; 4]) {
    let c = |i: u32| ((v >> (8 * (i + 1))) & 0xFF) as usize;
    ((v & 0xFF) as u8, [c(0), c(1), c(2), c(3)])
}
/// La ventana `wm-ui` (0: todavía no hay).
static WINDOW: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static LANE: std::cell::RefCell<Option<Lane>> = const { std::cell::RefCell::new(None) };
}

struct Lane {
    brain: u32,
    events: Sender<TrayEvent>,
    hwnd: HWND,
    /// El ícono actual y para qué (pausa, escritorio) se dibujó.
    icon: Option<HICON>,
    icon_key: (bool, u8),
    added: bool,
    taskbar_created: u32,
    command_msg: u32,
    settings: Option<std::process::Child>,
    /// Aviso de "atajos no disponibles": apps ya avisadas en esta sesión y
    /// cuándo fue el último (un globo por app, nunca dos en 60 s).
    warned: std::collections::HashSet<String>,
    last_warning: Option<std::time::Instant>,
    /// Con nosotros como administrador, los atajos llegan siempre.
    self_elevated: bool,
}

/// Nombre de la ventana `wm-ui` (con `WM_INSTANCE`, el de esa instancia).
fn window_name() -> HSTRING {
    HSTRING::from(crate::instance::mutex_name("ui"))
}

fn command_message() -> u32 {
    // SAFETY: cadena constante.
    unsafe { RegisterWindowMessageW(w!("wm.command")) }
}

/// El lado del brain.
pub struct Tray {
    events: Receiver<TrayEvent>,
}

impl Tray {
    /// Crea el hilo sin esperarlo; avisa con [`TrayEvent::Ready`].
    pub fn start(brain: u32) -> Self {
        let (tx, events) = mpsc::channel();
        let _ = std::thread::Builder::new().name("wm-tray".into()).spawn(move || run(brain, tx));
        Self { events }
    }

    pub fn drain(&mut self) -> Vec<TrayEvent> {
        self.events.try_iter().collect()
    }

    /// El brain cambió algo que se ve en la bandeja.
    pub fn sync(&self, paused: bool, can_undo: bool, desk: u8, counts: [usize; 4]) {
        let packed = pack_desks(desk, counts);
        let changed = (PAUSED.swap(paused, Ordering::Relaxed) != paused)
            | (CAN_UNDO.swap(can_undo, Ordering::Relaxed) != can_undo)
            | (DESKS.swap(packed, Ordering::Relaxed) != packed);
        if changed {
            post(WM_TRAY_SYNC, 0);
        }
    }

    /// Hito U: la versión lista para colocar (o ninguna), para el menú.
    pub fn set_update_ready(&self, version: Option<String>) {
        if let Ok(mut g) = UPDATE_READY.lock() {
            *g = version;
        }
    }

    /// Un globo de la bandeja (avisos de actualización).
    pub fn balloon(&self, title: &str, text: &str) {
        balloon(title, text);
    }

    /// Hito U: buscar actualizaciones 3 min después de arrancar y después una
    /// vez por día (un solo temporizador largo; 0 % de CPU en el medio).
    pub fn schedule_update_checks(&self) {
        post(WM_TRAY_BALLOON, 1);
    }

    /// Quita el ícono y cierra la ventana (salida).
    pub fn stop(&self) {
        post(WM_CLOSE, 0);
    }
}

/// Un globo de la bandeja desde cualquier hilo.
pub fn balloon(title: &str, text: &str) {
    if let Ok(mut g) = BALLOON.lock() {
        *g = Some((title.to_owned(), text.to_owned()));
    }
    post(WM_TRAY_BALLOON, 0);
}

fn post(msg: u32, wparam: usize) {
    let raw = WINDOW.load(Ordering::Acquire);
    if raw != 0 {
        // SAFETY: PostMessageW no bloquea; ventana propia.
        let _ = unsafe { PostMessageW(Some(HWND(raw as *mut _)), msg, WPARAM(wparam), LPARAM(0)) };
    }
}

/// Manda una orden a un `wm` que ya está corriendo (`wm quit`, una segunda
/// instancia). `false`: no hay ninguno.
pub fn send_order(order: Order) -> bool {
    send_order_to(&window_name(), order)
}

/// Como [`send_order`], a la instancia `instance` (`WM_INSTANCE`; para
/// `ci-probe`, que lanza su propio core).
pub fn send_order_in(instance: &str, order: Order) -> bool {
    send_order_to(&HSTRING::from(format!("Local\\wm.ui.{instance}")), order)
}

/// ¿Hay un `wm` corriendo en esta sesión (instancia normal, sin
/// `WM_INSTANCE`)? Lo usa el `ci-probe` para no pisarle los escritorios.
pub fn user_instance_running() -> bool {
    // SAFETY: nombres válidos; solo busca.
    unsafe { FindWindowW(w!("wm-ui"), &HSTRING::from("Local\\wm.ui")) }.is_ok()
}

fn send_order_to(name: &HSTRING, order: Order) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    // SAFETY: nombres válidos.
    let Ok(hwnd) = (unsafe { FindWindowW(w!("wm-ui"), name) }) else { return false };
    // Para que los ajustes que abra puedan venir al frente.
    // SAFETY: sin precondiciones.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    // SAFETY: PostMessageW no bloquea.
    unsafe { PostMessageW(Some(hwnd), command_message(), WPARAM(order as usize), LPARAM(0)) }
        .is_ok()
}

fn run(brain: u32, events: Sender<TrayEvent>) {
    let reply = |e: TrayEvent| {
        let _ = events.send(e);
        ops::wake(brain);
    };
    let hwnd = match create_window() {
        Ok(h) => h,
        Err(e) => return reply(TrayEvent::Ready(Err(format!("sin ventana wm-ui: {e}")))),
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Release);
    // SAFETY: cadena constante.
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let mut lane = Lane {
        brain,
        events: events.clone(),
        hwnd,
        icon: make_icon(false, 1).ok(),
        icon_key: (false, 1),
        added: false,
        taskbar_created,
        command_msg: command_message(),
        settings: None,
        warned: std::collections::HashSet::new(),
        last_warning: None,
        self_elevated: crate::session::self_elevated(),
    };
    lane.add_icon();
    watch_foreground();
    let ready = if lane.added { Ok(()) } else { Err("no se pudo agregar el ícono".to_owned()) };
    LANE.with(|l| *l.borrow_mut() = Some(lane));
    reply(TrayEvent::Ready(ready));
    let mut msg = MSG::default();
    // SAFETY: bucle de mensajes estándar.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        // SAFETY: mensaje recién obtenido.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    WINDOW.store(0, Ordering::Release);
}

fn create_window() -> windows::core::Result<HWND> {
    // SAFETY: clase y ventana propias.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance.into(),
            lpszClassName: w!("wm-ui"),
            ..Default::default()
        };
        RegisterClassW(&class);
        // Top-level oculta (no message-only): recibe `TaskbarCreated` y
        // `WM_ENDSESSION`, que son difusiones.
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("wm-ui"),
            &window_name(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
}

thread_local! {
    /// Mensajes que llegaron mientras se atendía otro (el menú es un bucle
    /// modal y `DestroyWindow` manda `WM_DESTROY` en el acto): se atienden
    /// al terminar, en vez de pedir el estado dos veces (eso era un pánico
    /// que abortaba el proceso al salir).
    static DEFERRED: std::cell::RefCell<Vec<(u32, WPARAM, LPARAM)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        // SAFETY: sin precondiciones.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    let handled = LANE.with(|l| {
        let Ok(mut guard) = l.try_borrow_mut() else {
            // Anidado: solo se guardan los mensajes propios.
            let ours = msg >= WM_APP || msg == WM_ENDSESSION || msg == WM_TIMER;
            if ours {
                DEFERRED.with(|d| d.borrow_mut().push((msg, wparam, lparam)));
            }
            return ours;
        };
        let Some(lane) = guard.as_mut() else { return false };
        let handled = lane.on_message(msg, wparam, lparam);
        // Los que llegaron mientras tanto.
        loop {
            // En el orden en que llegaron.
            let next = DEFERRED.with(|d| {
                let mut d = d.borrow_mut();
                (!d.is_empty()).then(|| d.remove(0))
            });
            let Some((m, w, p)) = next else { break };
            lane.on_message(m, w, p);
        }
        handled
    });
    if handled {
        return LRESULT(0);
    }
    // SAFETY: procesamiento por defecto.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Lane {
    fn send(&self, e: TrayEvent) {
        let _ = self.events.send(e);
        ops::wake(self.brain);
    }

    fn on_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        match msg {
            WM_TRAY_ICON => {
                match lparam.0 as u32 {
                    // Ella no hace clic derecho en la bandeja: el clic
                    // izquierdo abre el mismo menú, con los escritorios.
                    WM_LBUTTONUP => self.menu(),
                    WM_RBUTTONUP => self.menu(),
                    // Clic en el globo "Hay una versión nueva".
                    NIN_BALLOONUSERCLICK if UPDATE_READY.lock().is_ok_and(|g| g.is_some()) => {
                        self.send(TrayEvent::UpdateNow)
                    }
                    _ => {}
                }
                true
            }
            WM_TRAY_SYNC => {
                self.modify_icon();
                true
            }
            WM_TRAY_BALLOON if wparam.0 == 1 => {
                // SAFETY: temporizador de esta ventana.
                unsafe { SetTimer(Some(self.hwnd), TIMER_UPDATE, 3 * 60 * 1000, None) };
                true
            }
            WM_TRAY_BALLOON => {
                if let Some((title, text)) = BALLOON.lock().ok().and_then(|mut g| g.take()) {
                    self.show_balloon(&title, &text);
                }
                true
            }
            WM_TIMER if wparam.0 == TIMER_UPDATE => {
                // Después del primero, una vez por día.
                // SAFETY: temporizador de esta ventana.
                unsafe { SetTimer(Some(self.hwnd), TIMER_UPDATE, 24 * 3600 * 1000, None) };
                self.send(TrayEvent::CheckUpdates { forced: false });
                true
            }
            WM_TRAY_FOREGROUND => {
                self.foreground_changed(HWND(lparam.0 as *mut _));
                true
            }
            m if m == self.taskbar_created && m != 0 => {
                // Explorer se reinició: el ícono se perdió.
                self.added = false;
                self.add_icon();
                self.send(TrayEvent::Log(
                    "Explorer se reinició: ícono de la bandeja agregado de nuevo.".into(),
                ));
                // Las ventanas siguen con cloak, pero conviene revisar que
                // cada una esté como su escritorio pide (hito 4c).
                self.send(TrayEvent::Command(Command::Reconcile));
                true
            }
            m if m == self.command_msg && m != 0 => {
                if let Some(o) = Order::from_code(wparam.0) {
                    self.order(o);
                }
                true
            }
            WM_ENDSESSION if wparam.0 != 0 => {
                // Se cierra la sesión: salir prolijo (mostrar todo).
                self.send(TrayEvent::Command(Command::Quit));
                true
            }
            WM_CLOSE => {
                self.delete_icon();
                // SAFETY: ventana propia de este hilo.
                let _ = unsafe { DestroyWindow(self.hwnd) };
                true
            }
            _ => false,
        }
    }

    fn order(&mut self, o: Order) {
        match o {
            Order::Quit => self.send(TrayEvent::Command(Command::Quit)),
            Order::Pause => self.send(TrayEvent::Command(Command::SetPaused(true))),
            Order::Resume => self.send(TrayEvent::Command(Command::SetPaused(false))),
            Order::Undo => self.send(TrayEvent::Command(Command::Undo)),
            Order::ShowAll => self.send(TrayEvent::Command(Command::ShowAll)),
            Order::UpdateReady | Order::UpdateNow | Order::UpdateCheck => self.update_order(o),
            Order::Settings => self.open_settings(),
            Order::ToggleAutostart => {
                let on = crate::autostart::status() != Some(true);
                let line = match crate::autostart::set(on) {
                    Ok(()) if on => "Iniciar con Windows: activado.".to_owned(),
                    Ok(()) => "Iniciar con Windows: desactivado.".to_owned(),
                    Err(e) => format!("No se pudo cambiar el inicio con Windows: {e}"),
                };
                self.send(TrayEvent::Log(line));
            }
        }
    }

    /// Abre `wm-settings.exe` (al lado de `wm.exe`); si ya está abierto, no
    /// abre otro.
    fn open_settings(&mut self) {
        if self.settings.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))) {
            self.send(TrayEvent::Log("Los ajustes ya están abiertos.".into()));
            return;
        }
        if std::env::var_os("WM_SETTINGS").is_some_and(|v| v == "0") {
            self.send(TrayEvent::Log("Ajustes pedidos (WM_SETTINGS=0: no se abren).".into()));
            return;
        }
        let Some(exe) = std::env::current_exe().ok().map(|e| e.with_file_name("wm-settings.exe"))
        else {
            return;
        };
        match crate::launch::spawn_user(&exe) {
            Ok(child) => {
                self.settings = Some(child);
                self.send(TrayEvent::Log("Ajustes abiertos.".into()));
            }
            Err(e) => self.send(TrayEvent::Log(format!(
                "No se pudieron abrir los ajustes ({}): {e}",
                exe.display()
            ))),
        }
    }

    fn menu(&mut self) {
        const SETTINGS: usize = 1;
        const PAUSE: usize = 2;
        const UNDO: usize = 3;
        const AUTOSTART: usize = 4;
        const QUIT: usize = 5;
        const SHOW_ALL: usize = 6;
        const UPDATE_NOW: usize = 7;
        const UPDATE_CHECK: usize = 8;
        /// Escritorios: 11 a 14.
        const DESK_BASE: usize = 10;
        let paused = PAUSED.load(Ordering::Relaxed);
        let can_undo = CAN_UNDO.load(Ordering::Relaxed);
        let autostart = crate::autostart::status();
        // SAFETY: menú propio, se destruye al terminar.
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let ready = UPDATE_READY.lock().ok().and_then(|g| g.clone());
            if let Some(v) = &ready {
                let text: Vec<u16> =
                    format!("Actualizar ahora (versión {v})\0").encode_utf16().collect();
                let _ = AppendMenuW(menu, MF_STRING, UPDATE_NOW, PCWSTR(text.as_ptr()));
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            }
            let (active, counts) = unpack_desks(DESKS.load(Ordering::Relaxed));
            for n in 1..=4u8 {
                let c = counts[usize::from(n - 1)];
                let what = match c {
                    0 => "vacío".to_owned(),
                    1 => "1 ventana".to_owned(),
                    c => format!("{c} ventanas"),
                };
                let text: Vec<u16> =
                    format!("Escritorio {n}  ·  {what}\0").encode_utf16().collect();
                let flags = if n == active { MF_STRING | MF_CHECKED } else { MF_STRING };
                let _ = AppendMenuW(menu, flags, DESK_BASE + usize::from(n), PCWSTR(text.as_ptr()));
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, SETTINGS, w!("Ajustes…"));
            let pause = if paused { w!("Reanudar atajos") } else { w!("Pausar atajos") };
            let _ = AppendMenuW(menu, MF_STRING, PAUSE, pause);
            let undo_flags = if can_undo { MF_STRING } else { MF_STRING | MF_GRAYED };
            let _ = AppendMenuW(menu, undo_flags, UNDO, w!("Deshacer el último movimiento"));
            let _ = AppendMenuW(menu, MF_STRING, SHOW_ALL, w!("Mostrar todas las ventanas"));
            let auto_flags = match autostart {
                Some(true) => MF_STRING | MF_CHECKED,
                Some(false) => MF_STRING,
                None => MF_STRING | MF_GRAYED,
            };
            let _ = AppendMenuW(menu, auto_flags, AUTOSTART, w!("Iniciar con Windows"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let version: Vec<u16> =
                format!("Versión {} · buscar actualizaciones\0", env!("CARGO_PKG_VERSION"))
                    .encode_utf16()
                    .collect();
            let _ = AppendMenuW(menu, MF_STRING, UPDATE_CHECK, PCWSTR(version.as_ptr()));
            let _ = AppendMenuW(menu, MF_STRING, QUIT, w!("Salir"));
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            // Sin esto, el menú no se cierra al hacer clic afuera.
            let _ = SetForegroundWindow(self.hwnd);
            let chosen = TrackPopupMenuEx(
                menu,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
                p.x,
                p.y,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            match chosen.0 as usize {
                SETTINGS => self.order(Order::Settings),
                PAUSE => self.order(if paused { Order::Resume } else { Order::Pause }),
                UNDO => self.order(Order::Undo),
                SHOW_ALL => self.order(Order::ShowAll),
                UPDATE_NOW => self.order(Order::UpdateNow),
                UPDATE_CHECK => self.order(Order::UpdateCheck),
                id @ 11..=14 => {
                    self.send(TrayEvent::Command(Command::Desk((id - DESK_BASE) as u8)))
                }
                AUTOSTART => self.order(Order::ToggleAutostart),
                QUIT => self.order(Order::Quit),
                _ => {}
            }
        }
    }

    /// Cambió la ventana al frente: si es de una app que corre como
    /// administrador, el hook no recibe las teclas (UIPI) y los atajos no
    /// hacen nada. Un globo de la bandeja lo explica (una vez por app).
    fn foreground_changed(&mut self, hwnd: HWND) {
        if self.self_elevated || PAUSED.load(Ordering::Relaxed) || !self.added {
            return;
        }
        let id = crate::windows_list::WindowId(hwnd.0 as isize);
        let pid = crate::windows_list::window_pid(id);
        if pid == 0 || pid == std::process::id() || !crate::session::process_elevated(pid) {
            return;
        }
        let app = crate::windows_list::process_name(pid).unwrap_or_else(|| "esa app".into());
        let recent = self.last_warning.is_some_and(|t| t.elapsed().as_secs() < 60);
        if recent || !self.warned.insert(app.clone()) {
            return;
        }
        self.last_warning = Some(std::time::Instant::now());
        let mut d = self.data(NIF_INFO);
        put(&mut d.szInfoTitle, "Atajos no disponibles");
        put(
            &mut d.szInfo,
            &format!(
                "«{app}» se ejecuta como administrador: mientras esté al frente, Windows no deja que wm reciba las teclas."
            ),
        );
        d.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME;
        notify(NIM_MODIFY, &d);
        self.send(TrayEvent::Log(format!(
            "Atajos no disponibles: {app} se ejecuta como administrador (aviso en la bandeja)."
        )));
    }

    /// El ícono para el estado de ahora (se redibuja si cambió).
    fn current_icon(&mut self) -> HICON {
        let paused = PAUSED.load(Ordering::Relaxed);
        let (desk, _) = unpack_desks(DESKS.load(Ordering::Relaxed));
        let key = (paused, desk);
        if (self.icon.is_none() || self.icon_key != key)
            && let Ok(new) = make_icon(paused, desk)
        {
            if let Some(old) = self.icon.replace(new) {
                // SAFETY: ícono propio que ya no se usa.
                let _ = unsafe { DestroyIcon(old) };
            }
            self.icon_key = key;
        }
        self.icon.unwrap_or_default()
    }

    fn update_order(&mut self, o: Order) {
        match o {
            Order::UpdateReady => self.send(TrayEvent::UpdateReady),
            Order::UpdateNow => self.send(TrayEvent::UpdateNow),
            _ => self.send(TrayEvent::CheckUpdates { forced: true }),
        }
    }

    fn show_balloon(&mut self, title: &str, text: &str) {
        if !self.added {
            return;
        }
        let mut d = self.data(NIF_INFO);
        put(&mut d.szInfoTitle, title);
        put(&mut d.szInfo, text);
        d.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
        notify(NIM_MODIFY, &d);
    }

    fn data(&mut self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        let paused = PAUSED.load(Ordering::Relaxed);
        let (desk, _) = unpack_desks(DESKS.load(Ordering::Relaxed));
        let mut d = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: flags,
            uCallbackMessage: WM_TRAY_ICON,
            hIcon: self.current_icon(),
            ..Default::default()
        };
        let tip = if paused {
            format!("wm: atajos en pausa · escritorio {desk}")
        } else {
            format!("wm: escritorio {desk} · mantené Caps para ver los atajos")
        };
        put(&mut d.szTip, &tip);
        d
    }

    fn add_icon(&mut self) {
        let d = self.data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        self.added = notify(NIM_ADD, &d);
        if !self.added {
            // Explorer todavía no está (inicio de sesión): `TaskbarCreated`
            // llega cuando esté y se reintenta ahí.
            self.send(TrayEvent::Log(
                "La bandeja todavía no está: el ícono se agrega cuando aparezca.".into(),
            ));
        }
    }

    fn modify_icon(&mut self) {
        if !self.added {
            self.add_icon();
            return;
        }
        let d = self.data(NIF_ICON | NIF_TIP);
        notify(NIM_MODIFY, &d);
    }

    fn delete_icon(&mut self) {
        if self.added {
            let d = self.data(NOTIFY_ICON_DATA_FLAGS(0));
            notify(NIM_DELETE, &d);
            self.added = false;
        }
        if let Some(icon) = self.icon.take() {
            // SAFETY: ícono propio.
            let _ = unsafe { DestroyIcon(icon) };
        }
    }
}

/// Copia `text` a un búfer UTF-16 de tamaño fijo (recortado, con el 0 final).
fn put(buf: &mut [u16], text: &str) {
    let n = buf.len() - 1;
    let mut i = 0;
    for c in text.encode_utf16().take(n) {
        buf[i] = c;
        i += 1;
    }
    buf[i] = 0;
}

/// Ventana al frente cambió: `EVENT_SYSTEM_FOREGROUND` fuera de contexto,
/// que Windows despacha en el bucle de este hilo (0 en reposo, una llamada
/// por cambio de ventana). Se reenvía a `wm-ui` para atenderlo con el estado.
fn watch_foreground() {
    use windows::Win32::UI::Accessibility::SetWinEventHook;
    use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT};
    unsafe extern "system" fn changed(
        _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        let raw = WINDOW.load(Ordering::Acquire);
        if raw != 0 {
            // SAFETY: PostMessageW no bloquea; ventana propia.
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(raw as *mut _)),
                    WM_TRAY_FOREGROUND,
                    WPARAM(0),
                    LPARAM(hwnd.0 as isize),
                )
            };
        }
    }
    // SAFETY: callback con la firma de WINEVENTPROC; fuera de contexto.
    let _ = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(changed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
}

/// `Shell_NotifyIconW` resuelta en tiempo de ejecución: `wm.exe` no importa
/// shell32 (lo carga solo el core, la primera vez que hace falta).
fn notify(message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> bool {
    type Notify = unsafe extern "system" fn(u32, *const NOTIFYICONDATAW) -> i32;
    let Ok(f) = crate::dynlib::proc(w!("shell32.dll"), s!("Shell_NotifyIconW")) else {
        return false;
    };
    // SAFETY: firma documentada de Shell_NotifyIconW.
    let f: Notify = unsafe { std::mem::transmute(f) };
    // SAFETY: estructura local completa.
    unsafe { f(message.0, data) != 0 }
}

/// El ícono, dibujado en memoria (no hay archivo .ico en el repo): un
/// cuadrado redondeado con la mitad izquierda llena (una ventana acomodada
/// a la izquierda); en pausa, solo el contorno. Blanco sobre una barra
/// oscura, negro sobre una clara.
/// Dígitos 1-4 en 5 × 7 (filas de arriba a abajo; bit 4 = columna izquierda).
const DIGITS: [[u8; 7]; 4] = [
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
];

/// ¿El píxel `(x, y)` del ícono de lado `n` es parte del dígito `desk`?
fn digit_pixel(desk: u8, n: usize, x: usize, y: usize) -> bool {
    let Some(glyph) = DIGITS.get(usize::from(desk.max(1)) - 1) else { return false };
    let m = n / 8;
    let inner = n - 2 * m;
    let s = (inner / 9).max(1); // escala del dígito
    let (gw, gh) = (5 * s, 7 * s);
    let (ox, oy) = ((n - gw) / 2, (n - gh) / 2);
    if x < ox || y < oy || x >= ox + gw || y >= oy + gh {
        return false;
    }
    let (cx, cy) = ((x - ox) / s, (y - oy) / s);
    glyph[cy] & (0b10000 >> cx) != 0
}

fn make_icon(paused: bool, desk: u8) -> windows::core::Result<HICON> {
    // SAFETY: sin precondiciones.
    let n = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as usize;
    let light = crate::session::light_taskbar();
    let (r, g, b) = if light { (0x14, 0x14, 0x14) } else { (0xF2, 0xF2, 0xF2) };
    let mut px = vec![0u8; n * n * 4];
    let m = n / 8; // margen
    let t = (n / 16).max(1); // grosor del contorno
    let (lo, hi) = (m, n - m); // [lo, hi)
    for y in 0..n {
        for x in 0..n {
            if x < lo || x >= hi || y < lo || y >= hi {
                continue;
            }
            // Esquinas redondeadas: sin el píxel de cada esquina.
            let corner = (x == lo || x == hi - 1) && (y == lo || y == hi - 1);
            if corner {
                continue;
            }
            // Activo: cuadrado lleno con el número calado. En pausa: solo el
            // contorno, con el número dibujado.
            let edge = x < lo + t || x >= hi - t || y < lo + t || y >= hi - t;
            let digit = digit_pixel(desk, n, x, y);
            let on = if paused { edge || digit } else { !digit };
            if on {
                let i = (y * n + x) * 4;
                px[i..i + 4].copy_from_slice(&[b, g, r, 0xFF]);
            }
        }
    }
    let ni = n as i32;
    // SAFETY: buffers locales del tamaño indicado; los bitmaps se borran
    // después de crear el ícono (que hace su copia).
    unsafe {
        let color = CreateBitmap(ni, ni, 1, 32, Some(px.as_ptr().cast()));
        let mask_bits = vec![0u8; n.div_ceil(16) * 2 * n];
        let mask = CreateBitmap(ni, ni, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info =
            ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El número del escritorio entra en el ícono a 16 y a 24 px.
    #[test]
    fn digits_fit_in_the_icon() {
        for n in [16usize, 24, 32] {
            for desk in 1..=4u8 {
                let lit = (0..n * n).filter(|i| digit_pixel(desk, n, i % n, i / n)).count();
                assert!(lit >= 10, "dígito {desk} a {n} px: {lit} píxeles");
                // Ningún píxel del dígito en el margen.
                let m = n / 8 + (n / 16).max(1);
                for y in 0..n {
                    for x in 0..n {
                        if digit_pixel(desk, n, x, y) {
                            assert!(
                                x >= m && y >= m && x < n - m && y < n - m,
                                "{desk} {n}: ({x}, {y})"
                            );
                        }
                    }
                }
            }
        }
        // Para mirar: el 2 a 16 px.
        for y in 0..16 {
            let row: String =
                (0..16).map(|x| if digit_pixel(2, 16, x, y) { '#' } else { '.' }).collect();
            println!("{row}");
        }
    }

    #[test]
    fn desks_pack_and_unpack() {
        assert_eq!(unpack_desks(pack_desks(3, [5, 0, 2, 300])), (3, [5, 0, 2, 255]));
    }
}
