//! Avisos de cambios de pantallas, energía y sesión (architecture.md §4.3 y
//! §8, spike 7). Una ventana **top-level** oculta (las message-only no reciben
//! broadcasts) en su propio hilo; cada aviso se manda por un canal con la hora
//! QPC en que llegó.

use std::sync::Mutex;
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
            DisplayEvent::DevicesChanged => "dispositivos",
        }
    }
}

static SINK: Mutex<Option<Sender<(i64, DisplayEvent)>>> = Mutex::new(None);

fn send(e: DisplayEvent) {
    if let Ok(guard) = SINK.lock()
        && let Some(tx) = guard.as_ref()
    {
        let _ = tx.send((crate::clock::now(), e));
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
                        w!("wm display watch"),
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
    ready_rx.recv().map_err(|e| {
        windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
    })??;
    Ok(rx)
}
