//! Estado del core y sus decisiones (plan-core.md §1).
//!
//! Puro: [`World::step`] recibe un evento (una tecla, el resultado de un
//! pedido a Windows) y devuelve efectos (pedidos nuevos, líneas de log). El
//! brain (`app/src/brain.rs`) ejecuta los efectos con `wm-platform` y le
//! devuelve los resultados como eventos. Así toda la lógica se prueba sin
//! Windows, con una plataforma de mentira.
//!
//! Hito 0: la capa de apps (`Caps` + letra) y la capa del teclado. La capa de
//! ventana y los escritorios se suman en los hitos 1 y 4.

use std::collections::HashMap;

use crate::apps::{AppBinding, Target, resolve};
use crate::keyboard::{HyperAction, Layer};

/// Identificador de un pedido del modelo a la plataforma.
pub type Req = u64;

/// Una ventana: su HWND y el proceso dueño. El proceso evita actuar sobre un
/// HWND que Windows reutilizó para otra ventana (plan-core.md §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WinKey {
    pub hwnd: isize,
    pub pid: u32,
}

/// Una ventana de usuario, como la ve un `Scan`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinInfo {
    pub key: WinKey,
    /// Ejecutable sin extensión, en minúsculas.
    pub app: String,
    pub title: String,
}

/// Resultado de traer una ventana al frente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusReport {
    pub ok: bool,
    pub neutral_key: bool,
    pub elapsed_us: u64,
}

/// Lo que le llega al modelo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Una acción de Hyper. `age_ms`: cuánto tardó en llegar desde la tecla.
    Action {
        action: HyperAction,
        held: u8,
        age_ms: u32,
    },
    /// Cambió la capa del teclado.
    Layer(Layer),
    /// El estado del teclado se reinició (callback lento, hook reinstalado):
    /// la capa vuelve a `Off` sin aviso del hook.
    KeyboardReset,
    /// Resultado de un `Scan`: ventanas en orden Z (la primera, arriba) y la
    /// que tiene el foco.
    Windows {
        req: Req,
        result: Result<(Vec<WinInfo>, Option<WinKey>), String>,
    },
    Focused {
        req: Req,
        result: Result<FocusReport, String>,
    },
    Launched {
        req: Req,
        result: Result<(), String>,
    },
    /// Asignaciones nuevas (al arrancar o al recargar `apps.txt`).
    Bindings(Vec<AppBinding>),
}

/// Lo que el modelo le pide a la plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Scan {
        req: Req,
    },
    Focus {
        req: Req,
        win: WinKey,
    },
    Launch {
        req: Req,
        path: String,
    },
    Log(String),
    /// Terminar el core (Hyper + Esc, hasta que exista la bandeja).
    Quit,
}

/// Antigüedad máxima de una acción para ejecutarla: una que llega tarde (el
/// programa estuvo trabado) ya no es lo que el usuario quiere ahora.
pub const MAX_ACTION_AGE_MS: u32 = 1000;

/// Cuántas veces se intentó traer una app al frente y cómo salió.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activations {
    pub total: u32,
    pub ok: u32,
    pub neutral_key: u32,
}

#[derive(Debug, Clone)]
enum Pending {
    /// Enumerar ventanas para resolver `Caps` + letra.
    Scan {
        letter: u8,
        key_us: u64,
    },
    Focus {
        letter: u8,
        app: String,
        title: String,
        key_us: u64,
    },
    Launch {
        letter: u8,
        app: String,
        path: String,
    },
}

/// El estado del core.
#[derive(Debug, Default)]
pub struct World {
    bindings: Vec<AppBinding>,
    layer: Layer,
    next_req: Req,
    pending: HashMap<Req, Pending>,
    activations: Activations,
}

impl World {
    pub fn new(bindings: Vec<AppBinding>) -> Self {
        Self { bindings, ..Self::default() }
    }

    pub fn layer(&self) -> Layer {
        self.layer
    }

    pub fn activations(&self) -> Activations {
        self.activations
    }

    pub fn bindings(&self) -> &[AppBinding] {
        &self.bindings
    }

