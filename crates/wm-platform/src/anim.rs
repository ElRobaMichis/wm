//! Carril `anim` (hito 1e, plan-core.md §3.4): anima ventanas ajenas con un
//! fantasma, con la secuencia validada en el spike 6 (architecture.md §13.8).
//!
//! Un hilo propio, creado una vez y dormido en `GetMessageW` (0 % de CPU)
//! hasta que llega un vuelo. Por vuelo, cuadro a cuadro del compositor:
//!
//! 1. **Mostrar:** fantasma (miniatura en vivo, marco visible 1:1) encima de
//!    la real, en su marco; 2 cuadros.
//! 2. **Cloak** de la real, confirmado con `DWMWA_CLOAKED` (tope 100 ms; si
//!    no, se cancela y la ventana va sin animación); 1 cuadro más.
//! 3. **Un solo** `SetWindowPos` asíncrono al destino, que además la sube al
//!    frente (el fantasma se ve encima de todo).
//! 4. **Vuelo:** posición por tiempo para el cuadro siguiente, con
//!    desaceleración ([`ease_out`]).
//! 5. **Llegada:** esperar el marco real en el destino (tope 150 ms; si no
//!    llega, el fantasma va adonde esté la real).
//! 6. **Uncloak** confirmado (tope 100 ms) y 2 cuadros.
//! 7. Quitar el fantasma.
//!
//! Entre cuadros atiende pedidos nuevos: un vuelo nuevo para la **misma**
//! ventana la redirige desde donde va el fantasma (otro `SetWindowPos`, sin
//! repetir el cloak); un `Land`, o un vuelo de **otra** ventana, la baja ya.
//!
//! Todos los topes son de tiempo, no de cuadros: con la sesión bloqueada, un
//! cuadro puede durar 100 ms. El brain vigila con [`Anim::next_deadline`]: si
//! el hilo se traba, lo abandona, esconde el anfitrión desde su propio hilo
//! y la ventana se vuelve a mostrar por el carril `shell` (la marca de
//! `Hider` la cubre también tras una caída).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
};
use wm_model::Rect;

use crate::ghost::{self, Ghost, GhostHost};
use crate::hide::{self, HideMethod, Hider};
use crate::windows_list::{self, WindowId};
use crate::{clock, input, ops};

/// Mensaje al hilo `anim`: hay pedidos en su canal.
const WM_ANIM_RUN: u32 = WM_APP + 0x21;

/// Tope para ver confirmado el cloak o el uncloak.
const CLOAK_LIMIT: Duration = Duration::from_millis(100);
/// Tope para que la real llegue al destino después del vuelo.
const ARRIVE_LIMIT: Duration = Duration::from_millis(150);
/// Margen del vigilante sobre la duración del vuelo (despegue, llegada y
/// uncloak con sus topes, más holgura).
const WATCHDOG_MARGIN: Duration = Duration::from_millis(800);

pub type Ticket = u64;

/// Un vuelo terminado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlyDone {
    /// La real llegó al destino antes de quitar el fantasma.
    pub arrived: bool,
    /// Duración del vuelo (desde el primer cuadro movido).
    pub flight_ms: u32,
    /// QPC del primer y el último cuadro del vuelo (0 si no voló).
    pub t0: i64,
    pub t1: i64,
    /// Espera desde el último cuadro del vuelo hasta ver la real en el
    /// destino (o el tope).
    pub land_ms: u32,
}

/// Qué hace el carril si el destino tiene otro tamaño que la ventana.
/// Producción usa `Same` (el modelo solo pide vuelos del mismo tamaño); los
/// otros son el mini-spike 6b.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Solo el mismo tamaño: otro tamaño se rechaza (sin animación).
    Same,
    /// Recortar la fuente 1:1, sin estirar (estira solo en un eje que tiene
    /// que crecer más allá de la fuente). Si todo crece, `SetWindowPos` al
    /// despegar (la fuente ya es grande); si todo se achica, al final.
    Crop,
    /// Estirar la fuente entera al rectángulo del cuadro.
    Stretch,
    /// Estirar, con `SetWindowPos` al final: durante el vuelo se estira el
    /// contenido viejo (estable) y la app cambia de tamaño recién al llegar.
    StretchLate,
}

