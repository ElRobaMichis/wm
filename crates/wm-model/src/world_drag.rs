//! `Caps` + arrastrar con el mouse (hito 5, `plan-hito5.md` §5b-5c): el
//! modelo decide qué ventana y si se puede; el carril `wm-drag` la sigue.
//! El arrastre entero entra al diario (Z lo deshace).

use super::*;
use crate::drag::{Mode, corner_of};
use crate::mouse::Button;

/// Nada que tenga que ver con el mouse dura más de esto (si se perdió el
/// "arriba" del botón): se reinicia el teclado, como el tope del mapa.
const POINTER_MAX_US: u64 = 60_000_000;

#[derive(Debug, Clone, Copy)]
pub(super) struct Pointer {
    button: Button,
    /// Dónde se apretó el botón.
    at: (i32, i32),
    since_us: u64,
    phase: Phase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Esperando qué ventana hay bajo el cursor.
    Grabbing(Req),
    /// Arrastrando esta ventana.
    Dragging(WinKey),
    /// Había ventanas en vuelo: se pidió aterrizarlas y, cuando terminen, se
    /// mira qué hay bajo el cursor. Durante un vuelo la real tiene cloak (y
    /// `WindowFromPoint` devuelve la de atrás) o está en otro rectángulo.
    WaitLanding,
    /// No se pudo (o nada que hacer): se espera el "arriba".
    Idle,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Dragging {
    win: WinKey,
    before: Entry,
    app_resize: bool,
}

fn verb(button: Button) -> &'static str {
    match button {
        Button::Left => "Caps + arrastrar",
        Button::Right => "Caps + clic derecho",
    }
}

impl World {
    pub(super) fn pointer_deadline(&self) -> Option<u64> {
        self.pointer.map(|p| p.since_us + POINTER_MAX_US)
    }

    /// Se perdió el "arriba": cortar y reiniciar el teclado (y con él, la
    /// máquina del mouse del hook).
    pub(super) fn pointer_tick(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        if self.pointer_deadline().is_some_and(|t| t <= now_us) {
            self.pointer_reset(out);
            out.push(Effect::ResetKeyboard);
        }
    }

    /// Reinicio del teclado: el arrastre termina donde está.
    pub(super) fn pointer_reset(&mut self, out: &mut Vec<Effect>) {
        if let Some(p) = self.pointer.take()
            && let Phase::Dragging(win) = p.phase
        {
            out.push(Effect::DragStop { win });
        }
    }

    pub(super) fn on_pointer(
        &mut self,
        button: Button,
        down: bool,
        at: (i32, i32),
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if !down {
            if let Some(p) = self.pointer.take()
                && let Phase::Dragging(win) = p.phase
            {
                out.push(Effect::DragStop { win });
            }
            return;
        }
        if self.pointer.is_some() || self.paused {
            out.push(Effect::Log(format!("{}: se ignora (otro botón en curso).", verb(button))));
            return;
        }
        // El mapa taparía la ventana: un clic lo suprime o lo cierra.
        let mut m = Vec::new();
        if self.map.visible() {
            self.map.reset(&mut m);
        } else {
            self.map.on_action(false, now_us, &mut m);
        }
        self.map_effects(m, out);
        self.keys = None;
        if self.sync.is_some() || self.dragging.is_some() {
            // Cambiando de escritorio, o el arrastre anterior todavía termina.
            let why = if self.sync.is_some() {
                "cambiando de escritorio"
            } else {
                "el arrastre anterior todavía no terminó"
            };
            out.push(Effect::Log(format!("{}: se ignora ({why}).", verb(button))));
            self.pointer = Some(Pointer { button, at, since_us: now_us, phase: Phase::Idle });
            return;
        }
        let flying: Vec<WinKey> =
            self.jobs.iter().filter(|(_, j)| j.flying).map(|(w, _)| *w).collect();
        if !flying.is_empty() {
            for w in flying {
                self.ground(w, out);
            }
            self.pointer =
                Some(Pointer { button, at, since_us: now_us, phase: Phase::WaitLanding });
            return;
        }
        let req = self.req();
        self.pending.insert(req, Pending::Grab);
        self.pointer = Some(Pointer { button, at, since_us: now_us, phase: Phase::Grabbing(req) });
        out.push(Effect::InspectAt { req, x: at.0, y: at.1 });
    }

