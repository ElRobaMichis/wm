//! Hilo `ui` (plan-core.md §2 y §5, hito 2a): dibuja el mapa de teclado con
//! los overlays de Composition ([`crate::overlay`], [`crate::overlay_text`]).
//!
//! - Se crea al arrancar el core, sin esperarlo; avisa con [`UiEvent::Ready`]
//!   y después duerme en `GetMessageW` (0 % de CPU).
//! - El brain manda pedidos por canal + `WM_UI_RUN`; las respuestas vuelven
//!   por canal + `WM_OPS`, como los carriles de `ops` y `anim`.
//! - El contenido (Geist, formas) se arma la primera vez que se muestra en
//!   cada monitor y queda en caché mientras el [`MapView`] sea el mismo.
//! - **Nunca pegado:** el brain vigila cada pedido (500 ms). Si el hilo se
//!   traba, lo abandona y esconde las ventanas del mapa desde su propio hilo
//!   (cloak de DWM). Y si el brain se traba con el mapa visible, el propio
//!   hilo `ui` lo oculta a los 32 s (temporizador, solo mientras se ve).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::UI::Composition::ContainerVisual;
use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, GetMessageW, KillTimer, MSG, PostThreadMessageW, SetTimer,
    TranslateMessage, WM_APP, WM_QUIT, WM_TIMER,
};
use wm_model::Rect;
use wm_model::keyboard::Layer;
use wm_model::mapview::MapView;

use crate::overlay::Overlays;
use crate::{ghost, input, ops, overlay_text, windows_list};

const WM_UI_RUN: u32 = WM_APP + 0x22;

/// Plazo de cada pedido: si no vuelve, el hilo está trabado.
const DEADLINE: Duration = Duration::from_millis(500);

/// El hilo se oculta solo si el mapa lleva este tiempo visible (el brain
/// lo oculta a los 30 s; esto cubre al brain trabado).
const SELF_HIDE_MS: u32 = 32_000;

/// Las dos capas del mapa: apps y ventana.
pub type Views = Arc<[MapView; 2]>;

#[derive(Debug)]
pub enum UiEvent {
    Ready(Result<(), String>),
    /// El mapa se ve, en el monitor `monitor`, `ms` después del pedido.
    Shown {
        ms: f64,
        monitor: usize,
    },
    Failed(String),
}

enum Cmd {
    Show { layer: Layer, views: Views, at: Instant },
    Layer(Layer),
    Hide,
}

/// Las ventanas del mapa, para esconderlas si el hilo se traba.
static WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

struct Worker {
    thread_id: u32,
    cmds: Sender<Cmd>,
    abandoned: Arc<AtomicBool>,
}

/// El lado del brain: nunca bloquea.
pub struct Ui {
    worker: Option<Worker>,
    events: Receiver<UiEvent>,
    /// Pedidos sin respuesta (plazo del más viejo).
    waiting: Option<Instant>,
    outstanding: u32,
    dead: bool,
}

