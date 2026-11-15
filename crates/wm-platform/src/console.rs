//! Manejo de Ctrl+C / cierre de la consola para el guardián.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::System::Console::{CTRL_C_EVENT, CTRL_BREAK_EVENT, SetConsoleCtrlHandler};
use windows::core::BOOL;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
static FINISHED: AtomicBool = AtomicBool::new(false);

/// Cuánto esperamos a terminar de restaurar si cierran la consola o se apaga
/// la PC. Windows mata el proceso a los ~5 s de todos modos.
const CLOSE_GRACE: Duration = Duration::from_millis(4000);

unsafe extern "system" fn handler(ctrl: u32) -> BOOL {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
    // Ctrl+C / Ctrl+Break: este proceso sigue; el hijo (misma consola) recibe
    // la señal y termina, y el guardián se entera al esperarlo.
    // Cerrar la consola, cerrar sesión o apagar: Windows termina el proceso
    // apenas volvemos de acá, así que esperamos a que el hilo principal
    // termine de restaurar las ventanas.
    if ctrl != CTRL_C_EVENT && ctrl != CTRL_BREAK_EVENT {
        let deadline = Instant::now() + CLOSE_GRACE;
        while !FINISHED.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    true.into()
}

/// A partir de ahora, Ctrl+C o cerrar la consola no matan a este proceso:
/// solo marcan que el usuario pidió parar.
pub fn trap_stop_requests() {
    // SAFETY: `handler` cumple la firma de PHANDLER_ROUTINE y vive para siempre.
    let _ = unsafe { SetConsoleCtrlHandler(Some(handler), true) };
}

pub fn stop_requested() -> bool {
    STOP_REQUESTED.load(Ordering::SeqCst)
}

/// El hilo principal ya dejó todo en orden: el manejador puede volver.
pub fn mark_finished() {
    FINISHED.store(true, Ordering::SeqCst);
}

unsafe extern "system" fn swallow(_ctrl: u32) -> BOOL {
    true.into()
}

/// Solo para pruebas: manda Ctrl+C (o Ctrl+Break) a la consola del proceso
/// `pid`, como si el usuario lo apretara en esa ventana.
pub fn send_ctrl_c(pid: u32, as_break: bool) -> windows::core::Result<()> {
    use windows::Win32::System::Console::{AttachConsole, FreeConsole, GenerateConsoleCtrlEvent};
    // SAFETY: llamadas de consola sin punteros; ignoramos Ctrl+C en este
    // proceso (handler None + TRUE) antes de generarlo para no morir nosotros.
    unsafe {
        let _ = FreeConsole();
        AttachConsole(pid)?;
        SetConsoleCtrlHandler(None, true)?;
        let event = if as_break { CTRL_BREAK_EVENT } else { CTRL_C_EVENT };
        // Ctrl+Break no se puede ignorar con `None`: un manejador propio que lo come.
        SetConsoleCtrlHandler(Some(swallow), true)?;
        let result = GenerateConsoleCtrlEvent(event, 0);
        std::thread::sleep(Duration::from_millis(500));
        let _ = FreeConsole();
        result
    }
}

/// Solo para pruebas: cierra la ventana de la consola del proceso `pid`, como
/// si el usuario apretara la X (Windows manda CTRL_CLOSE_EVENT).
pub fn close_console(pid: u32) -> windows::core::Result<()> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Console::{AttachConsole, FreeConsole, GetConsoleWindow};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    // SAFETY: llamadas de consola y un PostMessage sin punteros.
    unsafe {
        let _ = FreeConsole();
        AttachConsole(pid)?;
        let window = GetConsoleWindow();
        let _ = FreeConsole();
        PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0))
    }
}
