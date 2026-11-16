//! Carril `wm-drag` (hito 5, `plan-hito5.md` §5b-5c): mueve o cambia el
//! tamaño de la ventana agarrada con `Caps` + clic, siguiendo al cursor.
//!
//! El movimiento del mouse no pasa por el hook ni por el brain: este hilo
//! lee `GetCursorPos` una vez por cuadro del compositor mientras dura el
//! arrastre (solo entonces despierta) y duerme en `GetMessageW` el resto del
//! tiempo. **Contrapresión:** un solo pedido a la app por vez, hasta que la
//! ventana cambió, llegó o pasaron 100 ms; una app lenta (Electron) nunca
//! acumula pedidos en su cola y siempre recibe el último punto.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP,
};
use wm_model::Rect;
use wm_model::drag::{Bounds, Mode, Session};

use crate::windows_list::WindowId;

const WM_DRAG_CMD: u32 = WM_APP + 0x50;

/// Tope de un arrastre (el brain también tiene el suyo, 60 s).
const MAX_DRAG: Duration = Duration::from_secs(70);

/// Cuánto se espera a que la app aplique un pedido antes de mandar otro.
const PENDING_MAX: Duration = Duration::from_millis(100);

#[derive(Debug)]
enum Cmd {
    Start {
        win: WindowId,
        pid: u32,
        mode: Mode,
        grab: (i32, i32),
        restore: Option<(i32, i32)>,
        /// Número de arrastre: termina cuando `stopped` lo alcanza.
        serial: u64,
    },
}

/// Cómo terminó un arrastre.
#[derive(Debug, Clone, Copy)]
pub struct DragReport {
    /// Se movió de verdad (pasó el umbral de arrastre).
    pub moved: bool,
    /// Marco visible al terminar.
    pub frame: Option<Rect>,
    pub ms: f64,
}

#[derive(Debug)]
pub enum DragEvent {
    Done { win: WindowId, result: Result<DragReport, String> },
}

pub struct Drag {
    thread: u32,
    cmds: Sender<Cmd>,
    events: Receiver<DragEvent>,
    /// Último arrastre al que se le pidió terminar (por número): así un
    /// arrastre nuevo que llega mientras el anterior termina no se pisa.
    stopped: Arc<AtomicU64>,
    serial: u64,
    /// El arrastre en curso y el plazo del vigilante (después del Stop).
    current: Option<(WindowId, Option<Instant>)>,
}

impl Drag {
    pub fn start(brain: u32) -> Option<Self> {
        let (cmds_tx, cmds_rx) = mpsc::channel();
        let (events_tx, events_rx) = mpsc::channel();
        let stopped = Arc::new(AtomicU64::new(0));
        let flag = Arc::clone(&stopped);
        let handle = std::thread::Builder::new()
            .name("wm-drag".into())
            .spawn(move || run(brain, cmds_rx, events_tx, flag))
            .ok()?;
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Threading::GetThreadId;
        // SAFETY: handle válido del hilo recién creado.
        let thread = unsafe { GetThreadId(HANDLE(handle.as_raw_handle())) };
        drop(handle);
        Some(Self { thread, cmds: cmds_tx, events: events_rx, stopped, serial: 0, current: None })
    }

    fn send(&self, cmd: Cmd) {
        if self.cmds.send(cmd).is_ok() {
            // SAFETY: PostThreadMessageW no bloquea.
            let _ = unsafe { PostThreadMessageW(self.thread, WM_DRAG_CMD, WPARAM(0), LPARAM(0)) };
        }
    }

    /// Empieza a arrastrar `win` desde `grab`. `restore`: si está maximizada,
    /// su tamaño normal (se restaura bajo el cursor).
    pub fn begin(
        &mut self,
        win: WindowId,
        pid: u32,
        mode: Mode,
        grab: (i32, i32),
        restore: Option<(i32, i32)>,
    ) {
        self.serial += 1;
        self.current = Some((win, None));
        self.send(Cmd::Start { win, pid, mode, grab, restore, serial: self.serial });
    }

