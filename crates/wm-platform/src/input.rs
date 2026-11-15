//! Hook de teclado de bajo nivel para Caps = Hyper (architecture.md §5).
//!
//! Corre en un hilo dedicado de prioridad máxima. El callback hace lo mínimo:
//! ejecuta la máquina de estados pura (`wm_model::keyboard`), inyecta lo que
//! haga falta y avisa al hilo "brain" con `PostThreadMessageW`. No asigna
//! memoria, no toma locks y no escribe logs.
//!
//! Protecciones (architecture.md §5.4):
//! - **Vigía:** Raw Input ve las mismas teclas físicas. Si ve varias seguidas
//!   que el hook no vio, Windows quitó el hook en silencio y se reinstala.
//! - **Reposo y bloqueo:** al volver del reposo o desbloquear la sesión, se
//!   reinstala de forma preventiva.
//! - **Modo seguro:** si el brain no responde a una acción en 2 s, el hook deja
//!   pasar todas las teclas hasta que el brain vuelva. El teclado nunca queda
//!   secuestrado.
//! - **Callback lento:** si un callback tarda más de 100 ms, Windows pudo haber
//!   dejado pasar eventos sin consultarnos (por ejemplo, soltar Caps). La
//!   máquina de estados vuelve a reposo para no quedar "trabada".
//! - **Inyección fuera del callback:** `SendInput` tarda ~0,85 ms (medido), así
//!   que lo hace un hilo aparte y el callback solo le avisa.

use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_INPUTSINK, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HHOOK,
    KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLKHF_UP, MSG, PostThreadMessageW, RegisterClassW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_INPUT,
    WM_POWERBROADCAST, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use wm_model::keyboard::{
    HyperAction, HyperKeyboard, Inject, KeyEvent, VK_CAPITAL, VK_LWIN, VK_RMENU, VK_RWIN,
};

/// Marca en `dwExtraInfo` de los eventos que inyectamos nosotros ("WMHY").
pub const INJECTED_MARKER: usize = 0x574D_4859;

/// Mensaje que recibe el hilo brain (acciones de Hyper y avisos).
pub const WM_HYPER: u32 = WM_APP + 1;

/// Tecla máscara: código virtual sin asignar (técnica de AutoHotkey,
/// architecture.md §5.3). Evita que Windows abra menús al soltar Alt/Win.
const MASK_VK: u16 = 0xE8;

/// Tiempo sin respuesta del brain antes de entrar en modo seguro.
const BRAIN_TIMEOUT_MS: u32 = 2000;

/// Teclas físicas seguidas que el hook no vio antes de darlo por muerto.
const WATCHDOG_MISSES: u32 = 3;

/// Intervalo mínimo entre reinstalaciones por el vigía.
const WATCHDOG_COOLDOWN_MS: u64 = 5000;

/// Duración de callback a partir de la cual se asume que Windows dejó pasar
/// eventos sin consultarnos.
const STALL_NS: u64 = 100_000_000;

/// Mensaje al hilo inyector.
const WM_INJECT: u32 = WM_APP + 2;

// Constantes de mensajes de sesión y energía (winuser.h / wtsapi32.h).
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const RI_KEY_BREAK: u16 = 0x1;

static BRAIN_THREAD: AtomicU32 = AtomicU32::new(0);
static INJECTOR_THREAD: AtomicU32 = AtomicU32::new(0);
/// Hora (reloj del hook, ms) de la acción más vieja sin respuesta; 0 = ninguna.
static PENDING_SINCE: AtomicU32 = AtomicU32::new(0);
static SAFE_MODE: AtomicBool = AtomicBool::new(false);
/// Hora (reloj del hook, ms) en que se entró en modo seguro.
static SAFE_MODE_SINCE: AtomicU32 = AtomicU32::new(0);
static TEST_MODE: AtomicBool = AtomicBool::new(false);

static SAFE_MODE_ENTRIES: AtomicU32 = AtomicU32::new(0);
static STALLS: AtomicU32 = AtomicU32::new(0);
static REINSTALLS_WATCHDOG: AtomicU32 = AtomicU32::new(0);
static REINSTALLS_RESUME: AtomicU32 = AtomicU32::new(0);
static REINSTALLS_UNLOCK: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static MACHINE: RefCell<HyperKeyboard> = const { RefCell::new(HyperKeyboard::new()) };
    static HOOK: Cell<isize> = const { Cell::new(0) };
    /// El hook vio una tecla física desde el último evento de Raw Input.
    static SEEN_BY_HOOK: Cell<bool> = const { Cell::new(true) };
    static MISSES: Cell<u32> = const { Cell::new(0) };
    static LAST_WATCHDOG_REINSTALL: Cell<u64> = const { Cell::new(0) };
}

