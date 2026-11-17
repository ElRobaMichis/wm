//! Hito 4d (plan-hito4d.md): cambios de pantallas, reposo y bloqueo.
//!
//! Los avisos de Windows alimentan un [`StormTracker`]; tras la calma se
//! observa una sola vez (`Survey`) y se rescata **solo** lo que quedó
//! inalcanzable (`screens::rescue_plan`). Windows mueve; nosotros no
//! recordamos destinos ni peleamos con su re-acomodo.

use std::collections::{HashMap, HashSet};

use super::{Effect, NoFly, Obs, Pending, Req, Spot, WinInfo, WinKey, World, index_of};
use crate::Rect;
use crate::place::{MonitorObs, centered, edge_error, outer_for, placement_name};
use crate::screens::{
    ScreenEvent, Seen, StormAction, StormTracker, Verdict, rescue_plan, topology_key,
};
use crate::{Placement, place};

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
    /// Escondidas que rescatamos, para devolverlas cuando vuelva su
    /// topología (ajuste del 4d).
    homes: HashMap<WinKey, Home>,
    /// La topología de antes, si esta tormenta la cambió (0 si no): las
    /// casas que se guardan ahora son de ella.
    home_key: u64,
}

/// De dónde sacamos una ventana al rescatarla.
#[derive(Debug, Clone, Copy)]
struct Home {
    /// Topología en la que estaba ahí.
    key: u64,
    /// Esquina exterior antes del rescate.
    from: (i32, i32),
    /// Marco justo después del rescate: si cambió (la movieron o le
    /// cambiaron el tamaño, aunque sea con la misma esquina), no vuelve.
    after: Rect,
    /// Marco de antes (para saber si vuelve a ser alcanzable).
    frame: Rect,
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
            // Un reencaje (o una tecla) a medio camino: la memoria de la
            // ubicación queda para reencajar tras la próxima calma.
            if j.spot.placement != Placement::Full {
                self.placed.insert(win, (j.spot, j.target));
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
        if !late {
            self.screens.home_key = if changed { self.screens.last_key } else { 0 };
        }
        let mut old_placed = HashMap::new();
        if changed {
            // Memorias de "repetir" (plan-core §3.3): con otros monitores
            // valen como primera vez. Lo que pusimos se reencaja abajo.
            old_placed = std::mem::take(&mut self.placed);
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
        let mut moves = Vec::new();
        if changed && !late {
            self.resnap(old_placed, &survey, dragging, false, now_us, out);
            self.return_home(key, &survey, &mut moves, out);
        } else if late {
            // Windows restaura algunas ventanas (minimizadas al desconectar)
            // después de la calma: otra oportunidad de reencajarlas.
            let mut placed = std::mem::take(&mut self.placed);
            placed.extend(old_placed);
            self.resnap(placed, &survey, dragging, true, now_us, out);
        }
        self.adopt_slots(&survey, dragging);
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
                    if self.screens.home_key != 0 {
                        let home = Home {
                            key: self.screens.home_key,
                            from: (w.outer.left, w.outer.top),
                            after: shift(w.frame, dx, dy),
                            frame: w.frame,
                        };
                        self.screens.homes.insert(win, home);
                    }
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

    /// Tras un cambio de topología: lo que pusimos en una mitad o esquina y
    /// sigue ahí (aunque Windows lo haya reescalado) vuelve a quedar exacto
    /// en la misma ubicación del mismo monitor. Una vez, sin Z ni vuelo.
    fn resnap(
        &mut self,
        old: HashMap<WinKey, (Spot, Rect)>,
        survey: &Survey,
        dragging: Option<WinKey>,
        late: bool,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let mut names = Vec::new();
        // Lo que todavía no se puede decidir (minimizada, su monitor no está,
        // algo en curso) queda para el próximo cambio: así una ventana que
        // Windows minimiza al desconectar y restaura al reconectar vuelve
        // exacta a su mitad.
        let mut keep = Vec::new();
        for (win, (spot, achieved)) in old {
            let Some(w) = survey.wins.iter().find(|w| w.info.key == win) else { continue };
            if spot.placement == Placement::Full
                || edge_error(achieved, place(spot.monitor.work, spot.placement)) > 1
            {
                // No era un encaje exacto (G/V, tamaño mínimo): con otra
                // topología se olvida; en el control tardío se conserva.
                if late {
                    keep.push((win, (spot, achieved)));
                }
                continue;
            }
            let Some(mi) = index_of(&survey.monitors, spot.monitor) else {
                keep.push((win, (spot, achieved)));
                continue;
            };
            let m = survey.monitors[mi];
            if w.info.minimized || w.parked || self.jobs.contains_key(&win) || dragging == Some(win)
            {
                keep.push((win, (spot, achieved)));
                continue;
            }
            if w.maximized || w.frame == m.bounds || !self.desks.in_scope(&w.info.app) {
                continue;
            }
            let slot = place(m.work, spot.placement);
            if !crate::screens::still_there(
                achieved,
                w.frame,
                spot.monitor.dpi,
                m.dpi,
                m.work,
                slot,
            ) {
                // Para afinar la regla con apps reales: qué se vio.
                let r = |r: Rect| format!("({}, {})–({}, {})", r.left, r.top, r.right, r.bottom);
                out.push(Effect::Log(format!(
                    "Pantallas: no reencajo {} ({}): está en {}, había quedado en {} (escala {} → {}).",
                    w.info.app,
                    placement_name(spot.placement),
                    r(w.frame),
                    r(achieved),
                    spot.monitor.dpi,
                    m.dpi
                )));
                continue;
            }
            let gap = edge_error(w.frame, slot);
            let obs = Obs {
                key: win,
                app: w.info.app.clone(),
                title: w.info.title.clone(),
                frame: w.frame,
                outer: w.outer,
                dpi: m.dpi,
                maximized: false,
                minimized: false,
                monitors: survey.monitors.clone(),
                no_fly: Some(NoFly::NotFront),
            };
            let what = format!(
                "Pantallas → {} ({}, reencaje)",
                placement_name(spot.placement),
                w.info.app
            );
            self.record = None;
            self.exact = true;
            self.go(
                &obs,
                Spot { monitor: m, placement: spot.placement },
                slot,
                what,
                now_us,
                now_us,
                out,
            );
            if gap > 0 {
                names.push(format!(
                    "{} ({}, estaba a {gap} px)",
                    w.info.app,
                    placement_name(spot.placement)
                ));
            }
        }
        for (win, v) in keep {
            self.placed.entry(win).or_insert(v);
        }
        if !names.is_empty() {
            out.push(Effect::Log(format!(
                "Pantallas: {} reencajadas: {}.",
                names.len(),
                names.join(", ")
            )));
        }
    }

    /// Las ventanas que están exactas en una mitad o esquina cuentan como
    /// puestas ahí (también las de antes de arrancar `wm`, o las que se
    /// pusieron a mano): un cambio de escala las reencaja igual.
    fn adopt_slots(&mut self, survey: &Survey, dragging: Option<WinKey>) {
        const SLOTS: [Placement; 8] = [
            Placement::LeftHalf,
            Placement::RightHalf,
            Placement::TopHalf,
            Placement::BottomHalf,
            Placement::TopLeft,
            Placement::TopRight,
            Placement::BottomLeft,
            Placement::BottomRight,
        ];
        for w in &survey.wins {
            let win = w.info.key;
            if w.info.minimized
                || w.maximized
                || w.parked
                || self.placed.contains_key(&win)
                || self.jobs.contains_key(&win)
                || dragging == Some(win)
                || !self.desks.in_scope(&w.info.app)
            {
                continue;
            }
            let Some(mi) = crate::place::monitor_of(&survey.monitors, w.frame) else { continue };
            let monitor = survey.monitors[mi];
            if let Some(&placement) = SLOTS.iter().find(|&&p| place(monitor.work, p) == w.frame) {
                self.placed.insert(win, (Spot { monitor, placement }, w.frame));
            }
        }
    }

    /// Volvió la topología de la que sacamos una ventana al rescatarla: si
    /// sigue donde la dejamos, vuelve a su lugar (Windows lo hace con las
    /// visibles; no con las escondidas en otro escritorio).
    fn return_home(
        &mut self,
        key: u64,
        survey: &Survey,
        moves: &mut Vec<(WinKey, i32, i32)>,
        out: &mut Vec<Effect>,
    ) {
        let areas: Vec<Rect> = survey.monitors.iter().map(|m| m.work).collect();
        let min = crate::screens::min_visible(&survey.monitors);
        let mut back = Vec::new();
        let homes = std::mem::take(&mut self.screens.homes);
        for (win, home) in homes {
            let Some(w) = survey.wins.iter().find(|w| w.info.key == win) else { continue };
            // Nadie la movió desde el rescate; si no, la casa se olvida.
            if edge_error(w.frame, home.after) > 1 {
                continue;
            }
            let ok = home.key == key
                && !w.info.minimized
                && !w.maximized
                && !w.parked
                && !self.jobs.contains_key(&win)
                && crate::screens::reachable(home.frame, &areas, min);
            if ok {
                moves.push((win, home.from.0, home.from.1));
                back.push(w.info.app.clone());
            } else {
                self.screens.homes.insert(win, home);
            }
        }
        if !back.is_empty() {
            out.push(Effect::Log(format!(
                "Pantallas: {} devuelta(s) a su monitor: {}.",
                back.len(),
                back.join(", ")
            )));
        }
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
