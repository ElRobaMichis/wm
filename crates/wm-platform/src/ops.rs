//! Hilos "ops" (architecture.md §4.2 y §13.6): toda llamada que toca ventanas
//! ajenas o a Explorer sale del brain y corre en uno de tres carriles fijos,
//! separados por de qué dependen:
//!
//! | Carril  | Qué corre                                        | Depende de        |
//! |---------|--------------------------------------------------|-------------------|
//! | `focus` | traer al frente (`SetForegroundWindow`)          | hooks LL, foco    |
//! | `win`   | mover, mostrar/esconder sin cloak, enumerar      | nada que bloquee  |
//! | `shell` | cloak / uncloak, `restore_all` (único STA)       | Explorer          |
//!
//! Reglas:
//! - El brain es el dueño de [`Ops`]: `submit` nunca bloquea y los resultados
//!   llegan con `WM_OPS` (ver [`crate::input::wait`]).
//! - Cada carril tiene **un solo pedido en vuelo**; la cola vive en el brain.
//! - Cada pedido tiene plazo. El brain espera con timeout = [`Ops::next_deadline`]
//!   (sin timers: 0 % de CPU en reposo). Si vence, el hilo se **abandona**
//!   (nunca `TerminateThread`) y se crea otro; el viejo, si algún día vuelve,
//!   avisa con [`OpsEvent::Late`] y termina.
//! - Tope de hilos trabados vivos: [`MAX_STUCK`]. Con el carril `shell`
//!   trabado (Explorer colgado), esconder con cloak pasa a `minimize`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
};
use wm_model::Rect;

use crate::clock;
use crate::hide::{self, HideMethod, Hider, RestoreReport};
use crate::input::{self, WM_OPS};
use crate::windows_list::{self, Activation, WindowId, WindowInfo};

/// Máximo de hilos trabados (abandonados y todavía sin volver) a la vez.
pub const MAX_STUCK: usize = 4;

/// Mensaje a un hilo ops: hay un pedido en su canal.
const WM_OP_RUN: u32 = WM_APP + 0x20;

/// Evita inundar la cola del brain: un solo `WM_OPS` pendiente a la vez.
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lane {
    Focus,
    Win,
    Shell,
}

impl Lane {
    pub const ALL: [Lane; 3] = [Lane::Focus, Lane::Win, Lane::Shell];

