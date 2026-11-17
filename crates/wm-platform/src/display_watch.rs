//! Avisos de cambios de pantallas, energía y sesión (architecture.md §4.3 y
//! §8, spike 7). Una ventana **top-level** oculta (las message-only no reciben
//! broadcasts) en su propio hilo; cada aviso se manda por un canal con la hora
//! QPC en que llegó.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
    TranslateMessage, WM_DISPLAYCHANGE, WM_POWERBROADCAST, WM_SETTINGCHANGE, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WM_DEVICECHANGE: u32 = 0x0219;
const SPI_SETWORKAREA: usize = 0x002F;
const PBT_APMSUSPEND: usize = 0x4;
const PBT_APMRESUMESUSPEND: usize = 0x7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
const WTS_CONSOLE_CONNECT: usize = 0x1;
const WTS_CONSOLE_DISCONNECT: usize = 0x2;
const WTS_REMOTE_CONNECT: usize = 0x3;
const WTS_REMOTE_DISCONNECT: usize = 0x4;
const DBT_DEVNODES_CHANGED: usize = 0x0007;

/// Un aviso de Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEvent {
    /// Cambió la resolución o la cantidad de monitores (bits por pixel, ancho, alto
    /// del monitor principal).
    DisplayChange {
        width: u16,
        height: u16,
    },
    /// Cambió el área de trabajo (barra de tareas).
    WorkArea,
    /// Otro cambio de configuración (incluye la escala, que llega como
    /// `WM_SETTINGCHANGE` con otro parámetro).
    Setting,
    Suspend,
    Resume,
    /// Cambio de estado de energía de un monitor (se apagó o prendió).
    PowerSetting,
    Lock,
    Unlock,
    ConsoleConnect,
    ConsoleDisconnect,
    /// Sesión remota (RDP) conectada o desconectada (hito 4d).
    RemoteConnect,
    RemoteDisconnect,
    /// Se conectó o desconectó algún dispositivo (por ejemplo un monitor).
    DevicesChanged,
}

impl DisplayEvent {
    pub fn name(self) -> &'static str {
        match self {
            DisplayEvent::DisplayChange { .. } => "pantallas",
            DisplayEvent::WorkArea => "área de trabajo",
            DisplayEvent::Setting => "configuración",
            DisplayEvent::Suspend => "reposo",
            DisplayEvent::Resume => "vuelta del reposo",
            DisplayEvent::PowerSetting => "energía de monitor",
            DisplayEvent::Lock => "bloqueo",
            DisplayEvent::Unlock => "desbloqueo",
            DisplayEvent::ConsoleConnect => "consola conectada",
            DisplayEvent::ConsoleDisconnect => "consola desconectada",
            DisplayEvent::RemoteConnect => "sesión remota conectada",
            DisplayEvent::RemoteDisconnect => "sesión remota desconectada",
            DisplayEvent::DevicesChanged => "dispositivos",
        }
    }
}

static SINK: Mutex<Option<Sender<(i64, DisplayEvent)>>> = Mutex::new(None);
/// Hilo del brain a despertar (0 = nadie: `wm display-watch` sondea).
static BRAIN: AtomicU32 = AtomicU32::new(0);