/// Un vuelo que no se hizo (o no terminó bien).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlyError {
    /// No se anima más en esta sesión (y la ventana puede necesitar
    /// `Unhide`).
    pub fatal: bool,
    pub why: String,
}

#[derive(Debug)]
pub enum AnimEvent {
    /// El hilo está listo para animar (o no puede: sin cloak, sin anfitrión).
    Ready(Result<(), String>),
    Flown {
        ticket: Ticket,
        win: WindowId,
        result: Result<FlyDone, FlyError>,
    },
}

enum Cmd {
    Fly { ticket: Ticket, win: WindowId, rect: Rect, to: Rect, ms: u32 },
    Land(WindowId),
    LandAll,
}

struct Worker {
    thread_id: u32,
    cmds: Sender<Cmd>,
    abandoned: Arc<AtomicBool>,
}

/// El lado del brain: nunca bloquea.
pub struct Anim {
    brain: u32,
    worker: Option<Worker>,
    events_rx: Receiver<AnimEvent>,
    /// Fallos inmediatos (carril muerto), entregados en el próximo `drain`.
    local: Vec<AnimEvent>,
    /// Vuelos sin respuesta: ventana y plazo del vigilante.
    in_flight: HashMap<Ticket, (WindowId, Instant)>,
    next_ticket: Ticket,
    /// Por qué no se anima más (hilo trabado o sin cloak).
    dead: Option<String>,
}

impl Anim {
    /// Crea el hilo sin esperarlo; avisa con [`AnimEvent::Ready`].
    pub fn start(brain: u32) -> Self {
        // Cambiar de tamaño también vuela, recortando en dos pasos: 0 defectos
        // con títere, Brave, Rayo y Terminal (architecture.md §13.8b).
        Self::start_with(brain, Fit::Crop)
    }