// ---------------------------------------------------------------------------
// Latencia del callback (spike 1: p99.9 < 200 µs, máximo < 5 ms)
// ---------------------------------------------------------------------------

const BUCKET_US: u64 = 10;
const BUCKETS: usize = 200; // 0–2000 µs en pasos de 10 µs, más un bucket de desborde

struct Histogram {
    buckets: [AtomicU32; BUCKETS + 1],
    count: AtomicU64,
    max_ns: AtomicU64,
}

impl Histogram {
    const fn new() -> Self {
        Self {
            buckets: [const { AtomicU32::new(0) }; BUCKETS + 1],
            count: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
        }
    }

    fn record(&self, ns: u64) {
        let bucket = ((ns / 1000) / BUCKET_US).min(BUCKETS as u64) as usize;
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    fn stats(&self) -> LatencyStats {
        let counts: Vec<u64> =
            self.buckets.iter().map(|b| u64::from(b.load(Ordering::Relaxed))).collect();
        let total: u64 = counts.iter().sum();
        let percentile = |p: f64| -> u64 {
            if total == 0 {
                return 0;
            }
            let target = ((total as f64) * p).ceil() as u64;
            let mut acc = 0;
            for (i, c) in counts.iter().enumerate() {
                acc += c;
                if acc >= target {
                    return (i as u64 + 1) * BUCKET_US; // límite superior del bucket
                }
            }
            u64::MAX
        };
        LatencyStats {
            events: self.count.load(Ordering::Relaxed),
            p50_us: percentile(0.50),
            p99_us: percentile(0.99),
            p999_us: percentile(0.999),
            max_us: self.max_ns.load(Ordering::Relaxed) as f64 / 1000.0,
        }
    }
}

/// Eventos donde el callback no inyectó nada.
static LAT_PLAIN: Histogram = Histogram::new();
/// Eventos donde el callback llamó a `SendInput` (Bloq Mayús o máscara).
static LAT_INJECTING: Histogram = Histogram::new();
static QPC_FREQ: AtomicU64 = AtomicU64::new(0);

/// Percentiles con resolución de 10 µs (cada valor es la cota superior).
#[derive(Debug, Clone, Copy, Default)]
pub struct LatencyStats {
    pub events: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HookStats {
    pub plain: LatencyStats,
    pub injecting: LatencyStats,
    pub reinstalls_watchdog: u32,
    pub reinstalls_resume: u32,
    pub reinstalls_unlock: u32,
    pub safe_mode_entries: u32,
    /// Callbacks de más de 100 ms (tras los que se reinició el estado).
    pub stalls: u32,
}

pub fn hook_stats() -> HookStats {
    HookStats {
        plain: LAT_PLAIN.stats(),
        injecting: LAT_INJECTING.stats(),
        reinstalls_watchdog: REINSTALLS_WATCHDOG.load(Ordering::Relaxed),
        reinstalls_resume: REINSTALLS_RESUME.load(Ordering::Relaxed),
        reinstalls_unlock: REINSTALLS_UNLOCK.load(Ordering::Relaxed),
        safe_mode_entries: SAFE_MODE_ENTRIES.load(Ordering::Relaxed),
        stalls: STALLS.load(Ordering::Relaxed),
    }
}

/// Reloj del sistema en ms, el mismo que usan los eventos del hook.
pub fn tick_ms() -> u32 {
    use windows::Win32::System::SystemInformation::GetTickCount;
    // SAFETY: sin precondiciones.
    unsafe { GetTickCount() }
}

fn qpc() -> i64 {
    let mut t = 0i64;
    // SAFETY: puntero a una variable local válida.
    let _ = unsafe { QueryPerformanceCounter(&mut t) };
    t
}

fn ticks_to_ns(ticks: i64) -> u64 {
    let freq = QPC_FREQ.load(Ordering::Relaxed).max(1);
    (ticks.max(0) as u64).saturating_mul(1_000_000_000) / freq
}

// ---------------------------------------------------------------------------
// Inyección
// ---------------------------------------------------------------------------

fn key_input(vk: u16, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    // Las teclas Windows y el Alt derecho son "extendidas".
    if matches!(vk as u8, VK_LWIN | VK_RWIN | VK_RMENU) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED_MARKER,
            },
        },
    }
}