    pub fn name(self) -> &'static str {
        match self {
            Lane::Focus => "focus",
            Lane::Win => "win",
            Lane::Shell => "shell",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Un pedido. `Stall` es solo para pruebas.
#[derive(Debug, Clone)]
pub enum Op {
    /// Traer al frente. El `u32` es el proceso esperado (0 = cualquiera): si
    /// el HWND ahora es de otro proceso, falla sin tocarlo.
    Focus(WindowId, u32),
    /// Mover y redimensionar (asíncrono).
    Place { id: WindowId, rect: Rect },
    /// Observar una ventana (`None` = la que tiene el foco).
    Inspect(Option<(WindowId, u32)>),
    /// Sacar de maximizada o minimizada sin activar (asíncrono).
    Normalize(WindowId),
    /// Maximizar (asíncrono).
    Maximize(WindowId),
    /// Esconder con un método.
    Hide { id: WindowId, method: HideMethod },
    /// Mostrar una ventana escondida por nosotros (con el método de su marca).
    Unhide(WindowId),
    /// Enumerar las ventanas de usuario.
    Scan,
    /// Mostrar todo lo marcado.
    RestoreAll,
    /// Abrir un programa (crear un proceso puede tardar: antivirus, disco).
    Launch(String),
    /// Prueba: una llamada que tarda `ms` en el carril indicado.
    Stall { lane: Lane, ms: u32 },
}

impl Op {
    /// Plazo de la llamada: si no vuelve antes, el carril se da por trabado.
    fn deadline(&self) -> Duration {
        match self {
            // `activate` puede esperar hasta 2 × 50 ms a que cambie el foco.
            Op::Focus(..) => Duration::from_millis(300),
            Op::Place { .. } | Op::Scan | Op::Inspect(_) | Op::Normalize(_) | Op::Maximize(_) => {
                Duration::from_millis(50)
            }
            Op::Hide { method: HideMethod::ShellCloak, .. } | Op::Unhide(_) => {
                Duration::from_millis(250)
            }
            Op::Hide { .. } => Duration::from_millis(50),
            Op::RestoreAll => Duration::from_secs(1),
            Op::Launch(_) => Duration::from_secs(2),
            Op::Stall { lane, .. } => match lane {
                Lane::Focus => Duration::from_millis(300),
                Lane::Win => Duration::from_millis(50),
                Lane::Shell => Duration::from_millis(250),
            },
        }
    }
}

/// Resultado útil de un pedido.
#[derive(Debug)]
pub enum Output {
    Nothing,
    Windows(Vec<WindowInfo>),
    Inspection(windows_list::Inspection),
    Activation(Activation),
    Restored(RestoreReport),
}

pub type Ticket = u64;

#[derive(Debug)]
pub enum OpsEvent {
    /// Terminó un pedido (o venció su plazo: `Err` con "plazo vencido").
    Done {
        ticket: Ticket,
        lane: Lane,
        result: Result<Output, String>,
        /// Desde `submit` hasta que el carril empezó la llamada.
        queued_us: u64,
        /// Lo que tardó la llamada.
        call_us: u64,
    },
    /// Volvió por fin la llamada de un hilo abandonado.
    Late { ticket: Ticket, lane: Lane, call_ms: u64 },
}

struct Job {
    ticket: Ticket,
    op: Op,
    submitted: i64,
}

struct Worker {
    thread_id: u32,
    jobs: Sender<Job>,
    abandoned: Arc<AtomicBool>,
}

struct InFlight {
    ticket: Ticket,
    deadline: Instant,
    started: Instant,
}

struct LaneState {
    lane: Lane,
    /// `None`: carril muerto (se alcanzó el tope de hilos trabados).
    worker: Option<Worker>,
    queue: VecDeque<Job>,
    in_flight: Option<InFlight>,
    /// Hilos de este carril abandonados que todavía no volvieron.
    stuck: usize,
}

/// Contadores para pruebas y diagnóstico.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpsStats {
    pub abandoned: u32,
    pub late: u32,
    /// Pedidos de cloak que se hicieron con `minimize` porque Explorer no respondía.
    pub degraded_hides: u32,
}

pub struct Ops {
    brain: u32,
    lanes: Vec<LaneState>,
    results_tx: Sender<OpsEvent>,
    results_rx: Receiver<OpsEvent>,
    /// Fallos inmediatos (carril muerto), entregados en el próximo `drain`.
    local: Vec<OpsEvent>,
    next_ticket: Ticket,
    stats: OpsStats,
}

impl Ops {
    /// Arranca los tres carriles. `brain` es el hilo que recibe `WM_OPS`.
    pub fn start(brain: u32) -> std::io::Result<Self> {
        let (results_tx, results_rx) = mpsc::channel();
        let mut lanes = Vec::new();
        for lane in Lane::ALL {
            let worker = spawn_worker(lane, brain, results_tx.clone())?;
            lanes.push(LaneState {
                lane,
                worker: Some(worker),
                queue: VecDeque::new(),
                in_flight: None,
                stuck: 0,
            });
        }
        Ok(Self {
            brain,
            lanes,
            results_tx,
            results_rx,
            local: Vec::new(),
            next_ticket: 1,
            stats: OpsStats::default(),
        })
    }

    pub fn stats(&self) -> OpsStats {
        self.stats
    }

