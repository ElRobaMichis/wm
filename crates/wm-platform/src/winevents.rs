//! WinEvents del brain (hito 4b.2, `plan-hito4.md` §4): qué ventana pasa al
//! frente y cuál pierde el cloak. Fuera de contexto y en el hilo del brain:
//! Windows los entrega mientras el brain lee su cola (`input::wait`), y la
//! espera despierta con ellos (verificado en `wm desk-check`). En reposo no
//! llega ninguno (0 en 2 s, medido).
//!
//! Sin `SKIPOWNPROCESS` se verían también el mapa y el fantasma: se saltean.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};

use crate::windows_list::WindowId;

/// Lo que avisó Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinEvent {
    /// Pasó al frente (el HWND tal cual: puede ser un diálogo).
    Foreground(WindowId),
    /// Le quitaron el cloak (el shell al activarla, un reinicio de Explorer).
    Uncloaked(WindowId),
}

thread_local! {
    static QUEUE: RefCell<Vec<WinEvent>> = const { RefCell::new(Vec::new()) };
}

static BRAIN: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    // Solo la ventana misma (no sus controles ni el cursor).
    if object != 0 || child != 0 || hwnd.is_invalid() {
        return;
    }
    let id = WindowId(hwnd.0 as isize);
    let e = if event == EVENT_SYSTEM_FOREGROUND {
        WinEvent::Foreground(id)
    } else {
        WinEvent::Uncloaked(id)
    };
    QUEUE.with(|q| q.borrow_mut().push(e));
    crate::ops::wake(BRAIN.load(Ordering::Relaxed));
}

/// Los hooks (se quitan al soltarlo). Crear y leer en el hilo del brain.
pub struct WinEvents {
    hooks: Vec<HWINEVENTHOOK>,
}

impl WinEvents {
    pub fn start(brain: u32) -> Self {
        BRAIN.store(brain, Ordering::Relaxed);
        let mut hooks = Vec::new();
        for event in [EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_UNCLOAKED] {
            // SAFETY: callback con la firma de WINEVENTPROC; fuera de contexto,
            // se despacha en la cola de este hilo.
            let h = unsafe {
                SetWinEventHook(
                    event,
                    event,
                    None,
                    Some(on_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if !h.is_invalid() {
                hooks.push(h);
            }
        }
        Self { hooks }
    }

    pub fn active(&self) -> bool {
        self.hooks.len() == 2
    }

    /// Lo que llegó desde la última vez, en orden.
    pub fn drain(&mut self) -> Vec<WinEvent> {
        QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
    }
}

impl Drop for WinEvents {
    fn drop(&mut self) {
        for h in &self.hooks {
            // SAFETY: hooks propios, en el mismo hilo.
            let _ = unsafe { UnhookWinEvent(*h) };
        }
    }
}