fn send(inputs: &[INPUT]) {
    // SAFETY: `inputs` es un slice válido de estructuras INPUT completas.
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
}

const INJECT_CAPS: usize = 1;
const INJECT_MASK: usize = 2;

/// Pide al hilo inyector que haga la inyección (no bloquea el callback).
fn request_inject(inject: Inject) {
    let (kind, vk) = match inject {
        Inject::None => return,
        Inject::ToggleCapsLock => (INJECT_CAPS, 0),
        Inject::MaskThenRelease(vk) => (INJECT_MASK, vk),
    };
    let thread = INJECTOR_THREAD.load(Ordering::Relaxed);
    // SAFETY: PostThreadMessageW no bloquea.
    let posted =
        unsafe { PostThreadMessageW(thread, WM_INJECT, WPARAM(kind), LPARAM(isize::from(vk))) };
    if posted.is_err() {
        perform(kind, vk); // sin hilo inyector: hacerlo acá antes que perderlo
    }
}

fn perform(kind: usize, vk: u8) {
    match kind {
        INJECT_CAPS => {
            let caps = u16::from(VK_CAPITAL);
            send(&[key_input(caps, false), key_input(caps, true)]);
        }
        INJECT_MASK => {
            let release = key_input(vk.into(), true);
            send(&[key_input(MASK_VK, false), key_input(MASK_VK, true), release]);
        }
        _ => {}
    }
}

/// Hilo que hace las inyecciones pedidas por el callback, en orden.
fn start_injector() -> std::io::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("wm-inject".into()).spawn(move || {
        ensure_message_queue();
        let _ = tx.send(current_thread_id());
        let mut msg = MSG::default();
        // SAFETY: bucle de mensajes estándar con un MSG válido.
        while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
            if msg.message == WM_INJECT {
                perform(msg.wParam.0, msg.lParam.0 as u8);
            }
        }
    })?;
    let thread = rx.recv().map_err(|e| std::io::Error::other(e.to_string()))?;
    INJECTOR_THREAD.store(thread, Ordering::Relaxed);
    Ok(())
}

/// Inyecta una tecla inocua marcada. Sirve para que nuestro proceso cuente como
/// el que recibió la última entrada y Windows le permita `SetForegroundWindow`
/// (architecture.md §5.6). La tecla llega a la app enfocada, que la ignora.
pub fn send_neutral_key() {
    send(&[key_input(MASK_VK, false), key_input(MASK_VK, true)]);
}

// ---------------------------------------------------------------------------
// Mensajes hacia el brain
// ---------------------------------------------------------------------------

const KIND_LETTER: usize = 1;
const KIND_DIGIT: usize = 2;
const KIND_ESCAPE: usize = 3;
const KIND_OTHER: usize = 4;
const KIND_REINSTALLED: usize = 5;
const KIND_STALL: usize = 6;

/// Por qué se reinstaló el hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReinstallReason {
    /// El vigía detectó que Windows lo había quitado.
    Watchdog,
    /// Volvió del reposo.
    Resume,
    /// Se desbloqueó la sesión.
    Unlock,
}

/// Lo que recibe el brain del hilo de entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Acción de Hyper. `time_ms` es la hora de la tecla (reloj de [`tick_ms`]);
    /// `qpc`, el instante en que el hook la vio (reloj de [`crate::clock`]).
    Action { action: HyperAction, time_ms: u32, qpc: i64 },
    HookReinstalled(ReinstallReason),
    /// Un callback tardó `ms` milisegundos; el estado del teclado se reinició.
    HookStalled { ms: u32 },
}

fn post_raw(kind: usize, data: isize) {
    post_raw_to(BRAIN_THREAD.load(Ordering::Relaxed), kind, data);
}

fn post_raw_to(thread: u32, kind: usize, data: isize) {
    // SAFETY: PostThreadMessageW no bloquea; si el hilo no existe, falla sin efectos.
    let _ = unsafe { PostThreadMessageW(thread, WM_HYPER, WPARAM(kind), LPARAM(data)) };
}

