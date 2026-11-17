//! Hito 4d (plan-hito4d.md): cambios de pantallas, reposo y bloqueo.
//!
//! Los avisos de Windows alimentan un [`StormTracker`]; tras la calma se
//! observa una sola vez (`Survey`) y se rescata **solo** lo que quedó
//! inalcanzable (`screens::rescue_plan`). Windows mueve; nosotros no
//! recordamos destinos ni peleamos con su re-acomodo.

use std::collections::HashSet;

use super::{Effect, Pending, Req, WinInfo, WinKey, World};
use crate::Rect;
use crate::place::{MonitorObs, centered, outer_for};
use crate::screens::{
    ScreenEvent, Seen, StormAction, StormTracker, Verdict, rescue_plan, topology_key,
};

/// Una ventana como la ve un `Survey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurveyWin {
    pub info: WinInfo,
    /// Marco visible (DWM).
    pub frame: Rect,
    /// Rectángulo de la ventana (con los bordes invisibles).
    pub outer: Rect,
    pub maximized: bool,
    /// Escondida por nosotros fuera de pantalla u oculta: no se rescata.
    pub parked: bool,
}

/// Monitores y ventanas en una sola observación.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Survey {
    pub monitors: Vec<MonitorObs>,
    pub wins: Vec<SurveyWin>,
}

/// Para qué se pidió la observación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    /// Dentro de la tormenta (instantáneas que tienen que coincidir).
    Snap,
    /// Control tardío, unos segundos después de reconciliar.
    Recheck,
}

/// Totales para el resumen al salir.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScreenStats {
    /// Tormentas reconciliadas.
    pub storms: u32,
    /// Ventanas rescatadas.
    pub rescued: u32,
}

#[derive(Debug, Default)]
pub(crate) struct Screens {
    storm: StormTracker,
    locked: bool,
    asleep: bool,
    away: bool,
    asked: Option<(Req, Ask)>,
    /// Topología de la última reconciliación (0 = ninguna todavía).
    last_key: u64,
    /// Ya hubo una reconciliación (la del arranque no avisa a la ui).
    booted: bool,
    /// Rescatadas en esta tormenta (una vez por ventana).
    rescued: HashSet<WinKey>,
    /// Se volvieron a ir después del rescate: no se insiste.
    stubborn: HashSet<WinKey>,
    /// Qué avisos llegaron en esta tormenta (para el log).
    hints: Vec<&'static str>,
    since_us: u64,
    /// Controles tardíos seguidos sin poder leer las pantallas.
    retries: u8,
    /// Un aviso fuerte (cambió la configuración, reposo) en esta tormenta:
    /// sin animaciones hasta la calma. El arranque o el área de trabajo no.
    frozen: bool,
    stats: ScreenStats,
}

impl Screens {
    /// Sesión bloqueada, en reposo o fuera de la consola: no se lee ni se
    /// mueve nada, sin timers.
    pub(crate) fn blocked(&self) -> bool {
        self.locked || self.asleep || self.away
    }

    /// Una tormenta con un aviso fuerte en curso (sin animaciones).
    pub(crate) fn frozen(&self) -> bool {
        self.frozen && self.storm.active()
    }

    pub(crate) fn deadline_us(&self) -> Option<u64> {
        if self.blocked() {
            return None;
        }
        self.storm.deadline().map(|ms| ms.saturating_mul(1000))
    }
}

fn shift(r: Rect, dx: i32, dy: i32) -> Rect {
    Rect::new(r.left + dx, r.top + dy, r.right + dx, r.bottom + dy)
}

impl World {
    /// Totales de pantallas para el resumen.
    pub fn screen_stats(&self) -> ScreenStats {
        self.screens.stats
    }

    /// Hay una tormenta con un cambio de pantallas real: sin animaciones.
    pub fn screens_frozen(&self) -> bool {
        self.screens.frozen()
    }

    pub(crate) fn on_screen(&mut self, e: ScreenEvent, now_us: u64, out: &mut Vec<Effect>) {
        match e {
            ScreenEvent::Changed => {
                // Primero el aviso (abre la tormenta), después congelar.
                self.screen_hint(e.name(), now_us, out);
                self.freeze(out);
            }
            ScreenEvent::WorkArea
            | ScreenEvent::Resume
            | ScreenEvent::Unlock
            | ScreenEvent::Connect => {
                match e {
                    ScreenEvent::Resume => self.screens.asleep = false,
                    ScreenEvent::Unlock => self.screens.locked = false,
                    ScreenEvent::Connect => self.screens.away = false,
                    _ => {}
                }
                self.screen_hint(e.name(), now_us, out);
            }
            ScreenEvent::Suspend => {
                self.freeze(out);
                self.screens.asleep = true;
            }
            ScreenEvent::Lock => {
                if !self.screens.locked {
                    out.push(Effect::Log(
                        "Pantallas: sesión bloqueada; reviso al desbloquear.".into(),
                    ));
                }
                self.screens.locked = true;
            }
            ScreenEvent::Disconnect => self.screens.away = true,
        }
        let blocked = self.screens.blocked();
        self.screens.storm.lock(blocked);
    }

