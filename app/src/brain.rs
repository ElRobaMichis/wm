//! El brain: ejecuta el modelo ([`World`]) con la plataforma real
//! (plan-core.md §2).
//!
//! Traduce en los dos sentidos: los eventos de Windows (teclas del hook,
//! resultados de los carriles de [`Ops`]) a [`Event`], y los [`Effect`] del
//! modelo a pedidos de `ops` o líneas de log. No decide nada: las decisiones
//! están en `wm_model::world`, con pruebas.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use wm_model::keyboard::Layer;
use wm_model::place::MonitorObs;
use wm_model::world::{
    Effect, Event, FlyFail, FlyReport, FocusReport, Obs, Req, WinInfo, WinKey, World,
};
use wm_platform::anim::{Anim, AnimEvent};
use wm_platform::input::{self, InputEvent, ReinstallReason};
use wm_platform::ops::{Op, Ops, OpsEvent, Output, Ticket};
use wm_platform::ui::{Ui, UiEvent};
use wm_platform::windows_list::{self, WindowId};

/// Teclas de prueba (solo con `WM_TEST=1`).
const VK_F10: u8 = 0x79;
const VK_F11: u8 = 0x7A;
const VK_F12: u8 = 0x7B;

/// Tamaño a partir del cual el log se rota al arrancar.
const LOG_MAX_BYTES: u64 = 1024 * 1024;

/// Log en un hilo aparte: el brain solo manda líneas. `println!` puede
/// trabarse (por ejemplo, si el usuario selecciona texto en la terminal) y
/// escribir a disco también, así que el brain nunca escribe directo.
///
/// Escribe en la consola (si hay) y en `%LOCALAPPDATA%\wm\wm.log`, con la
/// hora desde el arranque.
#[derive(Clone)]
pub struct Log(Sender<String>);

impl Log {
    pub fn start() -> (Self, std::thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<String>();
        let handle = std::thread::Builder::new()
            .name("wm-log".into())
            .spawn(move || {
                let started = Instant::now();
                let mut file = open_log_file();
                for line in rx {
                    println!("{line}");
                    if let Some(f) = file.as_mut() {
                        let t = started.elapsed().as_secs_f64();
                        if writeln!(f, "[{t:9.3}] {line}").is_err() {
                            file = None; // disco lleno o sin permiso: solo consola
                        }
                    }
                }
            })
            .expect("no se pudo crear el hilo de log");
        (Self(tx), handle)
    }

    pub fn line(&self, s: impl Into<String>) {
        let _ = self.0.send(s.into());
    }
}

/// `%LOCALAPPDATA%\wm\wm.log` (el anterior pasa a `wm.1.log` si es grande).
pub fn log_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("wm").join("wm.log"))
}

fn open_log_file() -> Option<std::fs::File> {
    let path = log_path()?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_file_name("wm.1.log"));
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
}

/// Qué espera el brain de cada pedido en vuelo.
enum Wait {
    Scan(Req),
    Focus(Req),
    Launch(Req),
    Inspect(Req),
    Place(Req),
    RestoreAtStart,
    RestoreAtExit,
}

pub struct Brain {
    pub world: World,
    pub ops: Ops,
    pub log: Log,
    test_mode: bool,
    started: Instant,
    waits: HashMap<Ticket, Wait>,
    /// Carril `anim` (hito 1e): se crea con la primera capa de ventana;
    /// `None` también con `WM_ANIM=0`.
    anim: Option<Anim>,
    anim_allowed: bool,
    /// Vuelos en curso: ticket del carril → pedido del modelo.
    flights: HashMap<wm_platform::anim::Ticket, Req>,
    /// Hilo `ui` (mapa de teclado, hito 2a); `None` con `WM_MAP=0`.
    ui: Option<Ui>,
    /// El modelo pidió terminar.
    pub quit: bool,
}

impl Brain {
    pub fn new(world: World, ops: Ops, log: Log, test_mode: bool) -> Self {
        Self {
            world,
            ops,
            log,
            test_mode,
            started: Instant::now(),
            waits: HashMap::new(),
            anim: None,
            anim_allowed: std::env::var_os("WM_ANIM").is_none_or(|v| v != "0"),
            flights: HashMap::new(),
            ui: std::env::var_os("WM_MAP")
                .is_none_or(|v| v != "0")
                .then(|| Ui::start(input::current_thread_id())),
            quit: false,
        }
    }