    /// Soltó el botón (o se reinició el teclado).
    pub fn end(&mut self) {
        self.stopped.store(self.serial, Ordering::Release);
        if let Some((_, deadline)) = &mut self.current {
            *deadline = Some(Instant::now() + Duration::from_millis(300));
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.current.and_then(|(_, d)| d)
    }

    pub fn drain(&mut self) -> Vec<DragEvent> {
        let out: Vec<DragEvent> = self.events.try_iter().collect();
        if !out.is_empty() {
            self.current = None;
        }
        out
    }

    /// El carril no respondió al Stop: se da por terminado (la ventana quedó
    /// donde quedó).
    pub fn expire(&mut self, now: Instant) -> Vec<DragEvent> {
        match self.current {
            Some((win, Some(d))) if d <= now => {
                self.current = None;
                vec![DragEvent::Done { win, result: Err("el arrastre no respondió".into()) }]
            }
            _ => Vec::new(),
        }
    }
}

fn run(brain: u32, cmds: Receiver<Cmd>, events: Sender<DragEvent>, stopped: Arc<AtomicU64>) {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
    };
    crate::input::ensure_message_queue();
    // SAFETY: cambia la prioridad del hilo actual.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
    let mut msg = MSG::default();
    loop {
        while let Ok(cmd) = cmds.try_recv() {
            let Cmd::Start { win, pid, mode, grab, restore, serial } = cmd;
            {
                let stop = || stopped.load(Ordering::Acquire) >= serial;
                let result = drag(win, pid, mode, grab, restore, &stop);
                if events.send(DragEvent::Done { win, result }).is_err() {
                    return;
                }
                crate::ops::wake(brain);
            }
        }
        // SAFETY: bucle de mensajes estándar.
        if !unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            return;
        }
        if msg.message != WM_DRAG_CMD {
            // SAFETY: mensaje recién obtenido.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    // SAFETY: puntero a una variable local.
    unsafe { GetCursorPos(&mut p) }.ok().map(|()| (p.x, p.y))
}

fn drag(
    win: WindowId,
    pid: u32,
    mode: Mode,
    grab: (i32, i32),
    restore: Option<(i32, i32)>,
    stop: &dyn Fn() -> bool,
) -> Result<DragReport, String> {
    let started = Instant::now();
    if crate::windows_list::window_pid(win) != pid {
        return Err("la ventana ya no existe".into());
    }
    // Si estaba en vuelo (el brain ya pidió aterrizarlo), esperar a que la
    // ventana real vuelva a verse: durante el vuelo su tamaño puede ser otro
    // (recorte del fantasma) y se aprendería un mínimo falso.
    let landing = Instant::now();
    while crate::ghost::is_cloaked(win) && landing.elapsed() < Duration::from_millis(300) {
        std::thread::sleep(Duration::from_millis(4));
    }
    let mut outer = crate::ghost::window_rect(win).ok_or("no se pudo leer la ventana")?;
    let mut maximized = restore;
    let mut session = Session::new(mode, grab, outer, Bounds::DEFAULT, drag_slop());
    // (pedido, rect de la ventana cuando se pidió, cuándo)
    let mut pending: Option<(Rect, Rect, Instant)> = None;
    let mut last_asked: Option<Rect> = None;
    crate::ghost::boost_clock(true);
    let finish = |session: &Session| {
        crate::ghost::boost_clock(false);
        Ok(DragReport {
            moved: session.moved(),
            frame: crate::ghost::frame_bounds(win),
            ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    };
    loop {
        let stopping = stop() || started.elapsed() > MAX_DRAG;
        let Some(p) = cursor() else {
            return finish(&session);
        };
        let now_rect = crate::ghost::window_rect(win);
        if now_rect.is_none() {
            crate::ghost::boost_clock(false);
            return Err("la ventana se cerró".into());
        }
        // ¿La app ya aplicó el pedido anterior?
        if let Some((asked, before, at)) = pending {
            let cur = now_rect.unwrap_or(before);
            if cur != before || cur == asked || at.elapsed() > PENDING_MAX {
                if cur != asked && matches!(mode, Mode::Resize { .. }) {
                    session.learn(asked, cur);
                }
                pending = None;
            }
        }
        if let Some(size) = maximized.filter(|_| session.moved() || moved_past_slop(grab, p)) {
            // Maximizada: restaurar bajo el cursor y seguir desde ahí.
            maximized = None;
            let max_rect = outer;
            let target = wm_model::drag::unmaximized(max_rect, size, grab);
            let _ = crate::windows_list::normalize(win);
            let _ = crate::windows_list::place_async(win, target);
            outer = target;
            session = Session::new(Mode::Move, grab, target, Bounds::DEFAULT, 0);
            pending = Some((target, max_rect, Instant::now()));
            last_asked = Some(target);
        } else if pending.is_none()
            && let Some(target) = session.at(p)
            && Some(target) != last_asked
        {
            let before = now_rect.unwrap_or(outer);
            let r = match mode {
                Mode::Move => move_to(win, target),
                Mode::Resize { .. } => crate::windows_list::place_async(win, target),
            };
            if let Err(e) = r {
                crate::ghost::boost_clock(false);
                return Err(e);
            }
            pending = Some((target, before, Instant::now()));
            last_asked = Some(target);
        }
        if stopping {
            return finish(&session);
        }
        crate::ghost::wait_frame();
    }
}

fn moved_past_slop(grab: (i32, i32), p: (i32, i32)) -> bool {
    let s = drag_slop();
    (p.0 - grab.0).abs() > s || (p.1 - grab.1).abs() > s
}

/// Umbral de arrastre del sistema (`SM_CXDRAG`, 4 px por defecto).
fn drag_slop() -> i32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDRAG};
    // SAFETY: sin precondiciones.
    unsafe { GetSystemMetrics(SM_CXDRAG) }.max(2)
}

/// Mover sin cambiar el tamaño: al cruzar a un monitor con otra escala, la
/// app se reescala sola (`WM_DPICHANGED`) y no se pelea por el tamaño.
fn move_to(win: WindowId, r: Rect) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    // SAFETY: asíncrono; no bloquea con apps colgadas.
    unsafe {
        SetWindowPos(
            win.hwnd(),
            None,
            r.left,
            r.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    }
    .map_err(|e| format!("SetWindowPos: {e}"))
}