    /// Arranque = recuperación: una pasada aunque Windows no avise nada.
    pub(crate) fn screens_boot(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        self.screen_hint("arranque", now_us, out);
    }

    /// "Mostrar todas las ventanas": revisar ya, también las que se habían
    /// dado por perdidas.
    pub(crate) fn screens_show_all(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        self.screens.stubborn.clear();
        self.screens.rescued.clear();
        self.screen_hint("Mostrar todas", now_us, out);
    }

    /// Una tecla con `Caps` prueba que la sesión está desbloqueada: cubre un
    /// desbloqueo perdido.
    pub(crate) fn screens_unblock(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        if !self.screens.blocked() {
            return;
        }
        self.screens.locked = false;
        self.screens.asleep = false;
        self.screens.away = false;
        self.screens.storm.lock(false);
        self.screen_hint("tecla", now_us, out);
    }

    fn screen_hint(&mut self, name: &'static str, now_us: u64, out: &mut Vec<Effect>) {
        let s = &mut self.screens;
        if !s.storm.active() {
            s.frozen = false;
            s.since_us = now_us;
            s.hints.clear();
            s.rescued.clear();
            if name != "arranque" {
                out.push(Effect::Log(format!(
                    "Pantallas: cambio detectado ({name}); espero a que se calme."
                )));
            }
        }
        s.hints.push(name);
        s.storm.hint(now_us / 1000);
        // Una observación en curso es de antes del aviso: se descarta.
        if let Some((req, _)) = s.asked.take() {
            self.pending.remove(&req);
        }
    }

    /// Un aviso fuerte (cambió la configuración, reposo): los vuelos
    /// aterrizan y se olvidan las colocaciones en curso (sus destinos son de
    /// la topología vieja); el mapa se oculta. El arrastre, el aviso y el
    /// borde de foco siguen.
    fn freeze(&mut self, out: &mut Vec<Effect>) {
        self.screens.frozen = true;
        for (win, j) in self.jobs.drain() {
            if j.flying {
                out.push(Effect::Land { win });
            }
        }
        self.deferred = None;
        self.gesture = None;
        self.keys = None;
        let mut m = Vec::new();
        self.map.reset(&mut m);
        self.map_effects(m, out);
    }

    pub(crate) fn screens_tick(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        if self.screens.blocked() || self.screens.asked.is_some() {
            return;
        }
        let ms = now_us / 1000;
        if self.screens.storm.deadline().is_none_or(|d| ms < d) {
            return;
        }
        let in_storm = self.screens.storm.active();
        if self.screens.storm.tick(ms) == Some(StormAction::Snapshot) {
            let ask = if in_storm { Ask::Snap } else { Ask::Recheck };
            let req = self.req();
            self.pending.insert(req, Pending::Survey);
            self.screens.asked = Some((req, ask));
            out.push(Effect::Survey { req });
        }
    }

    pub(crate) fn on_surveyed(
        &mut self,
        req: Req,
        result: Result<Survey, String>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some((asked, ask)) = self.screens.asked else { return };
        if asked != req {
            return;
        }
        self.screens.asked = None;
        let key = result.as_ref().map_or(0, |s| topology_key(&s.monitors));
        let ms = now_us / 1000;
        match ask {
            Ask::Snap => {
                // La primera instantánea espera la segunda; si no coinciden,
                // se espera otra calma.
                if let Some(StormAction::Reconcile(k)) = self.screens.storm.snapshot(ms, key) {
                    self.reconcile(result, k, false, now_us, out);
                }
            }
            Ask::Recheck => {
                if key == 0 {
                    self.screens.retries += 1;
                    if self.screens.retries < 3 {
                        self.screen_hint("control tardío", now_us, out);
                    } else {
                        out.push(Effect::Log(
                            "Pantallas: no pude leer las pantallas en el control tardío; lo dejo."
                                .into(),
                        ));
                    }
                } else if key != self.screens.last_key {
                    self.screens.retries = 0;
                    self.screen_hint("control tardío", now_us, out);
                } else {
                    self.screens.retries = 0;
                    self.reconcile(result, key, true, now_us, out);
                }
            }
        }
    }