    /// Respuestas del hilo `ui` (y su vigilante).
    fn collect_ui(&mut self) {
        let Some(ui) = self.ui.as_mut() else { return };
        let stuck = ui.expire(Instant::now());
        let events = ui.drain();
        if stuck {
            self.log.line("El mapa de teclado se trabó: lo escondo y lo apago por esta sesión.");
            self.feed(Event::MapEnabled(false));
        }
        for e in events {
            match e {
                UiEvent::Ready(Ok(())) => self.feed(Event::MapEnabled(true)),
                UiEvent::Ready(Err(e)) => {
                    self.log.line(format!("Mapa de teclado no disponible: {e}."))
                }
                UiEvent::Shown { ms, monitor } if ms > 0.0 => {
                    self.log.line(format!(
                        "Mapa de teclado visible (monitor {}), {ms:.1} ms.",
                        monitor + 1
                    ));
                }
                UiEvent::Shown { .. } => {}
                UiEvent::Failed(e) => self.log.line(e),
                UiEvent::Skipped(why) => {
                    self.log.line(format!("Mapa de teclado no mostrado: {why}."))
                }
            }
        }
    }

    /// Crea el carril `anim` la primera vez que se usa la capa de ventana:
    /// así el COM, el cloak y el anfitrión están listos antes de la tecla, y
    /// quien no usa la capa no lo paga.
    fn warm_anim(&mut self) {
        if self.anim.is_none() && self.anim_allowed {
            self.anim = Some(Anim::start(input::current_thread_id()));
        }
    }

    fn now_us(&self) -> u64 {
        self.started.elapsed().as_micros() as u64
    }

    /// Hasta cuándo puede dormir el brain: el primer plazo entre los carriles
    /// de `ops` y los chequeos del modelo. `None`: nada pendiente.
    pub fn next_deadline(&self) -> Option<Instant> {
        let world = self.world.deadline().map(|us| self.started + Duration::from_micros(us));
        let anim = self.anim.as_ref().and_then(Anim::next_deadline);
        let ui = self.ui.as_ref().and_then(Ui::next_deadline);
        [self.ops.next_deadline(), world, anim, ui].into_iter().flatten().min()
    }

    /// Si venció el plazo del modelo, lo despierta.
    pub fn tick(&mut self) {
        if self.world.deadline().is_some_and(|t| t <= self.now_us()) {
            self.feed(Event::Tick);
        }
    }

    /// Le pasa un evento al modelo y ejecuta lo que pida.
    pub fn feed(&mut self, event: Event) {
        let effects = self.world.step(event, self.now_us());
        for effect in effects {
            match effect {
                Effect::Scan { req } => {
                    let t = self.ops.submit(Op::Scan);
                    self.waits.insert(t, Wait::Scan(req));
                }
                Effect::Focus { req, win } => {
                    let t = self.ops.submit(Op::Focus(WindowId(win.hwnd), win.pid));
                    self.waits.insert(t, Wait::Focus(req));
                }
                Effect::Launch { req, path } => {
                    let t = self.ops.submit(Op::Launch(path));
                    self.waits.insert(t, Wait::Launch(req));
                }
                Effect::Inspect { req, win } => {
                    let target = win.map(|k| (WindowId(k.hwnd), k.pid));
                    let t = self.ops.submit(Op::Inspect(target));
                    self.waits.insert(t, Wait::Inspect(req));
                }
                Effect::Place { req, win, rect } => {
                    let t = self.ops.submit(Op::Place { id: WindowId(win.hwnd), rect });
                    self.waits.insert(t, Wait::Place(req));
                }
                Effect::Fly { req, win, rect, to, ms, .. } => match self.anim.as_mut() {
                    Some(anim) => {
                        let t = anim.fly(WindowId(win.hwnd), rect, to, ms);
                        self.flights.insert(t, req);
                    }
                    None => {
                        let why = "sin carril de animación".to_owned();
                        self.feed(Event::Flown { req, result: Err(FlyFail { fatal: false, why }) });
                    }
                },
                Effect::Land { win } => {
                    if let Some(anim) = self.anim.as_mut() {
                        anim.land(WindowId(win.hwnd));
                    }
                }
                Effect::ShowMap { layer, views } => {
                    if let (Some(ui), [apps, window]) = (self.ui.as_mut(), views.as_slice()) {
                        ui.show(layer, std::sync::Arc::new([apps.clone(), window.clone()]));
                    }
                }
                Effect::MapKey { letter } => {
                    if let Some(ui) = self.ui.as_mut() {
                        ui.light(letter);
                    }
                }
                Effect::MapLayer { layer } => {
                    if let Some(ui) = self.ui.as_mut() {
                        ui.set_layer(layer);
                    }
                }
                Effect::HideMap => {
                    if let Some(ui) = self.ui.as_mut() {
                        ui.hide();
                    }
                }
                Effect::ResetKeyboard => {
                    self.log.line("El mapa estuvo visible 30 s: reinicio el teclado.");
                    input::request_reset();
                }
                Effect::Normalize { win } => {
                    self.ops.submit(Op::Normalize(WindowId(win.hwnd)));
                }
                Effect::Maximize { win } => {
                    self.ops.submit(Op::Maximize(WindowId(win.hwnd)));
                }
                Effect::Log(line) => self.log.line(line),
                Effect::Quit => self.quit = true,
            }
        }
    }