    pub(super) fn on_grabbed(
        &mut self,
        req: Req,
        result: Result<Grab, String>,
        out: &mut Vec<Effect>,
    ) {
        let Some(p) = self.pointer.filter(|p| p.phase == Phase::Grabbing(req)) else {
            return; // ya soltó el botón
        };
        let what = verb(p.button);
        let idle = |w: &mut World| {
            if let Some(p) = w.pointer.as_mut() {
                p.phase = Phase::Idle;
            }
        };
        let g = match result {
            Ok(g) => g,
            Err(e) => {
                out.push(Effect::Log(format!("{what}: {e}.")));
                return idle(self);
            }
        };
        let obs = g.obs;
        if is_fullscreen(&obs) {
            out.push(Effect::Log(format!("{what}: {} está en pantalla completa.", obs.app)));
            return idle(self);
        }
        let mode = match p.button {
            Button::Left => Mode::Move,
            Button::Right if obs.maximized || !g.resizable => {
                out.push(Effect::Log(format!(
                    "{what}: el tamaño de {} no se puede cambiar{}.",
                    obs.app,
                    if obs.maximized { " (está maximizada)" } else { "" }
                )));
                return idle(self);
            }
            Button::Right => corner_of(obs.frame, p.at),
        };
        let win = obs.key;
        // Lo que estaba en curso con esta ventana ya no vale.
        self.ground(win, out);
        self.jobs.remove(&win);
        self.placed.remove(&win);
        self.centers.remove(&win);
        self.sizing.remove(&win);
        let tag = self.fresh_tag();
        let before = Entry { win, frame: obs.frame, maximized: obs.maximized, tag, desk: None };
        self.dragging =
            Some(Dragging { win, before, app_resize: matches!(mode, Mode::Resize { .. }) });
        if let Some(p) = self.pointer.as_mut() {
            p.phase = Phase::Dragging(win);
        }
        out.push(Effect::Raise { win });
        let restore = if mode == Mode::Move { g.restore } else { None };
        out.push(Effect::DragStart { win, mode, grab: p.at, restore });
    }

    /// Terminó (o se aterrizó) un vuelo: si el clic esperaba eso y no queda
    /// ninguno, mirar qué hay bajo el cursor.
    pub(super) fn drag_after_landing(&mut self, out: &mut Vec<Effect>) {
        let Some(p) = self.pointer.filter(|p| p.phase == Phase::WaitLanding) else {
            return;
        };
        if self.pending.values().any(|v| matches!(v, Pending::FlyCmd { .. })) {
            return;
        }
        let req = self.req();
        self.pending.insert(req, Pending::Grab);
        if let Some(pp) = self.pointer.as_mut() {
            pp.phase = Phase::Grabbing(req);
        }
        out.push(Effect::InspectAt { req, x: p.at.0, y: p.at.1 });
    }

    pub(super) fn on_drag_done(
        &mut self,
        win: WinKey,
        result: Result<DragReport, String>,
        out: &mut Vec<Effect>,
    ) {
        let Some(d) = self.dragging.filter(|d| d.win == win) else { return };
        self.dragging = None;
        let what = if d.app_resize { "Caps + clic derecho" } else { "Caps + arrastrar" };
        match result {
            Ok(r) if r.moved => {
                self.remember(d.before);
                let line = match (d.app_resize, r.frame) {
                    (true, Some(f)) => {
                        format!("{what} → tamaño {}×{} en {} ms", f.width(), f.height(), r.ms)
                    }
                    _ => format!("{what} → movida en {} ms", r.ms),
                };
                out.push(Effect::Log(line));
            }
            Ok(_) => {} // un clic quieto: nada que anotar
            Err(e) => out.push(Effect::Log(format!("{what}: {e}."))),
        }
    }
}