    /// Como [`Anim::start`], con otra política de tamaño (mini-spike 6b).
    pub fn start_with(brain: u32, fit: Fit) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        let (cmds_tx, cmds_rx) = mpsc::channel();
        let abandoned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&abandoned);
        let spawned = std::thread::Builder::new()
            .name("wm-anim".into())
            .spawn(move || run(brain, fit, cmds_rx, events_tx, flag));
        let (worker, dead) = match spawned {
            Ok(handle) => {
                use std::os::windows::io::AsRawHandle;
                use windows::Win32::Foundation::HANDLE;
                use windows::Win32::System::Threading::GetThreadId;
                // SAFETY: handle válido del hilo recién creado.
                let thread_id = unsafe { GetThreadId(HANDLE(handle.as_raw_handle())) };
                drop(handle);
                (Some(Worker { thread_id, cmds: cmds_tx, abandoned }), None)
            }
            Err(e) => (None, Some(format!("no se pudo crear el hilo: {e}"))),
        };
        Self {
            brain,
            worker,
            events_rx,
            local: Vec::new(),
            in_flight: HashMap::new(),
            next_ticket: 1,
            dead,
        }
    }

    fn send(&self, cmd: Cmd) -> bool {
        let Some(w) = &self.worker else { return false };
        if w.cmds.send(cmd).is_err() {
            return false;
        }
        // SAFETY: PostThreadMessageW no bloquea.
        let _ = unsafe { PostThreadMessageW(w.thread_id, WM_ANIM_RUN, WPARAM(0), LPARAM(0)) };
        true
    }

    /// Pide un vuelo de `win` hasta `to` (marco visible); `rect` es el
    /// rectángulo exterior para `SetWindowPos`.
    pub fn fly(&mut self, win: WindowId, rect: Rect, to: Rect, ms: u32) -> Ticket {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        let deadline = Instant::now() + Duration::from_millis(u64::from(ms)) + WATCHDOG_MARGIN;
        if self.dead.is_none() && self.send(Cmd::Fly { ticket, win, rect, to, ms }) {
            self.in_flight.insert(ticket, (win, deadline));
        } else {
            let why = self.dead.clone().unwrap_or_else(|| "el carril no está".into());
            let result = Err(FlyError { fatal: true, why });
            self.local.push(AnimEvent::Flown { ticket, win, result });
            ops::wake(self.brain);
        }
        ticket
    }

    /// Bajar ya el vuelo de `win` (si está volando).
    pub fn land(&mut self, win: WindowId) {
        if self.in_flight.values().any(|(w, _)| *w == win) {
            self.send(Cmd::Land(win));
        }
    }

    /// Bajar todo (salida, reinicio del teclado).
    pub fn land_all(&mut self) {
        if !self.in_flight.is_empty() {
            self.send(Cmd::LandAll);
        }
    }

    /// Hay vuelos sin terminar.
    pub fn busy(&self) -> bool {
        !self.in_flight.is_empty()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.in_flight.values().map(|(_, d)| *d).min()
    }

    /// Respuestas disponibles. Llamar en cada vuelta del brain.
    pub fn drain(&mut self) -> Vec<AnimEvent> {
        let mut out = std::mem::take(&mut self.local);
        while let Ok(event) = self.events_rx.try_recv() {
            match &event {
                AnimEvent::Ready(Ok(())) => {}
                AnimEvent::Ready(Err(e)) => self.dead = Some(e.clone()),
                AnimEvent::Flown { ticket, .. } => {
                    if self.in_flight.remove(ticket).is_none() {
                        continue; // de un hilo abandonado
                    }
                }
            }
            out.push(event);
        }
        out
    }

    /// Vigilante: si un vuelo pasó su plazo, el hilo está trabado. Se lo
    /// abandona (nunca `TerminateThread`), se esconde el anfitrión desde este
    /// hilo y no se anima más en la sesión. Devuelve un `Flown` fatal por
    /// cada vuelo pendiente: el brain tiene que volver a mostrar esas
    /// ventanas por el carril `shell`.
    pub fn expire(&mut self, now: Instant) -> Vec<AnimEvent> {
        if !self.in_flight.values().any(|(_, d)| *d <= now) {
            return Vec::new();
        }
        if let Some(w) = self.worker.take() {
            w.abandoned.store(true, Ordering::Release);
            // SAFETY: PostThreadMessageW no bloquea.
            let _ = unsafe { PostThreadMessageW(w.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }
        let raw = HOST_RAW.load(Ordering::Acquire);
        if raw != 0 {
            ghost::cloak_host(raw);
        }
        let why = "el carril de animación se trabó".to_owned();
        self.dead = Some(why.clone());
        self.in_flight
            .drain()
            .map(|(ticket, (win, _))| AnimEvent::Flown {
                ticket,
                win,
                result: Err(FlyError { fatal: true, why: why.clone() }),
            })
            .collect()
    }
}

// ---- El hilo ----

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Fantasma visible; faltan `frames` cuadros para el cloak.
    Show {
        frames: u8,
    },
    /// Cloak pedido; esperando verlo confirmado (y un cuadro más).
    Cloak {
        since: Instant,
        confirmed: bool,
    },
    Flight {
        start: Instant,
        from: Rect,
    },
    Arrive {
        since: Instant,
    },
    /// Uncloak pedido; cuadros desde que se confirmó.
    Uncloak {
        since: Instant,
        frames: u8,
    },
}

struct Current {
    ticket: Ticket,
    win: WindowId,
    rect: Rect,
    to: Rect,
    ms: u32,
    ghost: Ghost,
    /// Dónde está el fantasma ahora.
    at: Rect,
    phase: Phase,
    /// Duración de un cuadro (para calcular la posición del siguiente).
    period: Duration,
    arrived: bool,
    flight_ms: u32,
    t0: i64,
    t1: i64,
    land_ms: u32,
    /// Cambia de tamaño y se recorta la fuente ([`Fit::Crop`]).
    crop: bool,
    /// `SetWindowPos` al despegar (`None`: ninguno) y al final del vuelo.
    first: Option<Rect>,
    late: bool,
    /// Bordes invisibles (izquierda, arriba) de la ventana al despegar.
    border: (i32, i32),
    /// Marco real cuando se hizo el último `SetWindowPos`, y el del cuadro
    /// anterior (para ver si la app ya lo atendió y quedó quieta).
    asked_at: Option<Rect>,
    last_real: Option<Rect>,
}