/// wParam: tipo en los bits 0–7, QPC del hook en los bits 8–63.
fn kind_with_qpc(kind: usize, qpc: i64) -> usize {
    kind | ((qpc.max(0) as usize) << 8)
}

/// Solo para pruebas: manda al brain `thread` una acción como si viniera del
/// hook, con la hora actual.
pub fn simulate_action(thread: u32, action: HyperAction) {
    let (kind, data) = encode_action(action, tick_ms());
    post_raw_to(thread, kind_with_qpc(kind, crate::clock::now()), data);
}

fn encode_action(action: HyperAction, time_ms: u32) -> (usize, isize) {
    let (kind, value, thumb) = match action {
        HyperAction::Letter { letter, thumb } => (KIND_LETTER, letter, thumb),
        HyperAction::Digit { digit, thumb } => (KIND_DIGIT, digit, thumb),
        HyperAction::Escape => (KIND_ESCAPE, 0, false),
        HyperAction::Other { vk, thumb } => (KIND_OTHER, vk, thumb),
    };
    // lParam (64 bits): valor en 0–7, pulgar en 8, hora de la tecla en 16–47.
    (kind, isize::from(value) | (isize::from(thumb) << 8) | ((time_ms as isize) << 16))
}

fn post_action(action: HyperAction, time_ms: u32, qpc: i64) {
    // Marca el pedido como pendiente (si no había otro más viejo).
    let _ = PENDING_SINCE.compare_exchange(0, time_ms.max(1), Ordering::AcqRel, Ordering::Relaxed);
    let (kind, data) = encode_action(action, time_ms);
    post_raw(kind_with_qpc(kind, qpc), data);
}

/// Mensaje al brain: hay resultados de ops para drenar (ver [`crate::ops`]).
pub const WM_OPS: u32 = WM_APP + 3;
/// Mensaje al brain (solo pruebas): ping con el QPC del emisor en `lParam`.
pub const WM_PING: u32 = WM_APP + 4;

/// Por qué se despertó el brain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    Input(InputEvent),
    /// Hay resultados de ops.
    Ops,
    /// Ping de prueba con el QPC del emisor.
    Ping(i64),
    /// Venció el plazo pedido.
    Timeout,
    /// `WM_QUIT`.
    Quit,
}

/// Espera el próximo evento del brain, como mucho hasta `deadline` (`None` =
/// sin límite: 0 % de CPU, sin timers). Despacha los demás mensajes del hilo.
///
/// Después de procesar cada evento de entrada, el brain debe llamar a
/// [`acknowledge`].
pub fn wait(deadline: Option<std::time::Instant>) -> Wake {
    use windows::Win32::UI::WindowsAndMessaging::{
        MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT,
        WM_QUIT,
    };
    let mut msg = MSG::default();
    loop {
        // SAFETY: MSG válido; extracción estándar de la cola del hilo.
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            match msg.message {
                WM_QUIT => return Wake::Quit,
                WM_OPS => return Wake::Ops,
                WM_PING => return Wake::Ping(msg.lParam.0 as i64),
                _ => {}
            }
            if let Some(event) = decode(&msg) {
                return Wake::Input(event);
            }
            // SAFETY: mensaje recién obtenido de PeekMessageW.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let timeout = match deadline {
            None => u32::MAX, // INFINITE
            Some(d) => {
                let left = d.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    return Wake::Timeout;
                }
                // Redondeo hacia arriba: nunca despertar antes del plazo.
                left.as_micros().div_ceil(1000).min(u128::from(u32::MAX - 1)) as u32
            }
        };
        // SAFETY: sin handles; espera mensajes de la cola de este hilo.
        let r = unsafe {
            MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if r.0 == 0x102 {
            // WAIT_TIMEOUT
            return Wake::Timeout;
        }
    }
}