    /// Pedidos todavía sin respuesta.
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }

    fn req(&mut self) -> Req {
        self.next_req += 1;
        self.next_req
    }

    /// Procesa un evento. `now_us`: reloj monótono en microsegundos (el mismo
    /// para todos los eventos), para medir desde la tecla.
    pub fn step(&mut self, event: Event, now_us: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        match event {
            Event::Action { action, held: _, age_ms } => {
                if age_ms > MAX_ACTION_AGE_MS {
                    let what = describe(action);
                    out.push(Effect::Log(format!(
                        "Acción descartada: llegó {age_ms} ms tarde ({what})."
                    )));
                } else {
                    self.action(action, age_ms, now_us, &mut out);
                }
            }
            Event::Layer(layer) => self.layer = layer,
            Event::KeyboardReset => self.layer = Layer::Off,
            Event::Bindings(b) => self.bindings = b,
            Event::Windows { req, result } => {
                let Some(Pending::Scan { letter, key_us }) = self.pending.remove(&req) else {
                    return out;
                };
                match result {
                    Ok((windows, fg)) => self.go_to_app(letter, key_us, &windows, fg, &mut out),
                    Err(e) => out.push(Effect::Log(format!("Caps + {}: {e}", letter as char))),
                }
            }
            Event::Focused { req, result } => {
                let Some(Pending::Focus { letter, app, title, key_us }) = self.pending.remove(&req)
                else {
                    return out;
                };
                let key = letter as char;
                match result {
                    Ok(f) => {
                        self.activations.total += 1;
                        self.activations.ok += u32::from(f.ok);
                        self.activations.neutral_key += u32::from(f.neutral_key);
                        let status = if f.ok { "ok" } else { "FALLÓ" };
                        let trick = if f.neutral_key { ", con tecla neutra" } else { "" };
                        let since = now_us.saturating_sub(key_us) as f64 / 1000.0;
                        out.push(Effect::Log(format!(
                            "Caps + {key} → {app} «{}»: {status} en {:.1} ms{trick} \
                             (desde la tecla {since:.1} ms)",
                            shorten(&title, 50),
                            f.elapsed_us as f64 / 1000.0,
                        )));
                    }
                    Err(e) => out.push(Effect::Log(format!("Caps + {key}: {e}"))),
                }
            }
            Event::Launched { req, result } => {
                let Some(Pending::Launch { letter, app, path }) = self.pending.remove(&req) else {
                    return out;
                };
                let key = letter as char;
                out.push(Effect::Log(match result {
                    Ok(()) => format!("Caps + {key} → abriendo {app} ({path})"),
                    Err(e) => format!("Caps + {key} → no se pudo abrir {app} ({path}): {e}"),
                }));
            }
        }
        out
    }

    fn action(&mut self, action: HyperAction, age_ms: u32, now_us: u64, out: &mut Vec<Effect>) {
        match action {
            HyperAction::Letter { letter, thumb: false } => {
                let req = self.req();
                let key_us = now_us.saturating_sub(u64::from(age_ms) * 1000);
                self.pending.insert(req, Pending::Scan { letter, key_us });
                out.push(Effect::Scan { req });
            }
            HyperAction::Letter { letter, thumb: true } => {
                let key = letter as char;
                out.push(Effect::Log(format!("Caps + Alt + {key}: capa de ventana, pendiente.")));
            }
            HyperAction::Digit { digit, thumb } => {
                let what = if thumb { "llevar a" } else { "ir a" };
                out.push(Effect::Log(format!(
                    "Caps + {digit}: {what} escritorio {digit}, pendiente."
                )));
            }
            HyperAction::Escape => out.push(Effect::Quit),
            HyperAction::Other { .. } => {}
        }
    }

    fn go_to_app(
        &mut self,
        letter: u8,
        key_us: u64,
        windows: &[WinInfo],
        foreground: Option<WinKey>,
        out: &mut Vec<Effect>,
    ) {
        let list: Vec<(WinKey, &str)> = windows.iter().map(|w| (w.key, w.app.as_str())).collect();
        let key = letter as char;
        match resolve(letter, &self.bindings, &list, foreground) {
            Target::Focus { window, app } => {
                let title =
                    windows.iter().find(|w| w.key == window).map_or("", |w| w.title.as_str());
                let req = self.req();
                let pending =
                    Pending::Focus { letter, app: app.to_owned(), title: title.to_owned(), key_us };
                self.pending.insert(req, pending);
                out.push(Effect::Focus { req, win: window });
            }
            Target::Launch { app, path } => {
                let req = self.req();
                let pending =
                    Pending::Launch { letter, app: app.to_owned(), path: path.to_owned() };
                self.pending.insert(req, pending);
                out.push(Effect::Launch { req, path: path.to_owned() });
            }
            Target::Nothing => {
                out.push(Effect::Log(format!(
                    "Caps + {key}: no hay ninguna app abierta con la {key}."
                )));
            }
        }
    }
}