    /// Encola un pedido. Nunca bloquea.
    pub fn submit(&mut self, op: Op) -> Ticket {
        let ticket = self.next_ticket;
        self.next_ticket += 1;
        let op = self.degrade(op);
        let lane = route(&op);
        self.lanes[lane.index()].queue.push_back(Job { ticket, op, submitted: clock::now() });
        self.pump(lane);
        ticket
    }

    /// Con Explorer trabado, esconder con cloak se hace con `minimize`.
    fn degrade(&mut self, op: Op) -> Op {
        match op {
            Op::Hide { id, method: HideMethod::ShellCloak }
                if self.lanes[Lane::Shell.index()].stuck > 0 =>
            {
                self.stats.degraded_hides += 1;
                Op::Hide { id, method: HideMethod::Minimize }
            }
            other => other,
        }
    }

    fn pump(&mut self, lane: Lane) {
        let state = &mut self.lanes[lane.index()];
        if state.in_flight.is_some() {
            return;
        }
        let Some(worker) = &state.worker else {
            // Carril muerto: fallan en el acto.
            while let Some(job) = state.queue.pop_front() {
                self.local.push(OpsEvent::Done {
                    ticket: job.ticket,
                    lane,
                    result: Err("carril sin hilos disponibles".to_owned()),
                    queued_us: 0,
                    call_us: 0,
                });
            }
            wake(self.brain);
            return;
        };
        let Some(job) = state.queue.pop_front() else { return };
        let now = Instant::now();
        state.in_flight =
            Some(InFlight { ticket: job.ticket, deadline: now + job.op.deadline(), started: now });
        let thread = worker.thread_id;
        if worker.jobs.send(job).is_ok() {
            // SAFETY: PostThreadMessageW no bloquea.
            let _ = unsafe { PostThreadMessageW(thread, WM_OP_RUN, WPARAM(0), LPARAM(0)) };
        }
    }

    /// Próximo plazo a vigilar (`None` si no hay nada en vuelo).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.lanes.iter().filter_map(|l| l.in_flight.as_ref().map(|f| f.deadline)).min()
    }

    /// Resultados disponibles. Llamar en cada vuelta del brain.
    pub fn drain(&mut self) -> Vec<OpsEvent> {
        WAKE_PENDING.store(false, Ordering::Release);
        let mut out = std::mem::take(&mut self.local);
        while let Ok(event) = self.results_rx.try_recv() {
            match &event {
                OpsEvent::Done { ticket, lane, .. } => {
                    let lane = *lane;
                    let state = &mut self.lanes[lane.index()];
                    if state.in_flight.as_ref().is_some_and(|f| f.ticket == *ticket) {
                        state.in_flight = None;
                        out.push(event);
                        self.pump(lane);
                    } else {
                        // Respuesta de un hilo que abandonamos justo cuando
                        // terminaba: cuenta como tardía.
                        state.stuck = state.stuck.saturating_sub(1);
                        self.stats.late += 1;
                    }
                }
                OpsEvent::Late { lane, .. } => {
                    let state = &mut self.lanes[lane.index()];
                    state.stuck = state.stuck.saturating_sub(1);
                    self.stats.late += 1;
                    out.push(event);
                }
            }
        }
        out
    }

