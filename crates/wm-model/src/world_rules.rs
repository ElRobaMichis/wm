//! v1.1 (plan-v1.1.md): la regla "al abrir" de cada app. Se aplica una sola
//! vez, a la primera ventana principal de una app con regla, cuando la app
//! termina de ubicarla (dos observaciones iguales); si la app se reubica
//! después, gana la app. Sin animación; Z la devuelve adonde la abrió la app.

use super::{Effect, Entry, NoFly, Obs, Pending, Spot, WinInfo, WinKey, World, is_fullscreen};
use crate::place::{centered_percent, monitor_of, placement_name};
use crate::rules::OpenRule;
use crate::{Placement, Rect, place};

/// Cuándo mirar la ventana nueva (desde que se la vio).
const ARRIVE_MS: [u64; 4] = [100, 300, 700, 1500];

/// Cuánto puede tardar en pasar al frente una ventana nueva que apareció
/// detrás (la de IntelliJ, tras su pantalla de carga).
const BEHIND_MS: u64 = 5_000;

/// Cuándo volver a buscar una ventana que pasó al frente y el `Scan` no vio
/// (desde el `FOREGROUND`).
const UNSEEN_MS: [u64; 6] = [250, 600, 1200, 2500, 5000, 10_000];

/// Una ventana al frente que todavía no aparece en la lista.
#[derive(Debug, Clone, Copy)]
pub(super) struct Unseen {
    win: WinKey,
    since_us: u64,
    /// Próxima búsqueda (índice en `UNSEEN_MS`).
    step: usize,
}

/// Una ventana nueva esperando su regla.
#[derive(Debug, Clone)]
pub(super) struct Arrival {
    rule: OpenRule,
    since_us: u64,
    /// Próximo chequeo (índice en `ARRIVE_MS`).
    step: usize,
    inspecting: bool,
    /// Marco y maximizada de la observación anterior.
    last: Option<(Rect, bool)>,
}

impl World {
    /// Ventanas que el modelo no conocía (antes de adoptarlas): si su app
    /// tiene regla "al abrir", empieza a seguirlas. No se llama en el
    /// arranque (las que ya estaban abiertas no se tocan nunca).
    pub(super) fn greet(
        &mut self,
        windows: &[WinInfo],
        fg: Option<WinKey>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if let Some(u) = self.unseen
            && windows.iter().any(|w| w.key == u.win)
        {
            self.unseen = None;
            if u.step > 0 {
                out.push(Effect::Log(format!(
                    "Ventana nueva: apareció en la lista {} ms después de pasar al frente.",
                    now_us.saturating_sub(u.since_us) / 1000
                )));
            }
        }
        if self.paused {
            return;
        }
        for w in windows {
            if self.desks.desk_of(w.key).is_some()
                || self.arrivals.contains_key(&w.key)
                || self.behind.contains_key(&w.key)
            {
                continue;
            }
            let id = w.app_id();
            let Some(b) = self.bindings.iter().find(|b| b.open.is_some() && b.matches(&id)) else {
                continue;
            };
            let rule = b.open.expect("filtrada");
            let others = windows.iter().any(|o| o.key != w.key && o.main && b.matches(&o.app_id()));
            let why = if !w.main {
                Some("no es la ventana principal (pantalla de carga o diálogo)")
            } else if others {
                Some("la app ya tenía otra ventana")
            } else if w.minimized || w.concealed {
                Some("está minimizada o escondida")
            } else if self.screens_frozen() {
                Some("están cambiando las pantallas")
            } else {
                None
            };
            let app = b.display();
            let title = super::shorten(&w.title, 40);
            match why {
                Some(why) => out.push(Effect::Log(format!(
                    "Regla al abrir ({app}): «{title}» no se toca: {why}."
                ))),
                // Apareció detrás: la regla corre solo si pasa al frente
                // pronto; una que se abre de verdad en segundo plano no.
                None if fg != Some(w.key) => {
                    self.behind.insert(w.key, (rule, now_us));
                    out.push(Effect::Log(format!(
                        "Regla al abrir ({app}): «{title}» apareció detrás; espera a pasar al frente."
                    )));
                }
                None => {
                    let arrival =
                        Arrival { rule, since_us: now_us, step: 0, inspecting: false, last: None };
                    self.arrivals.insert(w.key, arrival);
                }
            }
        }
    }

    /// `FOREGROUND` de una ventana: si era una nueva con regla que había
    /// aparecido detrás, empieza a seguirla.
    pub(super) fn came_forward(&mut self, win: WinKey, now_us: u64) {
        self.behind.retain(|_, (_, seen)| now_us.saturating_sub(*seen) <= BEHIND_MS * 1000);
        let Some((rule, _)) = self.behind.remove(&win) else { return };
        if self.paused || self.screens_frozen() {
            return;
        }
        let arrival = Arrival { rule, since_us: now_us, step: 0, inspecting: false, last: None };
        self.arrivals.insert(win, arrival);
    }