/// Bloquea el hilo actual (el brain) hasta el próximo evento. Devuelve `None`
/// al recibir `WM_QUIT`. Sin timers ni consultas: 0 % de CPU mientras espera.
///
/// Después de procesar cada evento, el brain debe llamar a [`acknowledge`].
pub fn next_event() -> Option<InputEvent> {
    let mut msg = MSG::default();
    loop {
        // SAFETY: MSG válido; bucle de mensajes estándar.
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if result.0 == 0 || result.0 == -1 {
            return None; // WM_QUIT o error
        }
        if let Some(event) = decode(&msg) {
            return Some(event);
        }
        // SAFETY: mensaje recién obtenido de GetMessageW.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// El brain terminó de procesar lo pendiente. Si mientras tanto el hilo de
/// entrada había pasado a modo seguro, devuelve cuántos ms lleva en ese modo
/// (el hook sale de él con la próxima tecla).
pub fn acknowledge() -> Option<u32> {
    PENDING_SINCE.store(0, Ordering::Release);
    SAFE_MODE
        .load(Ordering::Acquire)
        .then(|| tick_ms().wrapping_sub(SAFE_MODE_SINCE.load(Ordering::Relaxed)))
}

fn decode(msg: &MSG) -> Option<InputEvent> {
    if msg.message != WM_HYPER {
        return None;
    }
    let value = (msg.lParam.0 & 0xFF) as u8;
    let thumb = msg.lParam.0 & 0x100 != 0;
    let time_ms = (msg.lParam.0 >> 16) as u32;
    let qpc = (msg.wParam.0 >> 8) as i64;
    let action = match msg.wParam.0 & 0xFF {
        KIND_LETTER => HyperAction::Letter { letter: value, thumb },
        KIND_DIGIT => HyperAction::Digit { digit: value, thumb },
        KIND_ESCAPE => HyperAction::Escape,
        KIND_OTHER => HyperAction::Other { vk: value, thumb },
        KIND_REINSTALLED => {
            let reason = match value {
                0 => ReinstallReason::Watchdog,
                1 => ReinstallReason::Resume,
                _ => ReinstallReason::Unlock,
            };
            return Some(InputEvent::HookReinstalled(reason));
        }
        KIND_STALL => return Some(InputEvent::HookStalled { ms: msg.lParam.0 as u32 }),
        _ => return None,
    };
    Some(InputEvent::Action { action, time_ms, qpc })
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/// Tecla de prueba (solo con `WM_TEST=1`): Hyper + F12 bloquea el callback
/// 1,5 s para provocar que Windows quite el hook y probar el vigía.
const TEST_STALL_VK: u8 = 0x7B;

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        // SAFETY: reenvío estándar de los parámetros recibidos.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let start = qpc();
    // SAFETY: con `code >= 0`, `lparam` apunta a un KBDLLHOOKSTRUCT válido
    // durante el callback (contrato de WH_KEYBOARD_LL).
    let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let ours = info.dwExtraInfo == INJECTED_MARKER;
    let down = info.flags.0 & LLKHF_UP.0 == 0;
    if !ours && down && info.flags.0 & LLKHF_INJECTED.0 == 0 {
        SEEN_BY_HOOK.with(|s| s.set(true));
    }

    let mut injected = false;
    let mut test_stall = false;
    let pass = if ours || info.vkCode > 0xFF || !brain_alive(info.time) {
        true
    } else {
        let event = KeyEvent { vk: info.vkCode as u8, down, time_ms: info.time };
        let decision = MACHINE.with(|m| m.borrow_mut().on_key(event));
        if decision.inject != Inject::None {
            injected = true;
            request_inject(decision.inject);
        }
        if let Some(action) = decision.action {
            post_action(action, info.time, start);
            if TEST_MODE.load(Ordering::Relaxed)
                && matches!(action, HyperAction::Other { vk: TEST_STALL_VK, .. })
            {
                test_stall = true;
                std::thread::sleep(Duration::from_millis(1500));
            }
        }
        decision.pass
    };

    let ns = ticks_to_ns(qpc() - start);
    if ns > STALL_NS {
        // Windows pudo haber dejado pasar eventos sin consultarnos mientras
        // tanto (por ejemplo, soltar Caps): volver a reposo para no quedar
        // con Hyper "trabado". Ver architecture.md §13.3.
        MACHINE.with(|m| m.borrow_mut().reset());
        STALLS.fetch_add(1, Ordering::Relaxed);
        post_raw(KIND_STALL, (ns / 1_000_000) as isize);
    }
    if !test_stall {
        if injected { LAT_INJECTING.record(ns) } else { LAT_PLAIN.record(ns) }
    }

    if pass {
        // SAFETY: reenvío estándar de los parámetros recibidos.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    } else {
        LRESULT(1)
    }
}

/// Decide si el brain responde. Si una acción lleva más de
/// `BRAIN_TIMEOUT_MS` sin respuesta, entra en modo seguro (todo pasa); sale
/// cuando el brain responde.
fn brain_alive(now_ms: u32) -> bool {
    let pending = PENDING_SINCE.load(Ordering::Acquire);
    if SAFE_MODE.load(Ordering::Acquire) {
        if pending != 0 {
            return false;
        }
        SAFE_MODE.store(false, Ordering::Release);
        MACHINE.with(|m| m.borrow_mut().reset());
        return true;
    }
    if pending != 0 && now_ms.wrapping_sub(pending) > BRAIN_TIMEOUT_MS {
        SAFE_MODE_SINCE.store(now_ms, Ordering::Relaxed);
        SAFE_MODE.store(true, Ordering::Release);
        SAFE_MODE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        MACHINE.with(|m| m.borrow_mut().reset());
        return false;
    }
    true
}

fn install_hook() -> windows::core::Result<HHOOK> {
    // SAFETY: `hook_proc` cumple la firma de HOOKPROC; el módulo es el
    // ejecutable actual, que vive mientras dura el proceso.
    unsafe {
        let module = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).ok();
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0)
    }
}