    /// Vence los pedidos cuyo plazo pasó: abandona esos hilos, crea otros y
    /// devuelve un `Done` con error por cada pedido vencido.
    pub fn expire(&mut self, now: Instant) -> Vec<OpsEvent> {
        let mut out = Vec::new();
        for i in 0..self.lanes.len() {
            let lane = self.lanes[i].lane;
            let expired = self.lanes[i].in_flight.as_ref().is_some_and(|f| f.deadline <= now);
            if !expired {
                continue;
            }
            let flight = self.lanes[i].in_flight.take().expect("comprobado arriba");
            if let Some(old) = self.lanes[i].worker.take() {
                old.abandoned.store(true, Ordering::Release);
                // Cuando la llamada vuelva, el hilo viejo verá WM_QUIT y terminará.
                // SAFETY: PostThreadMessageW no bloquea.
                let _ = unsafe { PostThreadMessageW(old.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            }
            self.lanes[i].stuck += 1;
            self.stats.abandoned += 1;
            let total_stuck: usize = self.lanes.iter().map(|l| l.stuck).sum();
            if total_stuck < MAX_STUCK {
                self.lanes[i].worker = spawn_worker(lane, self.brain, self.results_tx.clone()).ok();
            }
            out.push(OpsEvent::Done {
                ticket: flight.ticket,
                lane,
                result: Err("plazo vencido: hilo abandonado".to_owned()),
                queued_us: 0,
                call_us: flight.started.elapsed().as_micros() as u64,
            });
            self.pump(lane);
        }
        out
    }

    /// Hilos trabados por carril (para diagnóstico).
    pub fn stuck(&self, lane: Lane) -> usize {
        self.lanes[lane.index()].stuck
    }
}

/// Qué carril corre cada pedido. Leer la marca es una propiedad de la
/// ventana: no envía mensajes.
fn route(op: &Op) -> Lane {
    let cloaked = |id: &WindowId| hide::hidden_with(*id) == Some(HideMethod::ShellCloak);
    match op {
        Op::Focus(..) => Lane::Focus,
        Op::Place { .. }
        | Op::Scan
        | Op::Launch(_)
        | Op::Inspect(_)
        | Op::Normalize(_)
        | Op::Maximize(_) => Lane::Win,
        Op::Hide { id, method } => {
            if *method == HideMethod::ShellCloak || cloaked(id) {
                Lane::Shell
            } else {
                Lane::Win
            }
        }
        Op::Unhide(id) => {
            if cloaked(id) {
                Lane::Shell
            } else {
                Lane::Win
            }
        }
        Op::RestoreAll => Lane::Shell,
        Op::Stall { lane, .. } => *lane,
    }
}

fn wake(brain: u32) {
    if !WAKE_PENDING.swap(true, Ordering::AcqRel) {
        // SAFETY: PostThreadMessageW no bloquea.
        let _ = unsafe { PostThreadMessageW(brain, WM_OPS, WPARAM(0), LPARAM(0)) };
    }
}

/// Crea un hilo de carril **sin esperar a que arranque** (lo llama el brain:
/// esperar el arranque costó > 5 ms con la CPU al 100 %). El id sale del
/// handle; si el hilo todavía no tiene cola cuando le avisamos, igual levanta
/// los pedidos del canal al arrancar.
fn spawn_worker(lane: Lane, brain: u32, results: Sender<OpsEvent>) -> std::io::Result<Worker> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::GetThreadId;

    let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
    let abandoned = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&abandoned);
    let handle =
        std::thread::Builder::new().name(format!("wm-ops-{}", lane.name())).spawn(move || {
            input::ensure_message_queue();
            // SAFETY: cambia la prioridad del hilo actual.
            let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
            let mut ctx = Context::new(lane);
            let run_pending = |ctx: &mut Context| {
                while let Ok(job) = jobs_rx.try_recv() {
                    let started = clock::now();
                    let queued_us = clock::micros(job.submitted, started);
                    let result = ctx.run(&job.op);
                    let call_us = clock::micros_since(started);
                    let event = if flag.load(Ordering::Acquire) {
                        OpsEvent::Late { ticket: job.ticket, lane, call_ms: call_us / 1000 }
                    } else {
                        OpsEvent::Done { ticket: job.ticket, lane, result, queued_us, call_us }
                    };
                    let _ = results.send(event);
                    wake(brain);
                }
            };
            // Pedidos que llegaron antes de que existiera la cola.
            run_pending(&mut ctx);
            let mut msg = MSG::default();
            // SAFETY: bucle de mensajes estándar con un MSG válido. Para el
            // carril `shell` (STA) también bombea lo que COM necesite.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                if msg.message == WM_OP_RUN {
                    run_pending(&mut ctx);
                } else {
                    // SAFETY: mensaje recién obtenido de GetMessageW.
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            // Abandonado: soltar los objetos COM sería otra llamada a
            // Explorer, que puede estar colgado. Se dejan sin liberar.
            std::mem::forget(ctx);
        })?;
    // SAFETY: handle válido del hilo recién creado (vive mientras exista `handle`).
    let thread_id = unsafe { GetThreadId(HANDLE(handle.as_raw_handle())) };
    drop(handle); // el hilo queda suelto: nunca se espera a un carril
    Ok(Worker { thread_id, jobs: jobs_tx, abandoned })
}