    /// Tras la calma: rescatar lo inalcanzable (plan-hito4d §3.3). `late`: el
    /// control tardío (solo lo que no se rescató todavía).
    fn reconcile(
        &mut self,
        result: Result<Survey, String>,
        key: u64,
        late: bool,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if self.screens.blocked() {
            return;
        }
        let survey = match result {
            Ok(s) if key != 0 => s,
            Ok(_) | Err(_) => {
                let why = match &result {
                    Err(e) => e.clone(),
                    Ok(_) => "configuración incoherente".into(),
                };
                out.push(Effect::Log(format!(
                    "Pantallas: no pude leer las pantallas ({why}); no muevo nada."
                )));
                return;
            }
        };
        let changed = key != self.screens.last_key;
        if changed {
            // Memorias de "repetir" (plan-core §3.3): con otros monitores
            // valen como primera vez.
            self.placed.clear();
            self.centers.clear();
            self.sizing.clear();
            self.limits.clear();
            self.gesture = None;
            self.deferred = None;
            self.screens.stubborn.clear();
            if self.screens.booted {
                out.push(Effect::ScreensChanged);
            }
            self.screens.last_key = key;
        }
        let dragging = self.dragging_win();
        let seen: Vec<Seen> = survey
            .wins
            .iter()
            .map(|w| Seen {
                frame: w.frame,
                minimized: w.info.minimized,
                maximized: w.maximized,
                skip: w.parked
                    || self.jobs.contains_key(&w.info.key)
                    || dragging == Some(w.info.key)
                    || self.screens.stubborn.contains(&w.info.key)
                    || !self.desks.in_scope(&w.info.app),
            })
            .collect();
        let plan = rescue_plan(&survey.monitors, &seen);
        let mut moves = Vec::new();
        let mut names = Vec::new();
        let mut gave_up = Vec::new();
        for (i, verdict) in plan {
            let w = &survey.wins[i];
            let win = w.info.key;
            if late && self.screens.rescued.contains(&win) {
                self.screens.stubborn.insert(win);
                gave_up.push(w.info.app.clone());
                continue;
            }
            self.screens.rescued.insert(win);
            match verdict {
                Verdict::Move { dx, dy } => {
                    let to = shift(w.outer, dx, dy);
                    moves.push((win, to.left, to.top));
                    names.push(format!("{} ({dx:+}, {dy:+})", w.info.app));
                }
                Verdict::Remaximize { on } => {
                    let m = survey.monitors[on];
                    let target = centered(m.work, w.frame.width(), w.frame.height());
                    let rect = outer_for(w.frame, w.outer, target, m.dpi, m.dpi);
                    out.push(Effect::Normalize { win });
                    let req = self.req();
                    out.push(Effect::Place { req, win, rect });
                    out.push(Effect::Maximize { win });
                    names.push(format!("{} (maximizada)", w.info.app));
                }
            }
        }
        let n = names.len();
        self.screens.stats.rescued += n as u32;
        if !late {
            self.screens.stats.storms += 1;
        }
        if !moves.is_empty() {
            let req = self.req();
            self.pending.insert(req, Pending::Rescue);
            out.push(Effect::Rescue { req, moves });
        }
        for app in &gave_up {
            out.push(Effect::Log(format!(
                "Pantallas: {app} sigue fuera de pantalla después del rescate; no insisto."
            )));
        }
        if !late || n > 0 {
            let secs = now_us.saturating_sub(self.screens.since_us) as f64 / 1_000_000.0;
            let what = if late { "control tardío" } else { "en calma" };
            let hints = summarize(&self.screens.hints);
            let mut line = format!(
                "Pantallas: {what} tras {secs:.1} s ({hints}): {} monitores; {} ventanas revisadas, {n} fuera de pantalla",
                survey.monitors.len(),
                survey.wins.len(),
            );
            if n > 0 {
                line.push_str(&format!(
                    ": {}. Ventanas traídas a la pantalla: {n}.",
                    names.join(", ")
                ));
            } else {
                line.push('.');
            }
            out.push(Effect::Log(line));
        }
        if !late && self.screens.booted {
            // R3: un cambio de pantallas puede destapar ventanas.
            self.start_sync(crate::desks::Goal::Verify, now_us, out);
        }
        self.screens.booted = true;
    }

    pub(crate) fn on_rescued(&mut self, failed: Vec<(WinKey, String)>, out: &mut Vec<Effect>) {
        for (win, e) in failed {
            out.push(Effect::Log(format!(
                "Pantallas: no se pudo traer la ventana {:#x}: {e}",
                win.hwnd
            )));
        }
    }
}

/// "pantallas ×3, área de trabajo".
fn summarize(hints: &[&str]) -> String {
    let mut out: Vec<(&str, usize)> = Vec::new();
    for h in hints {
        match out.iter_mut().find(|(n, _)| n == h) {
            Some(slot) => slot.1 += 1,
            None => out.push((h, 1)),
        }
    }
    out.iter()
        .map(|(n, c)| if *c > 1 { format!("{n} ×{c}") } else { (*n).to_owned() })
        .collect::<Vec<_>>()
        .join(", ")
}