/// Texto corto de una acción, para el log.
pub fn describe(action: HyperAction) -> String {
    match action {
        HyperAction::Letter { letter, thumb: false } => format!("Caps + {}", letter as char),
        HyperAction::Letter { letter, thumb: true } => format!("Caps + Alt + {}", letter as char),
        HyperAction::Digit { digit, .. } => format!("Caps + {digit}"),
        HyperAction::Escape => "Caps + Esc".to_owned(),
        HyperAction::Other { vk, .. } => format!("Caps + tecla {vk:#04x}"),
    }
}

/// Recorta un texto a `max` caracteres con "…".
pub fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::parse_bindings;

    /// Plataforma de mentira: ventanas fijas; ejecuta los efectos y devuelve
    /// los eventos de respuesta, como haría el brain.
    struct Sim {
        windows: Vec<WinInfo>,
        foreground: Option<WinKey>,
        focus_ok: bool,
        logs: Vec<String>,
        focused: Vec<WinKey>,
        launched: Vec<String>,
        quit: bool,
    }

    fn win(hwnd: isize, app: &str) -> WinInfo {
        WinInfo {
            key: WinKey { hwnd, pid: hwnd as u32 + 1000 },
            app: app.into(),
            title: app.into(),
        }
    }

    impl Sim {
        fn new(apps: &[&str]) -> Self {
            let windows: Vec<WinInfo> =
                apps.iter().enumerate().map(|(i, a)| win(i as isize + 1, a)).collect();
            let foreground = windows.first().map(|w| w.key);
            Self {
                windows,
                foreground,
                focus_ok: true,
                logs: Vec::new(),
                focused: Vec::new(),
                launched: Vec::new(),
                quit: false,
            }
        }

        /// Corre `event` y todas las respuestas hasta que no quede nada.
        fn run(&mut self, w: &mut World, event: Event) {
            let mut queue = vec![event];
            while let Some(e) = queue.pop() {
                for effect in w.step(e, 1_000_000) {
                    match effect {
                        Effect::Scan { req } => queue.push(Event::Windows {
                            req,
                            result: Ok((self.windows.clone(), self.foreground)),
                        }),
                        Effect::Focus { req, win } => {
                            if self.focus_ok {
                                // Como Windows: la enfocada pasa arriba del orden Z.
                                if let Some(i) = self.windows.iter().position(|w| w.key == win) {
                                    let w = self.windows.remove(i);
                                    self.windows.insert(0, w);
                                }
                                self.foreground = Some(win);
                                self.focused.push(win);
                            }
                            let report = FocusReport {
                                ok: self.focus_ok,
                                neutral_key: false,
                                elapsed_us: 1500,
                            };
                            queue.push(Event::Focused { req, result: Ok(report) });
                        }
                        Effect::Launch { req, path } => {
                            self.launched.push(path);
                            queue.push(Event::Launched { req, result: Ok(()) });
                        }
                        Effect::Log(line) => self.logs.push(line),
                        Effect::Quit => self.quit = true,
                    }
                }
            }
        }
    }

    fn letter(l: u8) -> Event {
        Event::Action {
            action: HyperAction::Letter { letter: l, thumb: false },
            held: 0,
            age_ms: 2,
        }
    }

    #[test]
    fn letter_scans_then_focuses_the_app() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["code", "brave", "slack"]);
        let brave = sim.windows[1].key;
        sim.run(&mut w, letter(b'B'));
        assert_eq!(sim.focused, vec![brave]);
        assert_eq!(w.activations(), Activations { total: 1, ok: 1, neutral_key: 0 });
        assert!(sim.logs[0].starts_with("Caps + B → brave «brave»: ok"), "{:?}", sim.logs);
        assert_eq!(w.in_flight(), 0, "no quedan pedidos colgados");
    }

    #[test]
    fn repeating_the_letter_cycles_the_windows_of_the_app() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["brave", "code", "brave"]);
        let (first, second) = (sim.windows[0].key, sim.windows[2].key);
        sim.run(&mut w, letter(b'B')); // la primera ya está al frente → la otra
        sim.run(&mut w, letter(b'B')); // y vuelve
        assert_eq!(sim.focused, vec![second, first]);
    }

    #[test]
    fn bound_but_closed_app_is_launched() {
        let (b, _) = parse_bindings("n = notion | C:\\Notion.exe");
        let mut w = World::new(b);
        let mut sim = Sim::new(&["code"]);
        sim.run(&mut w, letter(b'N'));
        assert_eq!(sim.launched, vec!["C:\\Notion.exe".to_owned()]);
        assert!(sim.logs[0].contains("abriendo notion"));
    }

    #[test]
    fn unknown_letter_only_logs() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["code"]);
        sim.run(&mut w, letter(b'Q'));
        assert!(sim.focused.is_empty());
        assert_eq!(sim.logs, vec!["Caps + Q: no hay ninguna app abierta con la Q.".to_owned()]);
    }

    #[test]
    fn late_actions_are_discarded() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["brave"]);
        let late = Event::Action {
            action: HyperAction::Letter { letter: b'B', thumb: false },
            held: 0,
            age_ms: MAX_ACTION_AGE_MS + 1,
        };
        sim.run(&mut w, late);
        assert!(sim.focused.is_empty());
        assert!(sim.logs[0].starts_with("Acción descartada"));
    }

    #[test]
    fn failed_focus_is_counted() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&["code", "brave"]);
        sim.focus_ok = false;
        sim.run(&mut w, letter(b'B'));
        assert_eq!(w.activations(), Activations { total: 1, ok: 0, neutral_key: 0 });
        assert!(sim.logs[0].contains("FALLÓ"));
    }

    #[test]
    fn answers_to_unknown_requests_are_ignored() {
        let mut w = World::new(Vec::new());
        let stray = Event::Focused { req: 99, result: Err("x".into()) };
        assert!(w.step(stray, 0).is_empty());
    }

    #[test]
    fn escape_quits_and_layers_are_tracked() {
        let mut w = World::new(Vec::new());
        let mut sim = Sim::new(&[]);
        sim.run(&mut w, Event::Layer(Layer::Window));
        assert_eq!(w.layer(), Layer::Window);
        sim.run(&mut w, Event::KeyboardReset);
        assert_eq!(w.layer(), Layer::Off);
        sim.run(&mut w, Event::Action { action: HyperAction::Escape, held: 0, age_ms: 0 });
        assert!(sim.quit);
    }

    #[test]
    fn latency_is_measured_from_the_key() {
        let mut w = World::new(Vec::new());
        let effects = w.step(letter(b'B'), 10_000); // tecla 2 ms antes: 8000 µs
        let Effect::Scan { req } = effects[0] else { panic!("{effects:?}") };
        let win = win(1, "brave");
        let effects = w.step(Event::Windows { req, result: Ok((vec![win.clone()], None)) }, 11_000);
        let Effect::Focus { req, .. } = effects[0] else { panic!("{effects:?}") };
        let report = FocusReport { ok: true, neutral_key: false, elapsed_us: 500 };
        let effects = w.step(Event::Focused { req, result: Ok(report) }, 13_000);
        let Effect::Log(line) = &effects[0] else { panic!("{effects:?}") };
        assert!(line.contains("desde la tecla 5.0 ms"), "{line}");
    }
}