/// Reinstala el hook en el hilo de entrada (donde corre este código).
fn reinstall(reason: ReinstallReason) {
    let old = HOOK.with(Cell::get);
    if old != 0 {
        // SAFETY: handle propio; si Windows ya lo quitó, la llamada falla sin efectos.
        let _ = unsafe { UnhookWindowsHookEx(HHOOK(old as *mut _)) };
    }
    let Ok(hook) = install_hook() else { return };
    HOOK.with(|h| h.set(hook.0 as isize));
    MACHINE.with(|m| m.borrow_mut().reset());
    SEEN_BY_HOOK.with(|s| s.set(true));
    MISSES.with(|m| m.set(0));
    let counter = match reason {
        ReinstallReason::Watchdog => &REINSTALLS_WATCHDOG,
        ReinstallReason::Resume => &REINSTALLS_RESUME,
        ReinstallReason::Unlock => &REINSTALLS_UNLOCK,
    };
    counter.fetch_add(1, Ordering::Relaxed);
    let code = match reason {
        ReinstallReason::Watchdog => 0,
        ReinstallReason::Resume => 1,
        ReinstallReason::Unlock => 2,
    };
    post_raw(KIND_REINSTALLED, code);
}

// ---------------------------------------------------------------------------
// Ventana oculta del hilo de entrada: Raw Input, reposo y sesión
// ---------------------------------------------------------------------------

unsafe extern "system" fn sink_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_INPUT => {
            on_raw_input(lparam);
        }
        WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC => {
            reinstall(ReinstallReason::Resume);
        }
        WM_WTSSESSION_CHANGE if wparam.0 == WTS_SESSION_UNLOCK => {
            reinstall(ReinstallReason::Unlock);
        }
        WM_WTSSESSION_CHANGE if wparam.0 == WTS_SESSION_LOCK => {
            // Las teclas soltadas con la sesión bloqueada no llegan al hook.
            MACHINE.with(|m| m.borrow_mut().reset());
        }
        _ => {}
    }
    // SAFETY: procesamiento por defecto; para WM_INPUT además libera el buffer.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn on_raw_input(lparam: LPARAM) {
    let mut raw = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    // SAFETY: `raw` es un buffer válido del tamaño indicado en `size`.
    let read = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut _),
            RID_INPUT,
            Some(&mut raw as *mut RAWINPUT as *mut _),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if read == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return;
    }
    // Teclas inyectadas por software no tienen dispositivo: no cuentan.
    if raw.header.hDevice.0.is_null() {
        return;
    }
    // SAFETY: `dwType` indica que la unión contiene un RAWKEYBOARD.
    let flags = unsafe { raw.data.keyboard.Flags };
    if flags & RI_KEY_BREAK != 0 {
        return; // solo contamos "abajo", igual que el hook
    }

    if SEEN_BY_HOOK.with(|s| s.replace(false)) {
        MISSES.with(|m| m.set(0));
        return;
    }
    let misses = MISSES.with(|m| {
        m.set(m.get() + 1);
        m.get()
    });
    if misses >= WATCHDOG_MISSES {
        // SAFETY: sin precondiciones.
        let now = unsafe { GetTickCount64() };
        let last = LAST_WATCHDOG_REINSTALL.with(Cell::get);
        if last == 0 || now - last >= WATCHDOG_COOLDOWN_MS {
            LAST_WATCHDOG_REINSTALL.with(|l| l.set(now));
            reinstall(ReinstallReason::Watchdog);
        }
    }
}