impl Ui {
    pub fn start(brain: u32) -> Self {
        let (events_tx, events) = mpsc::channel();
        let (cmds_tx, cmds_rx) = mpsc::channel();
        let abandoned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&abandoned);
        let spawned = std::thread::Builder::new()
            .name("wm-ui".into())
            .spawn(move || run(brain, cmds_rx, events_tx, flag));
        let worker = spawned.ok().map(|handle| {
            use std::os::windows::io::AsRawHandle;
            use windows::Win32::Foundation::HANDLE;
            use windows::Win32::System::Threading::GetThreadId;
            // SAFETY: handle válido del hilo recién creado.
            let thread_id = unsafe { GetThreadId(HANDLE(handle.as_raw_handle())) };
            drop(handle);
            Worker { thread_id, cmds: cmds_tx, abandoned }
        });
        let dead = worker.is_none();
        Self { worker, events, waiting: None, outstanding: 0, dead }
    }

    fn send(&mut self, cmd: Cmd) {
        if self.dead {
            return;
        }
        let Some(w) = &self.worker else { return };
        if w.cmds.send(cmd).is_ok() {
            // SAFETY: PostThreadMessageW no bloquea.
            let _ = unsafe { PostThreadMessageW(w.thread_id, WM_UI_RUN, WPARAM(0), LPARAM(0)) };
            self.outstanding += 1;
            self.waiting.get_or_insert(Instant::now() + DEADLINE);
        }
    }

    pub fn show(&mut self, layer: Layer, views: Views) {
        self.send(Cmd::Show { layer, views, at: Instant::now() });
    }

    pub fn set_layer(&mut self, layer: Layer) {
        self.send(Cmd::Layer(layer));
    }

    pub fn hide(&mut self) {
        self.send(Cmd::Hide);
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.waiting
    }

    pub fn drain(&mut self) -> Vec<UiEvent> {
        let mut out = Vec::new();
        while let Ok(e) = self.events.try_recv() {
            if !matches!(e, UiEvent::Ready(_)) {
                self.outstanding = self.outstanding.saturating_sub(1);
                self.waiting = (self.outstanding > 0).then(|| Instant::now() + DEADLINE);
            }
            if let UiEvent::Ready(Err(_)) = &e {
                self.dead = true;
            }
            out.push(e);
        }
        out
    }

    /// Vigilante: si un pedido pasó su plazo, el hilo está trabado. Se lo
    /// abandona y se esconden sus ventanas desde este hilo (cloak de DWM, no
    /// envía mensajes a la ventana). Devuelve `true` si lo abandonó.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.dead || !self.waiting.is_some_and(|d| d <= now) {
            return false;
        }
        if let Some(w) = self.worker.take() {
            w.abandoned.store(true, Ordering::Release);
            // SAFETY: PostThreadMessageW no bloquea.
            let _ = unsafe { PostThreadMessageW(w.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }
        for &raw in WINDOWS.lock().map(|w| w.clone()).unwrap_or_default().iter() {
            ghost::cloak_host(raw);
        }
        self.dead = true;
        self.waiting = None;
        true
    }
}

// ---- El hilo ----

struct Built {
    views: Views,
    /// Por monitor: (apps, ventana), armados la primera vez que se muestran ahí.
    screens: Vec<Option<(ContainerVisual, ContainerVisual)>>,
}

struct State {
    overlays: Overlays,
    built: Option<Built>,
    shown: Option<usize>,
    timer: usize,
}