fn send(e: DisplayEvent) {
    let brain = BRAIN.load(Ordering::Acquire);
    // En el core, lo que no es de pantallas ni de sesión no despierta a nadie
    // (un pendrive, un auricular, un cambio de tema; plan-hito4d §1).
    if brain != 0
        && matches!(
            e,
            DisplayEvent::Setting | DisplayEvent::DevicesChanged | DisplayEvent::PowerSetting
        )
    {
        return;
    }
    if let Ok(guard) = SINK.lock()
        && let Some(tx) = guard.as_ref()
    {
        let _ = tx.send((crate::clock::now(), e));
    }
    if brain != 0 {
        crate::ops::wake(brain);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DISPLAYCHANGE => send(DisplayEvent::DisplayChange {
            width: (lparam.0 & 0xFFFF) as u16,
            height: ((lparam.0 >> 16) & 0xFFFF) as u16,
        }),
        WM_SETTINGCHANGE if wparam.0 == SPI_SETWORKAREA => send(DisplayEvent::WorkArea),
        WM_SETTINGCHANGE => send(DisplayEvent::Setting),
        WM_POWERBROADCAST => match wparam.0 {
            PBT_APMSUSPEND => send(DisplayEvent::Suspend),
            PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => send(DisplayEvent::Resume),
            PBT_POWERSETTINGCHANGE => send(DisplayEvent::PowerSetting),
            _ => {}
        },
        WM_WTSSESSION_CHANGE => match wparam.0 {
            WTS_SESSION_LOCK => send(DisplayEvent::Lock),
            WTS_SESSION_UNLOCK => send(DisplayEvent::Unlock),
            WTS_CONSOLE_CONNECT => send(DisplayEvent::ConsoleConnect),
            WTS_CONSOLE_DISCONNECT => send(DisplayEvent::ConsoleDisconnect),
            WTS_REMOTE_CONNECT => send(DisplayEvent::RemoteConnect),
            WTS_REMOTE_DISCONNECT => send(DisplayEvent::RemoteDisconnect),
            _ => {}
        },
        WM_DEVICECHANGE if wparam.0 == DBT_DEVNODES_CHANGED => send(DisplayEvent::DevicesChanged),
        _ => {}
    }
    // SAFETY: procesamiento por defecto.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Arranca el hilo con la ventana oculta y devuelve el canal de avisos.
pub fn start() -> windows::core::Result<Receiver<(i64, DisplayEvent)>> {
    let (tx, rx) = mpsc::channel();
    if let Ok(mut guard) = SINK.lock() {
        *guard = Some(tx);
    }
    let (ready_tx, ready_rx) = mpsc::channel();
    spawn(ready_tx)?;
    ready_rx.recv().map_err(|e| {
        windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
    })??;
    Ok(rx)
}

/// Hito 4d: los avisos para el core. No espera a que el hilo arranque (eso
/// costó > 5 ms en otros hilos); el resultado llega por [`Watch::ready`].
pub struct Watch {
    rx: Receiver<(i64, DisplayEvent)>,
    ready: Receiver<windows::core::Result<()>>,
}

impl Watch {
    /// Arranca el hilo; cada aviso despierta al hilo `brain`.
    pub fn start(brain: u32) -> Self {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut guard) = SINK.lock() {
            *guard = Some(tx);
        }
        BRAIN.store(brain, Ordering::Release);
        let (ready_tx, ready) = mpsc::channel();
        if let Err(e) = spawn(ready_tx.clone()) {
            let _ = ready_tx.send(Err(e));
        }
        Self { rx, ready }
    }

    /// `Some` una vez: si la ventana se pudo crear.
    pub fn ready(&mut self) -> Option<windows::core::Result<()>> {
        self.ready.try_recv().ok()
    }

    /// Los avisos pendientes, sin repetidos seguidos.
    pub fn drain(&mut self) -> Vec<DisplayEvent> {
        let mut out: Vec<DisplayEvent> = Vec::new();
        for (_, e) in self.rx.try_iter() {
            if out.last() != Some(&e) {
                out.push(e);
            }
        }
        out
    }
}

/// Para pruebas: postea `msg` (con `wparam`) a la ventana de avisos de la
/// instancia `instance` (`None` = la normal). Solo a nuestra ventana: nada
/// cambia en Windows.
pub fn poke(instance: Option<&str>, msg: u32, wparam: usize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
    let name = match instance {
        Some(i) => format!("Local\\wm.display.{i}"),
        None => "Local\\wm.display".to_owned(),
    };
    // SAFETY: nombres válidos; solo busca.
    let Ok(h) =
        (unsafe { FindWindowW(w!("wm-display-watch"), &windows::core::HSTRING::from(name)) })
    else {
        return false;
    };
    // SAFETY: PostMessageW no bloquea.
    unsafe { PostMessageW(Some(h), msg, WPARAM(wparam), LPARAM(0)) }.is_ok()
}

/// `WM_DISPLAYCHANGE` y `WM_WTSSESSION_CHANGE`, para [`poke`].
pub const MSG_DISPLAYCHANGE: u32 = WM_DISPLAYCHANGE;
pub const MSG_SESSION: u32 = WM_WTSSESSION_CHANGE;
pub const SESSION_LOCK: usize = WTS_SESSION_LOCK;
pub const SESSION_UNLOCK: usize = WTS_SESSION_UNLOCK;

fn spawn(ready_tx: Sender<windows::core::Result<()>>) -> windows::core::Result<()> {
    let title = windows::core::HSTRING::from(crate::instance::mutex_name("display"));
    std::thread::Builder::new()
        .name("wm-display-watch".into())
        .spawn(move || {
            // SAFETY: ventana propia de este hilo; el bucle la atiende siempre.
            let created = unsafe {
                (|| -> windows::core::Result<HWND> {
                    let instance = HINSTANCE(GetModuleHandleW(None)?.0);
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(proc),
                        hInstance: instance,
                        lpszClassName: w!("wm-display-watch"),
                        ..Default::default()
                    };
                    RegisterClassW(&class);
                    // Top-level (no message-only): las message-only no
                    // reciben broadcasts (§4.3). Nunca se muestra.
                    let h = CreateWindowExW(
                        WS_EX_TOOLWINDOW,
                        w!("wm-display-watch"),
                        &title,
                        WS_POPUP,
                        0,
                        0,
                        0,
                        0,
                        None,
                        None,
                        Some(instance),
                        None,
                    )?;
                    WTSRegisterSessionNotification(h, NOTIFY_FOR_THIS_SESSION)?;
                    Ok(h)
                })()
            };
            let ok = created.is_ok();
            let _ = ready_tx.send(created.map(|_| ()));
            if !ok {
                return;
            }
            let mut msg = MSG::default();
            // SAFETY: bucle de mensajes estándar con un MSG válido.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                // SAFETY: mensaje recién obtenido.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        })
        .map_err(|e| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
        })?;
    Ok(())
}