/// Estado propio de cada hilo ops (los `Hider`, con su COM en `shell`).
struct Context {
    lane: Lane,
    hiders: Vec<(HideMethod, Hider)>,
}

impl Context {
    fn new(lane: Lane) -> Self {
        Self { lane, hiders: Vec::new() }
    }

    fn hider(&mut self, method: HideMethod) -> Result<&Hider, String> {
        if let Some(i) = self.hiders.iter().position(|(m, _)| *m == method) {
            return Ok(&self.hiders[i].1);
        }
        let h = Hider::new(method)?;
        self.hiders.push((method, h));
        Ok(&self.hiders.last().expect("recién agregado").1)
    }

    fn run(&mut self, op: &Op) -> Result<Output, String> {
        match op {
            Op::Focus(id, pid) => {
                if *pid != 0 && windows_list::window_pid(*id) != *pid {
                    return Err("la ventana ya no existe".to_owned());
                }
                Ok(Output::Activation(windows_list::activate(*id)))
            }
            Op::Place { id, rect } => {
                windows_list::place_async(*id, *rect).map(|()| Output::Nothing)
            }
            Op::Inspect(target) => windows_list::inspect(*target).map(Output::Inspection),
            Op::Normalize(id) => windows_list::normalize(*id).map(|()| Output::Nothing),
            Op::Maximize(id) => windows_list::maximize(*id).map(|()| Output::Nothing),
            Op::Hide { id, method } => self.hider(*method)?.hide(*id).map(|()| Output::Nothing),
            Op::Unhide(id) => {
                // `show` usa el método de la marca; en `win` no hace falta COM.
                let method = if self.lane == Lane::Shell {
                    HideMethod::ShellCloak
                } else {
                    HideMethod::Minimize
                };
                self.hider(method)?.show(*id).map(|()| Output::Nothing)
            }
            Op::Scan => Ok(Output::Windows(windows_list::user_windows())),
            Op::RestoreAll => Ok(Output::Restored(hide::restore_all())),
            Op::Launch(path) => std::process::Command::new(path)
                .spawn()
                .map(|_| Output::Nothing)
                .map_err(|e| e.to_string()),
            Op::Stall { ms, .. } => {
                std::thread::sleep(Duration::from_millis(u64::from(*ms)));
                Ok(Output::Nothing)
            }
        }
    }
}

/// Prioridades para CPU alta (architecture.md §4.4): proceso `ABOVE_NORMAL` y
/// sin "power throttling" (sin ventana visible, Windows 11 podría tratarnos
/// como proceso de fondo).
pub fn boost_process() {
    use windows::Win32::System::Threading::{
        ABOVE_NORMAL_PRIORITY_CLASS, GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetPriorityClass, SetProcessInformation,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: operan sobre el proceso actual con una estructura local válida.
    unsafe {
        let _ = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS);
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const _ as *const core::ffi::c_void,
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

/// Prioridad del brain: `THREAD_PRIORITY_HIGHEST`.
pub fn boost_brain_thread() {
    use windows::Win32::System::Threading::THREAD_PRIORITY_HIGHEST;
    // SAFETY: cambia la prioridad del hilo actual.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
}
