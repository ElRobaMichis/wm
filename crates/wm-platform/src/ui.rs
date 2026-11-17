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

/// El hilo se oculta solo si el mapa lleva este tiempo visible **sin
/// pedidos** (el brain lo oculta a los 30 s sin actividad; esto cubre al
/// brain trabado). Cada pedido lo renueva: antes contaba desde que apareció
/// y el mapa desaparecía en pleno uso (prueba del usuario).
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
    /// No se mostró a propósito (por ejemplo, un juego o video a pantalla
    /// completa en ese monitor).
    Skipped(&'static str),
    Failed(String),
    /// Hito 4d: tras un cambio de pantallas, ¿se rearmaron las ventanas?
    Screens {
        n: usize,
        rebuilt: bool,
        ms: f64,
    },
}

/// Duración del fundido de entrada (design.md §3.4 usa 140 ms; el mapa,
/// algo menos, para que no parezca lento).
const FADE: Duration = Duration::from_millis(120);

enum Cmd {
    Show {
        layer: Layer,
        views: Views,
        at: Instant,
    },
    Layer(Layer),
    /// Pintar esta tecla en `accent` (la recién usada).
    Light(char),
    Hide,
    /// Un aviso corto en pantalla.
    Notice(String),
    /// Borde de foco: seguir a esta ventana (`None`: apagado).
    Border(Option<isize>),
    /// La lista de ventanas de una letra (`None`: cerrarla).
    List(Option<wm_model::listview::ListView>),
    /// Hito 4d: cambiaron las pantallas.
    Screens,
}

/// Cuánto dura un aviso en pantalla (con los fundidos).
const NOTICE: Duration = Duration::from_millis(2200);

/// Las ventanas del mapa, para esconderlas si el hilo se traba.
static WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// La ventana del mapa que se ve ahora (0: ninguna). La lee el carril
/// `anim` para que el fantasma de un vuelo quede debajo del mapa.
static SHOWN: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// La ventana del mapa visible, si hay.
pub fn shown_window() -> Option<isize> {
    let raw = SHOWN.load(Ordering::Acquire);
    (raw != 0).then_some(raw)
}

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

    pub fn light(&mut self, letter: char) {
        self.send(Cmd::Light(letter));
    }

    /// Un aviso corto en pantalla (en el monitor de la ventana al frente).
    pub fn notice(&mut self, text: String) {
        self.send(Cmd::Notice(text));
    }

    /// Borde de foco alrededor de `win` (`None`: ninguno).
    pub fn border(&mut self, win: Option<isize>) {
        self.send(Cmd::Border(win));
    }

    /// La lista de ventanas de una letra (plan-apps.md §8); `None` la cierra.
    pub fn list(&mut self, view: Option<wm_model::listview::ListView>) {
        self.send(Cmd::List(view));
    }

    /// Hito 4d: cambiaron las pantallas; rearmar lo que haga falta ya (no
    /// en el próximo mapa).
    pub fn screens_changed(&mut self) {
        self.send(Cmd::Screens);
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
    screens: Vec<Option<(overlay_text::MapVisual, overlay_text::MapVisual)>>,
}