fn create_sink_window() -> windows::core::Result<HWND> {
    // SAFETY: clase y ventana propias; la ventana es top-level oculta (las
    // message-only no reciben los broadcasts de energía, architecture.md §4.3).
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(sink_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("wm-input-sink"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("wm-input-sink"),
            w!("wm-input-sink"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )?;
        let device = RAWINPUTDEVICE {
            usUsagePage: 0x01, // Generic Desktop
            usUsage: 0x06,     // Keyboard
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32)?;
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION)?;
        Ok(hwnd)
    }
}

/// Arranca el hilo de entrada (hook + vigía). Los eventos llegan como
/// mensajes al hilo `brain_thread_id`, que ya debe tener cola de mensajes
/// (llamar a [`ensure_message_queue`] antes).
///
/// Con `test_mode`, Hyper + F12 bloquea el callback 1,5 s (para probar el vigía).
pub fn start_keyboard_hook(brain_thread_id: u32, test_mode: bool) -> std::io::Result<()> {
    BRAIN_THREAD.store(brain_thread_id, Ordering::Relaxed);
    TEST_MODE.store(test_mode, Ordering::Relaxed);
    let mut freq = 0i64;
    // SAFETY: puntero a una variable local válida.
    let _ = unsafe { QueryPerformanceFrequency(&mut freq) };
    QPC_FREQ.store(freq.max(1) as u64, Ordering::Relaxed);
    start_injector()?;

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("wm-input".into()).spawn(move || {
        // SAFETY: llamada sin precondiciones sobre el hilo actual.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
        }
        let setup = install_hook().and_then(|hook| {
            HOOK.with(|h| h.set(hook.0 as isize));
            create_sink_window().map(|_| ())
        });
        if let Err(e) = setup {
            let _ = tx.send(Err(std::io::Error::other(e.to_string())));
            return;
        }
        let _ = tx.send(Ok(()));

        // El hook y la ventana oculta se atienden dentro de este bucle.
        let mut msg = MSG::default();
        // SAFETY: bucle de mensajes estándar con un MSG válido.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(HHOOK(HOOK.with(Cell::get) as *mut _));
        }
    })?;
    rx.recv().map_err(|e| std::io::Error::other(e.to_string()))?
}

/// Crea la cola de mensajes del hilo actual (necesaria antes de recibir
/// `PostThreadMessageW`).
pub fn ensure_message_queue() {
    use windows::Win32::UI::WindowsAndMessaging::{PM_NOREMOVE, PeekMessageW};
    let mut msg = MSG::default();
    // SAFETY: MSG válido; PM_NOREMOVE no consume mensajes.
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
}

static QUIT_THREAD: AtomicU32 = AtomicU32::new(0);

/// Hace que Ctrl+C o cerrar la consola terminen el bucle de mensajes del hilo
/// `thread_id` (le postea `WM_QUIT`), para salir limpiamente.
pub fn quit_on_console_close(thread_id: u32) {
    use windows::Win32::System::Console::SetConsoleCtrlHandler;
    use windows::Win32::UI::WindowsAndMessaging::WM_QUIT;
    use windows::core::BOOL;

    unsafe extern "system" fn handler(_ctrl: u32) -> BOOL {
        let thread = QUIT_THREAD.load(Ordering::Relaxed);
        // SAFETY: PostThreadMessageW no bloquea.
        let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        true.into()
    }

    QUIT_THREAD.store(thread_id, Ordering::Relaxed);
    // SAFETY: `handler` cumple la firma de PHANDLER_ROUTINE y vive para siempre.
    let _ = unsafe { SetConsoleCtrlHandler(Some(handler), true) };
}

pub fn current_thread_id() -> u32 {
    // SAFETY: sin precondiciones.
    unsafe { GetCurrentThreadId() }
}

/// Solo para pruebas: manda al brain `thread` un ping con el QPC actual.
pub fn post_ping(thread: u32) {
    let now = crate::clock::now();
    // SAFETY: PostThreadMessageW no bloquea.
    let _ = unsafe { PostThreadMessageW(thread, WM_PING, WPARAM(0), LPARAM(now as isize)) };
}