    /// Mostrar todo lo que tenga nuestra marca (arranque = recuperación,
    /// architecture.md §9.5).
    pub fn restore_at_start(&mut self) {
        let t = self.ops.submit(Op::RestoreAll);
        self.waits.insert(t, Wait::RestoreAtStart);
    }

    /// Salida (limpia o tras un pánico): mostrar todo, esperando como mucho
    /// `limit`.
    pub fn restore_at_exit(&mut self, limit: Duration) {
        if let Some(anim) = self.anim.as_mut() {
            anim.land_all();
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.hide();
        }
        let t = self.ops.submit(Op::RestoreAll);
        self.waits.insert(t, Wait::RestoreAtExit);
        let until = Instant::now() + limit;
        while self.waits.values().any(|w| matches!(w, Wait::RestoreAtExit)) {
            let deadline = self.ops.next_deadline().map_or(until, |d| d.min(until));
            if matches!(input::wait(Some(deadline)), input::Wake::Quit) || Instant::now() >= until {
                break;
            }
            self.collect();
        }
    }

    /// Un evento del hilo de entrada.
    pub fn on_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::Action { action, held, time_ms, .. } => {
                if self.test_mode && self.test_key(action) {
                    return;
                }
                let age_ms = input::tick_ms().wrapping_sub(time_ms);
                self.feed(Event::Action { action, held, age_ms });
            }
            InputEvent::Layer(layer) => {
                if layer == Layer::Window {
                    self.warm_anim();
                }
                self.feed(Event::Layer(layer));
            }
            InputEvent::HookReinstalled(reason) => {
                let why = match reason {
                    ReinstallReason::Watchdog => "el vigía detectó que Windows lo había quitado",
                    ReinstallReason::Resume => "la PC volvió del reposo",
                    ReinstallReason::Unlock => "se desbloqueó la sesión",
                };
                self.log.line(format!("Hook reinstalado: {why}."));
                self.land_all();
                self.feed(Event::KeyboardReset);
            }
            InputEvent::KeyboardReset(_) => {
                self.land_all();
                self.feed(Event::KeyboardReset);
            }
            InputEvent::HookStalled { ms } => {
                self.log.line(format!("El hook tardó {ms} ms: estado del teclado reiniciado."));
                self.land_all();
                self.feed(Event::KeyboardReset);
            }
        }
    }

    /// Teclas de prueba (`WM_TEST=1`). Devuelve `true` si la consumió.
    fn test_key(&mut self, action: wm_model::keyboard::HyperAction) -> bool {
        use wm_model::keyboard::HyperAction;
        match action {
            HyperAction::Other { vk: VK_F10, .. } => {
                panic!("prueba de pánico (Caps + F10)");
            }
            HyperAction::Other { vk: VK_F11, .. } => {
                self.log.line("PRUEBA: el programa se traba 6 s. Pasados 2 s, Caps + B tiene que");
                self.log.line("        escribir una B en vez de ir a Brave (modo seguro).");
                std::thread::sleep(Duration::from_secs(6));
                true
            }
            HyperAction::Other { vk: VK_F12, .. } => {
                self.log.line("PRUEBA: el hook se trabó 1,5 s. Escribí unas letras:");
                self.log.line("        tienen que salir normales (Caps no puede quedar trabado).");
                true
            }
            _ => false,
        }
    }

    fn land_all(&mut self) {
        if let Some(anim) = self.anim.as_mut() {
            anim.land_all();
        }
    }

    /// Respuestas del carril `anim` (y su vigilante).
    fn collect_anim(&mut self) {
        let Some(anim) = self.anim.as_mut() else { return };
        let mut events = anim.expire(Instant::now());
        events.extend(anim.drain());
        for event in events {
            match event {
                AnimEvent::Ready(Ok(())) => self.feed(Event::FlyEnabled(true)),
                AnimEvent::Ready(Err(e)) => {
                    self.log.line(format!("Animación no disponible: {e}."));
                }
                AnimEvent::Flown { ticket, win, result } => {
                    let Some(req) = self.flights.remove(&ticket) else { continue };
                    let result = match result {
                        Ok(d) => Ok(FlyReport { arrived: d.arrived, flight_ms: d.flight_ms }),
                        Err(e) => {
                            // La ventana puede haber quedado con cloak: que
                            // la muestre el carril `shell` (usa la marca).
                            if e.fatal && wm_platform::hide::hidden_with(win).is_some() {
                                self.ops.submit(Op::Unhide(win));
                            }
                            Err(FlyFail { fatal: e.fatal, why: e.why })
                        }
                    };
                    self.feed(Event::Flown { req, result });
                }
            }
        }
    }

    /// Resultados de los carriles (y plazos vencidos).
    pub fn collect(&mut self) {
        self.collect_anim();
        self.collect_ui();
        let mut events = self.ops.expire(Instant::now());
        events.extend(self.ops.drain());
        for event in events {
            match event {
                OpsEvent::Done { ticket, result, .. } => {
                    if let Some(wait) = self.waits.remove(&ticket) {
                        self.on_done(wait, result);
                    }
                }
                OpsEvent::Late { lane, call_ms, .. } => {
                    self.log.line(format!(
                        "Aviso: una llamada del carril {} volvió tras {call_ms} ms.",
                        lane.name()
                    ));
                }
            }
        }
    }

    fn on_done(&mut self, wait: Wait, result: Result<Output, String>) {
        match wait {
            Wait::Scan(req) => {
                let result = result.and_then(|out| match out {
                    Output::Windows(list) => {
                        Ok((list.into_iter().map(win_info).collect(), focus()))
                    }
                    _ => Err("respuesta inesperada".to_owned()),
                });
                self.feed(Event::Windows { req, result });
            }
            Wait::Focus(req) => {
                let result = result.and_then(|out| match out {
                    Output::Activation(a) => Ok(FocusReport {
                        ok: a.ok,
                        neutral_key: a.used_neutral_key,
                        elapsed_us: a.elapsed.as_micros() as u64,
                    }),
                    _ => Err("respuesta inesperada".to_owned()),
                });
                self.feed(Event::Focused { req, result });
            }
            Wait::Launch(req) => self.feed(Event::Launched { req, result: result.map(|_| ()) }),
            Wait::Inspect(req) => {
                let result = result.and_then(|out| match out {
                    Output::Inspection(i) => Ok(obs(i)),
                    _ => Err("respuesta inesperada".to_owned()),
                });
                self.feed(Event::Inspected { req, result });
            }
            Wait::Place(req) => self.feed(Event::Placed { req, result: result.map(|_| ()) }),
            Wait::RestoreAtStart => {
                if let Ok(Output::Restored(r)) = result
                    && !r.restored.is_empty()
                {
                    let n = r.restored.len();
                    self.log.line(format!("Restauradas {n} ventanas de una sesión anterior."));
                }
            }
            Wait::RestoreAtExit => {}
        }
    }
}

fn obs(i: windows_list::Inspection) -> Obs {
    Obs {
        key: WinKey { hwnd: i.id.0, pid: i.pid },
        app: i.app,
        title: i.title,
        frame: i.frame,
        outer: i.outer,
        dpi: i.dpi,
        maximized: i.maximized,
        minimized: i.minimized,
        monitors: i
            .monitors
            .iter()
            .map(|m| MonitorObs { bounds: m.bounds, work: m.work_area, dpi: m.dpi })
            .collect(),
        no_fly: i.no_fly,
    }
}

fn win_info(w: windows_list::WindowInfo) -> WinInfo {
    WinInfo { key: WinKey { hwnd: w.id.0, pid: w.pid }, app: w.app, title: w.title }
}

/// La ventana con el foco ahora (lectura que no bloquea).
fn focus() -> Option<WinKey> {
    windows_list::foreground().map(|id| WinKey { hwnd: id.0, pid: windows_list::window_pid(id) })
}