fn run(brain: u32, cmds: Receiver<Cmd>, events: Sender<UiEvent>, abandoned: Arc<AtomicBool>) {
    input::ensure_message_queue();
    let reply = |e: UiEvent| {
        let _ = events.send(e);
        ops::wake(brain);
    };
    let overlays = match Overlays::new() {
        Ok(o) => o,
        Err(e) => return reply(UiEvent::Ready(Err(format!("sin overlays: {e}")))),
    };
    if let Ok(mut w) = WINDOWS.lock() {
        *w = overlays.screens().iter().map(|s| s.hwnd.0 as isize).collect();
    }
    reply(UiEvent::Ready(Ok(())));
    let mut st = State { overlays, built: None, shown: None, timer: 0 };
    let mut msg = MSG::default();
    // SAFETY: bucle de mensajes estándar (Composition usa esta cola).
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        if abandoned.load(Ordering::Acquire) {
            break;
        }
        if msg.hwnd.is_invalid() && msg.message == WM_UI_RUN {
            while let Ok(cmd) = cmds.try_recv() {
                reply(st.run(cmd));
            }
        } else if msg.hwnd.is_invalid() && msg.message == WM_TIMER {
            // El brain no ocultó el mapa a tiempo: se oculta solo.
            st.hide();
        } else {
            // SAFETY: mensaje recién obtenido de GetMessageW.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    // Abandonado: las ventanas ya las escondió el brain (cloak). Liberar
    // Composition sería trabajar en un hilo que puede estar trabado.
    std::mem::forget(st);
}

impl State {
    fn run(&mut self, cmd: Cmd) -> UiEvent {
        match cmd {
            Cmd::Show { layer, views, at } => match self.show(layer, views) {
                Ok(monitor) => UiEvent::Shown { ms: at.elapsed().as_secs_f64() * 1000.0, monitor },
                Err(e) => {
                    self.hide();
                    UiEvent::Failed(format!("no se pudo mostrar el mapa: {e}"))
                }
            },
            Cmd::Layer(layer) => {
                if let Some(i) = self.shown {
                    let _ = self.set_layer(i, layer);
                }
                UiEvent::Shown { ms: 0.0, monitor: self.shown.unwrap_or(0) }
            }
            Cmd::Hide => {
                self.hide();
                UiEvent::Shown { ms: 0.0, monitor: usize::MAX }
            }
        }
    }

    fn show(&mut self, layer: Layer, views: Views) -> windows::core::Result<usize> {
        let n = self.overlays.screens().len();
        if self.built.as_ref().is_none_or(|b| b.views != views) {
            // Contenido nuevo (cambiaron las asignaciones): se rearma.
            for s in self.overlays.screens() {
                s.root.Children()?.RemoveAll()?;
            }
            self.built = Some(Built { views, screens: vec![None; n] });
        }
        let i = pick_monitor(&self.overlays);
        let built = self.built.as_mut().expect("recién armado");
        if built.screens[i].is_none() {
            let s = &self.overlays.screens()[i];
            let (w, h) = (s.bounds.width() as f32, s.bounds.height() as f32);
            let c = self.overlays.compositor();
            let apps = overlay_text::build_view(c, &s.root, w, h, &built.views[0])?;
            let window = overlay_text::build_view(c, &s.root, w, h, &built.views[1])?;
            built.screens[i] = Some((apps, window));
        }
        self.set_layer(i, layer)?;
        self.overlays.show_only(i);
        self.shown = Some(i);
        if self.timer == 0 {
            // SAFETY: temporizador de hilo (sin ventana); WM_TIMER llega a la cola.
            self.timer = unsafe { SetTimer(None, 0, SELF_HIDE_MS, None) };
        }
        Ok(i)
    }

    fn set_layer(&self, i: usize, layer: Layer) -> windows::core::Result<()> {
        let Some(Some((apps, window))) = self.built.as_ref().map(|b| &b.screens[i]) else {
            return Ok(());
        };
        apps.SetIsVisible(layer != Layer::Window)?;
        window.SetIsVisible(layer == Layer::Window)
    }

    fn hide(&mut self) {
        self.overlays.hide();
        self.shown = None;
        if self.timer != 0 {
            // SAFETY: temporizador propio de este hilo.
            let _ = unsafe { KillTimer(None, self.timer) };
            self.timer = 0;
        }
    }
}

/// El monitor de la ventana al frente; si no hay, el del puntero; si no, el
/// principal. Solo lecturas que no esperan a otras apps.
fn pick_monitor(overlays: &Overlays) -> usize {
    let screens = overlays.screens();
    let inside = |x: i32, y: i32| {
        screens.iter().position(|s| {
            let b: Rect = s.bounds;
            x >= b.left && x < b.right && y >= b.top && y < b.bottom
        })
    };
    let from_focus = windows_list::foreground()
        .and_then(ghost::frame_bounds)
        .filter(|f| f.width() > 0 && f.height() > 0)
        .and_then(|f| inside((f.left + f.right) / 2, (f.top + f.bottom) / 2));
    let from_cursor = || {
        let mut p = POINT::default();
        // SAFETY: variable local válida.
        unsafe { GetCursorPos(&mut p) }.ok().and_then(|()| inside(p.x, p.y))
    };
    from_focus.or_else(from_cursor).or_else(|| screens.iter().position(|s| s.primary)).unwrap_or(0)
}

/// ¿Se ve una ventana del mapa del proceso `pid` (visible y sin cloak)?
/// Para `ci-probe`.
pub fn map_visible_in(pid: u32) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, IsWindowVisible};
    use windows::core::BOOL;

    let mut found: Vec<HWND> = Vec::new();
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` es el `&mut Vec` que pasamos abajo.
        let found = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        let mut name = [0u16; 32];
        // SAFETY: buffer local válido.
        let len = unsafe { GetClassNameW(hwnd, &mut name) };
        if String::from_utf16_lossy(&name[..len.max(0) as usize]) == "wm-overlay" {
            found.push(hwnd);
        }
        true.into()
    }
    // SAFETY: el callback solo usa `data` como el `&mut Vec` que le pasamos.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut found as *mut Vec<HWND> as isize)) };
    found.into_iter().any(|h| {
        let id = windows_list::WindowId(h.0 as isize);
        windows_list::window_pid(id) == pid
            // SAFETY: lectura sin efectos.
            && unsafe { IsWindowVisible(h) }.as_bool()
            && !ghost::is_cloaked(id)
    })
}