    /// `FOREGROUND` de una ventana desconocida: si hay reglas, se la sigue
    /// buscando hasta que aparezca en la lista (o se rinde a los 10 s). Sin
    /// reglas, nada extra.
    pub(super) fn watch_unseen(&mut self, win: WinKey, now_us: u64) {
        if self.paused || !self.bindings.iter().any(|b| b.open.is_some()) {
            return;
        }
        self.unseen = Some(Unseen { win, since_us: now_us, step: 0 });
    }

    pub(super) fn unseen_deadline(&self) -> Option<u64> {
        let u = self.unseen?;
        UNSEEN_MS.get(u.step).map(|ms| u.since_us + ms * 1000)
    }

    pub(super) fn unseen_tick(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        let Some(due) = self.unseen_deadline() else { return };
        if due > now_us {
            return;
        }
        let Some(u) = self.unseen.as_mut() else { return };
        u.step += 1;
        if u.step >= UNSEEN_MS.len() {
            self.unseen = None;
        }
        self.start_sync(crate::desks::Goal::Verify, now_us, out);
    }

    pub(super) fn arrival_deadline(&self) -> Option<u64> {
        self.arrivals
            .values()
            .filter(|a| !a.inspecting)
            .filter_map(|a| ARRIVE_MS.get(a.step).map(|ms| a.since_us + ms * 1000))
            .min()
    }

    pub(super) fn arrival_tick(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        let due: Vec<WinKey> = self
            .arrivals
            .iter()
            .filter(|(_, a)| {
                !a.inspecting
                    && ARRIVE_MS.get(a.step).is_some_and(|ms| a.since_us + ms * 1000 <= now_us)
            })
            .map(|(k, _)| *k)
            .collect();
        for win in due {
            if let Some(a) = self.arrivals.get_mut(&win) {
                a.inspecting = true;
            }
            let req = self.req();
            self.pending.insert(req, Pending::Arrive { win });
            out.push(Effect::Inspect { req, win: Some(win) });
        }
    }

    /// Una observación de la ventana nueva: si quedó quieta, la regla.
    pub(super) fn arrival_seen(
        &mut self,
        win: WinKey,
        result: Result<Obs, String>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(mut a) = self.arrivals.remove(&win) else { return };
        let Ok(obs) = result else { return }; // se cerró
        // El usuario ya la movió (Caps + Alt, arrastre) o la puso a pantalla
        // completa: la regla no corre.
        if self.jobs.contains_key(&win) || self.dragging_win() == Some(win) || is_fullscreen(&obs) {
            return;
        }
        let now = (obs.frame, obs.maximized);
        if a.last == Some(now) {
            self.apply_rule(a.rule, obs, now_us, out);
            return;
        }
        a.last = Some(now);
        a.step += 1;
        a.inspecting = false;
        if a.step >= ARRIVE_MS.len() {
            out.push(Effect::Log(format!(
                "Regla al abrir ({}): la ventana no se quedó quieta; no la muevo.",
                obs.app
            )));
            return;
        }
        self.arrivals.insert(win, a);
    }

    fn apply_rule(&mut self, rule: OpenRule, obs: Obs, now_us: u64, out: &mut Vec<Effect>) {
        let Some(m) = monitor_of(&obs.monitors, obs.frame) else { return };
        let monitor = obs.monitors[m];
        self.next_gesture += 1;
        let entry = Entry {
            win: obs.key,
            frame: obs.frame,
            maximized: obs.maximized,
            tag: self.next_gesture,
            desk: None,
        };
        let name = match rule {
            OpenRule::Place(p) => placement_name(p).to_owned(),
            OpenRule::Maximized => "maximizada".to_owned(),
            OpenRule::Centered(p) => format!("centrada al {p} %"),
        };
        let what = format!("Regla al abrir → {name} ({})", obs.app);
        // Sin animación: la miniatura de una ventana recién abierta sale vacía.
        let quiet = Obs { no_fly: Some(NoFly::NotFront), ..obs.clone() };
        match rule {
            OpenRule::Maximized => {
                if obs.maximized {
                    return;
                }
                self.remember(entry);
                out.push(Effect::Maximize { win: obs.key });
                out.push(Effect::Log(what));
            }
            OpenRule::Place(p) => {
                self.record = Some(entry);
                let spot = Spot { monitor, placement: p };
                self.go(&quiet, spot, place(monitor.work, p), what, now_us, now_us, out);
            }
            OpenRule::Centered(pct) => {
                self.record = Some(entry);
                let spot = Spot { monitor, placement: Placement::Full };
                let target = centered_percent(monitor.work, i32::from(pct));
                self.go(&quiet, spot, target, what, now_us, now_us, out);
            }
        }
    }
}