impl Current {
    /// Pone el fantasma en `r` (recortando la fuente si corresponde).
    fn show_at(&mut self, r: Rect) {
        self.at = r;
        if self.crop {
            let (sw, sh) = self.ghost.source_size().unwrap_or((r.width(), r.height()));
            let size = (r.width().min(sw), r.height().min(sh));
            let _ = self.ghost.place_crop(r, self.border, size);
        } else {
            let _ = self.ghost.place(r);
        }
    }
}

struct Lane {
    brain: u32,
    fit: Fit,
    events: Sender<AnimEvent>,
    abandoned: Arc<AtomicBool>,
    hider: Hider,
    host: GhostHost,
    current: Option<Current>,
}

fn run(
    brain: u32,
    fit: Fit,
    cmds: Receiver<Cmd>,
    events: Sender<AnimEvent>,
    abandoned: Arc<AtomicBool>,
) {
    input::ensure_message_queue();
    let ready = Hider::new(HideMethod::ShellCloak)
        .map_err(|e| format!("sin cloak: {e}"))
        .and_then(|h| GhostHost::new().map(|g| (h, g)).map_err(|e| format!("sin anfitrión: {e}")));
    let (hider, host) = match ready {
        Ok(x) => x,
        Err(e) => {
            let _ = events.send(AnimEvent::Ready(Err(e)));
            ops::wake(brain);
            return;
        }
    };
    HOST_RAW.store(host.raw(), Ordering::Release);
    let _ = events.send(AnimEvent::Ready(Ok(())));
    ops::wake(brain);
    let mut lane = Lane { brain, fit, events, abandoned, hider, host, current: None };
    let mut msg = MSG::default();
    // SAFETY: bucle de mensajes estándar con un MSG válido (STA: también
    // bombea lo que COM necesite).
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        if lane.abandoned.load(Ordering::Acquire) {
            break;
        }
        if msg.message == WM_ANIM_RUN {
            lane.commands(&cmds);
            lane.fly_until_idle(&cmds);
        } else {
            // SAFETY: mensaje recién obtenido de GetMessageW.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    // Abandonado: lo único que se intenta es devolver la ventana (el brain
    // también lo pide por el carril `shell`). Los objetos COM no se liberan
    // (sería otra llamada al Explorer, que puede estar colgado).
    if let Some(c) = lane.current.take() {
        let _ = lane.hider.show(c.win);
        std::mem::forget(c.ghost);
    }
    std::mem::forget(lane);
}

/// HWND del anfitrión del hilo `anim` (0: todavía no hay): si el hilo se
/// traba, el brain lo esconde desde su propio hilo.
static HOST_RAW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Desaceleración: ease-out cúbico mezclado con un 25 % lineal. El cúbico
/// puro llega con velocidad cero y en los últimos 3-4 cuadros avanza menos
/// de 1 px (el fantasma repite posición: 94,7 % de cuadros distintos en vez
/// de ≥ 95 %); con la mezcla llega a ~5 px por cuadro en 600 px.
pub fn ease_out(t: f64) -> f64 {
    0.75 * (1.0 - (1.0 - t).powi(3)) + 0.25 * t
}

fn lerp(a: Rect, b: Rect, t: f64) -> Rect {
    let l = |x: i32, y: i32| x + (f64::from(y - x) * t).round() as i32;
    Rect {
        left: l(a.left, b.left),
        top: l(a.top, b.top),
        right: l(a.right, b.right),
        bottom: l(a.bottom, b.bottom),
    }
}

fn same_size(a: Rect, b: Rect) -> bool {
    a.width() == b.width() && a.height() == b.height()
}

/// Duración de un cuadro en el monitor que contiene `r`.
fn frame_period(r: Rect) -> Duration {
    let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    let hz = crate::monitors()
        .into_iter()
        .find(|m| {
            let b = m.bounds;
            cx >= b.left && cx < b.right && cy >= b.top && cy < b.bottom
        })
        .and_then(|m| m.refresh_hz)
        .unwrap_or(60)
        .max(24);
    Duration::from_secs_f64(1.0 / f64::from(hz))
}

fn set_priority(critical: bool) {
    let p = if critical { THREAD_PRIORITY_TIME_CRITICAL } else { THREAD_PRIORITY_NORMAL };
    // SAFETY: cambia la prioridad del hilo actual.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), p) };
}

/// Un cuadro del vuelo: la posición para el cuadro siguiente, por tiempo.
fn flight_step(c: &mut Current, now: Instant, start: Instant, from: Rect) {
    let total = Duration::from_millis(u64::from(c.ms));
    let t = ((now - start) + c.period).as_secs_f64() / total.as_secs_f64();
    if t >= 1.0 {
        c.show_at(c.to);
        c.t1 = clock::now();
        c.flight_ms = (now - start).as_millis() as u32;
        if c.late {
            // Recorte que achica: recién ahora la real cambia de tamaño.
            c.asked_at = ghost::frame_bounds(c.win);
            let _ = windows_list::place_async_top(c.win, c.rect);
        }
        c.phase = Phase::Arrive { since: now };
    } else {
        c.show_at(lerp(from, c.to, ease_out(t)));
    }
}

impl Lane {
    fn reply(&self, ticket: Ticket, win: WindowId, result: Result<FlyDone, FlyError>) {
        let _ = self.events.send(AnimEvent::Flown { ticket, win, result });
        ops::wake(self.brain);
    }

    fn refuse(&self, ticket: Ticket, win: WindowId, why: impl Into<String>) {
        self.reply(ticket, win, Err(FlyError { fatal: false, why: why.into() }));
    }

    fn commands(&mut self, cmds: &Receiver<Cmd>) {
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                Cmd::Fly { ticket, win, rect, to, ms } => self.fly(ticket, win, rect, to, ms),
                Cmd::Land(win) => {
                    if self.current.as_ref().is_some_and(|c| c.win == win) {
                        self.land_now();
                    }
                }
                Cmd::LandAll => self.land_now(),
            }
        }
    }

    /// Avanza el vuelo en curso cuadro a cuadro, atendiendo pedidos.
    fn fly_until_idle(&mut self, cmds: &Receiver<Cmd>) {
        while self.current.is_some() {
            if self.abandoned.load(Ordering::Acquire) {
                return;
            }
            self.step();
            if self.current.is_none() {
                break;
            }
            ghost::wait_frame();
            self.commands(cmds);
        }
        set_priority(false);
        ghost::boost_clock(false);
    }

    fn fly(&mut self, ticket: Ticket, win: WindowId, rect: Rect, to: Rect, ms: u32) {
        // Misma ventana con cambio de tamaño (en el vuelo en curso o en el
        // nuevo): la fuente ya no alcanza para redirigir; se baja y se vuela
        // de nuevo desde donde quedó la real.
        if self.current.as_ref().is_some_and(|c| c.win == win && (c.crop || !same_size(c.to, to))) {
            self.land_now();
        }
        // Misma ventana: redirigir desde donde va.
        if let Some(c) = self.current.as_mut().filter(|c| c.win == win) {
            let old = c.ticket;
            match c.phase {
                Phase::Show { .. } | Phase::Cloak { .. } => {
                    // Todavía no despegó: el pedido al despegar es el nuevo
                    // (acá solo llegan vuelos del mismo tamaño).
                    (c.ticket, c.rect, c.to, c.ms) = (ticket, rect, to, ms);
                    c.first = Some(rect);
                }
                Phase::Flight { .. } | Phase::Arrive { .. } => {
                    c.asked_at = ghost::frame_bounds(win);
                    c.last_real = None;
                    if windows_list::place_async_top(win, rect).is_err() {
                        // No se pudo pedir: bajarla donde esté y que el
                        // modelo la coloque sin animación.
                        self.land_now();
                        self.refuse(ticket, win, "no se pudo mover");
                        return;
                    }
                    (c.ticket, c.rect, c.to, c.ms) = (ticket, rect, to, ms);
                    c.phase = Phase::Flight { start: Instant::now(), from: c.at };
                }
                Phase::Uncloak { .. } => {
                    // Ya está volviendo a mostrarse: terminar y empezar otro.
                    self.land_now();
                    return self.fly(ticket, win, rect, to, ms);
                }
            }
            let done = FlyDone { arrived: false, flight_ms: 0, t0: 0, t1: 0, land_ms: 0 };
            self.reply(old, win, Ok(done));
            return;
        }
        // Otra ventana en vuelo: bajarla ya.
        if self.current.is_some() {
            self.land_now();
        }
        let Some(from) = ghost::frame_bounds(win) else {
            return self.refuse(ticket, win, "la ventana ya no existe");
        };
        let resize = !same_size(from, to);
        if resize && self.fit == Fit::Same {
            return self.refuse(ticket, win, "cambió de tamaño");
        }
        let crop = resize && self.fit == Fit::Crop;
        // Recorte: la fuente tiene que cubrir al fantasma en todo el vuelo.
        // Al despegar, la real (escondida) pasa a un tamaño que cubre los dos
        // rectángulos; al llegar, al final. Si todo crece, eso es el final
        // de una; si todo se achica, no hace falta el primer pedido.
        let (uw, uh) = (from.width().max(to.width()), from.height().max(to.height()));
        let union = Rect::new(
            rect.left,
            rect.top,
            rect.right + uw - to.width(),
            rect.bottom + uh - to.height(),
        );
        let (first, late) = match self.fit {
            Fit::Crop if resize => {
                let grows = (uw, uh) == (to.width(), to.height());
                let shrinks = (uw, uh) == (from.width(), from.height());
                match (grows, shrinks) {
                    (true, _) => (Some(rect), false),
                    (_, true) => (None, true),
                    _ => (Some(union), true),
                }
            }
            Fit::StretchLate if resize => (None, true),
            _ => (Some(rect), false),
        };
        // La fuente de la miniatura empieza en el rectángulo de la ventana,
        // con los bordes invisibles: el recorte se corre por ellos.
        let border =
            ghost::window_rect(win).map_or((0, 0), |w| (from.left - w.left, from.top - w.top));
        if hide::hidden_with(win).is_some() || ghost::is_cloaked(win) {
            return self.refuse(ticket, win, "la ventana está escondida");
        }
        if !self.host.is_current() {
            match GhostHost::new() {
                Ok(h) => {
                    self.host = h;
                    HOST_RAW.store(self.host.raw(), Ordering::Release);
                }
                Err(e) => return self.refuse(ticket, win, format!("sin anfitrión: {e}")),
            }
        }
        let mut g = match self.host.ghost(win) {
            Ok(g) => g,
            Err(e) => return self.refuse(ticket, win, format!("sin miniatura: {e}")),
        };
        if let Err(e) = g.place(from) {
            return self.refuse(ticket, win, format!("miniatura: {e}"));
        }
        self.host.show_below(crate::ui::shown_window());
        self.current = Some(Current {
            ticket,
            win,
            rect,
            to,
            ms,
            ghost: g,
            at: from,
            phase: Phase::Show { frames: 2 },
            period: frame_period(to),
            arrived: false,
            flight_ms: 0,
            t0: 0,
            t1: 0,
            land_ms: 0,
            crop,
            first,
            late,
            border,
            asked_at: None,
            last_real: None,
        });
    }

    /// Un paso de la secuencia (se llama una vez por cuadro).
    fn step(&mut self) {
        let Some(c) = self.current.as_mut() else { return };
        let now = Instant::now();
        match c.phase {
            Phase::Show { frames } if frames > 1 => c.phase = Phase::Show { frames: frames - 1 },
            Phase::Show { .. } => match self.hider.hide(c.win) {
                Ok(()) => c.phase = Phase::Cloak { since: now, confirmed: false },
                Err(e) => self.abort(format!("cloak: {e}")),
            },
            Phase::Cloak { since, confirmed } => {
                if confirmed {
                    // Un cuadro con la real escondida: ahora sí, el pedido
                    // (salvo un recorte que achica: va al final).
                    c.asked_at = ghost::frame_bounds(c.win);
                    if let Some(r) = c.first
                        && let Err(e) = windows_list::place_async_top(c.win, r)
                    {
                        return self.abort(e);
                    }
                    set_priority(true);
                    ghost::boost_clock(true);
                    c.t0 = clock::now();
                    c.phase = Phase::Flight { start: now, from: c.at };
                    flight_step(c, now, now, c.at);
                } else if ghost::is_cloaked(c.win) {
                    c.phase = Phase::Cloak { since, confirmed: true };
                } else if now - since > CLOAK_LIMIT {
                    self.abort("cloak no confirmado".into());
                }
            }
            Phase::Flight { start, from } => flight_step(c, now, start, from),
            Phase::Arrive { since } => {
                let real = ghost::frame_bounds(c.win);
                c.arrived = real == Some(c.to);
                // La app ya atendió el último pedido (el marco cambió desde
                // entonces) y quedó quieta en otro lugar: tamaño mínimo o
                // fijo. No hay nada más que esperar (una app lenta todavía
                // tiene el marco de antes del pedido y sí se espera).
                let settled_elsewhere = real.is_some() && real != c.asked_at && real == c.last_real;
                c.last_real = real;
                if c.arrived || settled_elsewhere || now - since > ARRIVE_LIMIT {
                    c.land_ms = (now - since).as_millis() as u32;
                    if !c.arrived
                        && let Some(r) = real
                    {
                        c.show_at(r);
                    }
                    set_priority(false);
                    ghost::boost_clock(false);
                    let _ = self.hider.show(c.win);
                    c.phase = Phase::Uncloak { since: now, frames: 0 };
                }
            }
            Phase::Uncloak { since, frames } => {
                if !ghost::is_cloaked(c.win) {
                    if frames >= 1 {
                        self.finish(None);
                    } else {
                        c.phase = Phase::Uncloak { since, frames: frames + 1 };
                    }
                } else if now - since > CLOAK_LIMIT {
                    // La real sigue escondida: que el brain la muestre por el
                    // carril `shell` y no animar más.
                    self.finish(Some("uncloak no confirmado".into()));
                }
            }
        }
    }

    /// Quitar el fantasma y responder.
    fn finish(&mut self, fatal: Option<String>) {
        let Some(mut c) = self.current.take() else { return };
        c.ghost.conceal();
        drop(c.ghost);
        self.host.hide();
        let result = match fatal {
            None => Ok(FlyDone {
                arrived: c.arrived,
                flight_ms: c.flight_ms,
                t0: c.t0,
                t1: c.t1,
                land_ms: c.land_ms,
            }),
            Some(why) => Err(FlyError { fatal: true, why }),
        };
        self.reply(c.ticket, c.win, result);
    }

    /// No se pudo despegar: devolver la ventana (si llegó a esconderse) y
    /// que el modelo la coloque sin animación.
    fn abort(&mut self, why: String) {
        let Some(c) = self.current.as_ref() else { return };
        let (ticket, win) = (c.ticket, c.win);
        if hide::hidden_with(win).is_some() {
            let _ = self.hider.show(win);
            if !self.wait_visible(win) {
                return self.finish(Some(format!("{why}; uncloak no confirmado")));
            }
        }
        let Some(mut c) = self.current.take() else { return };
        c.ghost.conceal();
        drop(c.ghost);
        self.host.hide();
        self.refuse(ticket, win, why);
    }

    /// Bajar ya el vuelo en curso: la real ya fue pedida al destino (o
    /// todavía no se movió); se muestra, confirmado, y se quita el fantasma.
    fn land_now(&mut self) {
        let Some(c) = self.current.as_mut() else { return };
        match c.phase {
            Phase::Show { .. } => {}
            Phase::Cloak { .. } => {
                let _ = self.hider.show(c.win);
            }
            Phase::Flight { .. } | Phase::Arrive { .. } => {
                // Recorte que pedía el tamaño final al llegar: pedirlo ya.
                if c.late && matches!(c.phase, Phase::Flight { .. }) {
                    let _ = windows_list::place_async_top(c.win, c.rect);
                }
                if let Some(r) = ghost::frame_bounds(c.win) {
                    c.show_at(r);
                }
                c.arrived = ghost::frame_bounds(c.win) == Some(c.to);
                let _ = self.hider.show(c.win);
            }
            Phase::Uncloak { .. } => {}
        }
        let win = c.win;
        let visible = hide::hidden_with(win).is_none() && self.wait_visible(win);
        set_priority(false);
        ghost::boost_clock(false);
        self.finish((!visible).then(|| "uncloak no confirmado".to_owned()));
    }

    /// Espera (con tope) a que DWM deje de esconder la ventana.
    fn wait_visible(&self, win: WindowId) -> bool {
        let since = Instant::now();
        while ghost::is_cloaked(win) {
            if since.elapsed() > CLOAK_LIMIT {
                return false;
            }
            ghost::wait_frame();
        }
        ghost::wait_frame();
        true
    }
}