struct State {
    overlays: Overlays,
    built: Option<Built>,
    shown: Option<usize>,
    timer: usize,
    /// El aviso en pantalla: monitor, visual y su temporizador.
    toast: Option<(usize, windows::UI::Composition::ContainerVisual)>,
    toast_timer: usize,
    /// Borde de foco (se crea la primera vez que se pide).
    border: Option<crate::focus_border::FocusBorder>,
    /// La lista de ventanas de una letra: monitor y visual.
    list: Option<(usize, windows::UI::Composition::ContainerVisual)>,
    /// La capa del mapa (para volver a mostrarlo al cerrar la lista).
    layer: Layer,
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
    let mut st = State {
        overlays,
        built: None,
        shown: None,
        timer: 0,
        toast: None,
        toast_timer: 0,
        border: None,
        list: None,
        layer: Layer::Off,
    };
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
        } else if msg.hwnd.is_invalid() && msg.message == crate::focus_border::WM_BORDER_UPDATE {
            if let Some(b) = st.border.as_mut() {
                b.update();
            }
        } else if msg.hwnd.is_invalid()
            && msg.message == WM_TIMER
            && msg.wParam.0 == st.toast_timer
            && st.toast_timer != 0
        {
            st.end_toast();
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
        if self.shown.is_some() && !matches!(cmd, Cmd::Hide) {
            self.arm_timer();
        }
        match cmd {
            Cmd::Show { layer, views, at } => match (self.shown, {
                self.layer = layer;
                self.show(layer, views)
            }) {
                (_, Ok(None)) => UiEvent::Skipped("hay una app a pantalla completa en ese monitor"),
                // Ya estaba visible ahí: solo se actualizó (tras cambiar de
                // escritorio); no es una aparición nueva para el log.
                (Some(before), Ok(Some(monitor))) if before == monitor => {
                    UiEvent::Shown { ms: 0.0, monitor }
                }
                (_, Ok(Some(monitor))) => {
                    UiEvent::Shown { ms: at.elapsed().as_secs_f64() * 1000.0, monitor }
                }
                (_, Err(e)) => {
                    self.hide();
                    UiEvent::Failed(format!("no se pudo mostrar el mapa: {e}"))
                }
            },
            Cmd::Layer(layer) => {
                self.layer = layer;
                if let Some(i) = self.shown {
                    let _ = self.set_layer(i, layer);
                }
                UiEvent::Shown { ms: 0.0, monitor: self.shown.unwrap_or(0) }
            }
            Cmd::Light(letter) => {
                if let Some(i) = self.shown
                    && let Some(Some((apps, window))) =
                        self.built.as_mut().map(|b| &mut b.screens[i])
                {
                    let _ = apps.light(Some(letter));
                    let _ = window.light(Some(letter));
                }
                UiEvent::Shown { ms: 0.0, monitor: self.shown.unwrap_or(0) }
            }
            Cmd::Hide => {
                self.hide();
                UiEvent::Shown { ms: 0.0, monitor: usize::MAX }
            }
            Cmd::Border(win) => {
                if self.border.is_none() && win.is_some() {
                    match crate::focus_border::FocusBorder::new(self.overlays.compositor()) {
                        Ok(b) => self.border = Some(b),
                        Err(e) => {
                            return UiEvent::Failed(format!(
                                "no se pudo crear el borde de foco: {e}"
                            ));
                        }
                    }
                }
                if let Some(b) = self.border.as_mut() {
                    b.follow(win.map(windows_list::WindowId));
                }
                UiEvent::Shown { ms: 0.0, monitor: usize::MAX }
            }
            Cmd::Screens => {
                let start = Instant::now();
                let result = self.ensure_current();
                if let Some(b) = self.border.as_mut() {
                    b.update();
                }
                match result {
                    Ok(rebuilt) => UiEvent::Screens {
                        n: self.overlays.screens().len(),
                        rebuilt,
                        ms: start.elapsed().as_secs_f64() * 1000.0,
                    },
                    Err(e) => UiEvent::Failed(format!("no se pudo rearmar el mapa: {e}")),
                }
            }
            Cmd::List(view) => {
                if let Err(e) = self.list(view) {
                    self.end_list();
                    return UiEvent::Failed(format!(
                        "no se pudo mostrar la lista de ventanas: {e}"
                    ));
                }
                UiEvent::Shown { ms: 0.0, monitor: usize::MAX }
            }
            Cmd::Notice(text) => {
                if let Err(e) = self.notice(&text) {
                    return UiEvent::Failed(format!("no se pudo mostrar el aviso: {e}"));
                }
                UiEvent::Shown { ms: 0.0, monitor: usize::MAX }
            }
        }
    }

    /// Si las ventanas ya no corresponden a los monitores de ahora: termina
    /// el aviso, oculta el mapa y las rearma. `Ok(true)` si las rearmó.
    fn ensure_current(&mut self) -> windows::core::Result<bool> {
        if self.overlays.is_current() {
            return Ok(false);
        }
        self.end_toast();
        if self.shown.is_some() {
            self.hide();
        }
        self.overlays.rebuild_screens()?;
        if let Ok(mut w) = WINDOWS.lock() {
            *w = self.overlays.screens().iter().map(|s| s.hwnd.0 as isize).collect();
        }
        self.built = None;
        self.shown = None;
        Ok(true)
    }

    /// `Ok(None)`: no se muestra (pantalla completa).
    fn show(&mut self, layer: Layer, views: Views) -> windows::core::Result<Option<usize>> {
        // Se conectó o desconectó un monitor, o cambió la escala: ventanas
        // nuevas y contenido nuevo.
        self.ensure_current()?;
        let n = self.overlays.screens().len();
        if self.built.as_ref().is_none_or(|b| b.views != views) {
            // Contenido nuevo (cambiaron las asignaciones): se rearma.
            self.end_toast();
            for s in self.overlays.screens() {
                s.root.Children()?.RemoveAll()?;
            }
            self.built = Some(Built { views, screens: (0..n).map(|_| None).collect() });
        }
        let i = pick_monitor(&self.overlays);
        if fullscreen_on(&self.overlays.screens()[i].bounds) {
            return Ok(None);
        }
        let built = self.built.as_mut().expect("recién armado");
        if built.screens[i].is_none() {
            let s = &self.overlays.screens()[i];
            let (w, h) = (s.bounds.width() as f32, s.bounds.height() as f32);
            let c = self.overlays.compositor();
            let apps = overlay_text::build_view(c, &s.root, w, h, s.dpi, &built.views[0])?;
            let window = overlay_text::build_view(c, &s.root, w, h, s.dpi, &built.views[1])?;
            built.screens[i] = Some((apps, window));
        }
        // Ya visible en este monitor (contenido nuevo tras cambiar de
        // escritorio): sin fundido. Si no, pulsación nueva: ninguna tecla en
        // `accent` todavía.
        let again = self.shown == Some(i);
        if !again && let Some(Some((apps, window))) = self.built.as_mut().map(|b| &mut b.screens[i])
        {
            apps.light(None)?;
            window.light(None)?;
        }
        self.set_layer(i, layer)?;
        if !again {
            self.fade_in(i)?;
        }
        self.overlays.show_only(i);
        SHOWN.store(self.overlays.screens()[i].hwnd.0 as isize, Ordering::Release);
        self.shown = Some(i);
        self.arm_timer();
        Ok(Some(i))
    }

    /// Fundido de entrada (lo anima DWM); instantáneo si el sistema pide
    /// reducir animaciones. Ocultar es siempre instantáneo: nunca queda a
    /// medio camino.
    fn fade_in(&self, i: usize) -> windows::core::Result<()> {
        let root = &self.overlays.screens()[i].root;
        if windows_list::reduced_motion() {
            return root.SetOpacity(1.0);
        }
        let c = self.overlays.compositor();
        let anim = c.CreateScalarKeyFrameAnimation()?;
        anim.InsertKeyFrame(0.0, 0.0)?;
        anim.InsertKeyFrame(1.0, 1.0)?;
        anim.SetDuration(windows::Foundation::TimeSpan {
            Duration: (FADE.as_nanos() / 100) as i64,
        })?;
        root.StartAnimation(&windows::core::HSTRING::from("Opacity"), &anim)
    }

    /// (Re)arma el autocierre: un temporizador de hilo nuevo desde ahora.
    fn arm_timer(&mut self) {
        if self.timer != 0 {
            // SAFETY: temporizador propio de este hilo.
            let _ = unsafe { KillTimer(None, self.timer) };
        }
        // SAFETY: temporizador de hilo (sin ventana); WM_TIMER llega a la cola.
        self.timer = unsafe { SetTimer(None, 0, SELF_HIDE_MS, None) };
    }

    fn set_layer(&self, i: usize, layer: Layer) -> windows::core::Result<()> {
        let Some(Some((apps, window))) = self.built.as_ref().map(|b| &b.screens[i]) else {
            return Ok(());
        };
        apps.container.SetIsVisible(layer != Layer::Window)?;
        window.container.SetIsVisible(layer == Layer::Window)
    }

    /// Un aviso: reemplaza al anterior. No en un monitor con pantalla
    /// completa (un juego o un video).
    fn notice(&mut self, text: &str) -> windows::core::Result<()> {
        // Monitores cambiados: se rearma ya (antes el aviso se omitía hasta
        // el próximo mapa; hito 4d).
        self.ensure_current()?;
        self.end_toast();
        let i = pick_monitor(&self.overlays);
        if fullscreen_on(&self.overlays.screens()[i].bounds) {
            return Ok(());
        }
        let s = &self.overlays.screens()[i];
        let (w, h) = (s.bounds.width() as f32, s.bounds.height() as f32);
        let visual = overlay_text::build_toast(
            self.overlays.compositor(),
            &s.root,
            w,
            h,
            s.dpi,
            text,
            NOTICE,
        )?;
        if self.shown != Some(i) {
            // La ventana no tiene el mapa: el fundido del mapa la pudo dejar
            // transparente.
            s.root.SetOpacity(1.0)?;
            self.overlays.show_also(i);
        }
        self.toast = Some((i, visual));
        // SAFETY: temporizador de hilo (sin ventana).
        self.toast_timer = unsafe { SetTimer(None, 0, NOTICE.as_millis() as u32 + 100, None) };
        Ok(())
    }

    /// Muestra o actualiza la lista de una letra (reemplaza al mapa mientras
    /// dura, en el mismo overlay); `None` la cierra y el mapa vuelve.
    fn list(&mut self, view: Option<wm_model::listview::ListView>) -> windows::core::Result<()> {
        let Some(view) = view else {
            self.end_list();
            return Ok(());
        };
        let keep = self.list.as_ref().map(|(i, _)| *i);
        self.remove_list();
        self.ensure_current()?;
        let i = keep
            .filter(|&i| i < self.overlays.screens().len())
            .unwrap_or_else(|| pick_monitor(&self.overlays));
        let s = &self.overlays.screens()[i];
        let (w, h) = (s.bounds.width() as f32, s.bounds.height() as f32);
        let visual =
            overlay_text::build_list(self.overlays.compositor(), &s.root, w, h, s.dpi, &view)?;
        self.map_visible(i, false);
        if self.shown != Some(i) {
            s.root.SetOpacity(1.0)?;
            self.overlays.show_also(i);
        }
        self.list = Some((i, visual));
        Ok(())
    }

    fn remove_list(&mut self) {
        if let Some((i, visual)) = self.list.take()
            && let Some(s) = self.overlays.screens().get(i)
            && let Ok(children) = s.root.Children()
        {
            let _ = children.Remove(&visual);
        }
    }

    /// Cierra la lista: el mapa vuelve si estaba; si no, la ventana se
    /// esconde (salvo que tenga un aviso).
    fn end_list(&mut self) {
        let Some((i, _)) = self.list.as_ref().map(|(i, v)| (*i, v.clone())) else { return };
        self.remove_list();
        if self.shown == Some(i) {
            self.map_visible(i, true);
        } else if self.toast.as_ref().is_none_or(|(t, _)| *t != i) {
            self.overlays.hide_one(i);
        }
    }

    /// Esconde (o vuelve a mostrar) el mapa del monitor `i`, si está armado.
    fn map_visible(&self, i: usize, on: bool) {
        if on {
            let _ = self.set_layer(i, self.layer);
            return;
        }
        if let Some(Some((apps, window))) = self.built.as_ref().and_then(|b| b.screens.get(i)) {
            let _ = apps.container.SetIsVisible(false);
            let _ = window.container.SetIsVisible(false);
        }
    }

    /// Quita el aviso (y esconde su ventana si el mapa no está ahí).
    fn end_toast(&mut self) {
        if self.toast_timer != 0 {
            // SAFETY: temporizador propio.
            let _ = unsafe { KillTimer(None, self.toast_timer) };
            self.toast_timer = 0;
        }
        if let Some((i, visual)) = self.toast.take() {
            if let Some(s) = self.overlays.screens().get(i)
                && let Ok(children) = s.root.Children()
            {
                let _ = children.Remove(&visual);
            }
            if self.shown != Some(i) {
                self.overlays.hide_one(i);
            }
        }
    }

    fn hide(&mut self) {
        SHOWN.store(0, Ordering::Release);
        self.overlays.hide();
        self.shown = None;
        // Un aviso en curso sigue (su ventana vuelve a mostrarse).
        if let Some((i, _)) = &self.toast {
            if let Some(s) = self.overlays.screens().get(*i) {
                let _ = s.root.SetOpacity(1.0);
            }
            self.overlays.show_also(*i);
        }
        if self.timer != 0 {
            // SAFETY: temporizador propio de este hilo.
            let _ = unsafe { KillTimer(None, self.timer) };
            self.timer = 0;
        }
    }
}

/// ¿La ventana al frente cubre todo `monitor` (un juego o un video a
/// pantalla completa)? Ahí el mapa estorbaría.
fn fullscreen_on(monitor: &Rect) -> bool {
    windows_list::foreground()
        .filter(|&id| !crate::hide::is_maximized(id))
        .and_then(ghost::frame_bounds)
        .is_some_and(|f| f == *monitor)
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
