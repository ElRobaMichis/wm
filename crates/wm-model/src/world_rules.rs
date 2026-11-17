//! v1.1 (plan-v1.1.md): la regla "al abrir" de cada app. Se aplica una sola
//! vez, a la primera ventana principal de una app con regla, cuando la app
//! termina de ubicarla (dos observaciones iguales); si la app se reubica
//! después, gana la app. Sin animación; Z la devuelve adonde la abrió la app.

use super::{Effect, Entry, NoFly, Obs, Pending, Spot, WinInfo, WinKey, World, is_fullscreen};
use crate::place::{centered_percent, edge_error, monitor_of, placement_name};
use crate::rules::OpenRule;
use crate::{Placement, Rect};

/// Cuándo mirar la ventana nueva (desde que se la vio).
const ARRIVE_MS: [u64; 4] = [100, 300, 700, 1500];

/// Cuánto puede tardar en pasar al frente una ventana nueva que apareció
/// detrás (la de IntelliJ, tras su pantalla de carga).
const BEHIND_MS: u64 = 5_000;

/// Margen para creerle a una ventana que dice estar maximizada (px).
const COVER_PX: i32 = 8;

/// Cuándo volver a buscar una ventana que pasó al frente y el `Scan` no vio
/// (desde el `FOREGROUND`).
const UNSEEN_MS: [u64; 12] =
    [250, 500, 750, 1000, 1500, 2000, 2500, 3000, 4000, 5000, 7000, 10_000];

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

/// ¿Maximizada de verdad? Steam (CEF) tiene la marca puesta con la ventana
/// a tamaño normal: solo cuenta si además cubre el área de trabajo.
pub(super) fn looks_maximized(obs: &Obs) -> bool {
    let Some(m) = monitor_of(&obs.monitors, obs.frame) else { return obs.maximized };
    let (w, f) = (obs.monitors[m].work, obs.frame);
    obs.maximized
        && f.left <= w.left + COVER_PX
        && f.top <= w.top + COVER_PX
        && f.right >= w.right - COVER_PX
        && f.bottom >= w.bottom - COVER_PX
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
        // La que pasó al frente antes de aparecer en la lista ya vino al
        // frente: no "apareció detrás" aunque ahora no lo esté (la de
        // IntelliJ; pruebas del usuario, 17/11/2026).
        let mut came_first = None;
        if let Some(u) = self.unseen
            && windows.iter().any(|w| w.key == u.win)
        {
            self.unseen = None;
            came_first = Some(u.win);
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
                None if fg != Some(w.key) && came_first != Some(w.key) => {
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
    pub(super) fn came_forward(&mut self, win: WinKey, now_us: u64, out: &mut Vec<Effect>) {
        self.behind.retain(|_, (_, seen)| now_us.saturating_sub(*seen) <= BEHIND_MS * 1000);
        let Some((rule, _)) = self.behind.remove(&win) else { return };
        if self.paused || self.screens_frozen() {
            out.push(Effect::Log(
                "Regla al abrir: pasó al frente, pero wm está en pausa o cambiando las pantallas."
                    .into(),
            ));
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
        let obs = match result {
            Ok(obs) => obs,
            Err(e) => {
                out.push(Effect::Log(format!(
                    "Regla al abrir: la ventana ya no está ({e}); no la muevo."
                )));
                return;
            }
        };
        // El usuario ya la movió (Caps + Alt, arrastre) o la puso a pantalla
        // completa: la regla no corre.
        let why = if self.jobs.contains_key(&win) || self.dragging_win() == Some(win) {
            Some("la moviste tú antes de que se quedara quieta")
        } else if is_fullscreen(&obs) {
            Some("está a pantalla completa")
        } else {
            None
        };
        if let Some(why) = why {
            out.push(Effect::Log(format!("Regla al abrir ({}): no la muevo: {why}.", obs.app)));
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
        let Some(own) = monitor_of(&obs.monitors, obs.frame) else {
            out.push(Effect::Log(format!(
                "Regla al abrir ({}): no está en ningún monitor ({:?}); no la muevo.",
                obs.app, obs.frame
            )));
            return;
        };
        // (b) Abrir en la pantalla del puntero (con más de un monitor).
        let m = obs
            .pointer
            .filter(|_| self.options.open_on_pointer && obs.monitors.len() > 1)
            .and_then(|(x, y)| monitor_of(&obs.monitors, Rect::new(x, y, x + 1, y + 1)))
            .unwrap_or(own);
        let monitor = obs.monitors[m];
        let there = if m == own { "" } else { ", en la pantalla del puntero" };
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
        let what = format!("Regla al abrir → {name}{there} ({})", obs.app);
        // Sin animación: la miniatura de una ventana recién abierta sale vacía.
        let quiet = Obs { no_fly: Some(NoFly::NotFront), ..obs.clone() };
        match rule {
            OpenRule::Maximized => {
                if m != own {
                    // Z la devuelve a su monitor, como se veía.
                    self.remember(Entry { maximized: looks_maximized(&obs), ..entry });
                    self.maximize_on(&obs, m, out);
                    out.push(Effect::Log(what));
                    return;
                }
                if looks_maximized(&obs) {
                    out.push(Effect::Log(format!(
                        "Regla al abrir ({}): ya se abrió maximizada.",
                        obs.app
                    )));
                    return;
                }
                // Z la devuelve a como se veía: a tamaño normal.
                self.remember(Entry { maximized: false, ..entry });
                if obs.maximized {
                    // Con la marca puesta, maximizar no hace nada: primero
                    // se la saca.
                    out.push(Effect::Normalize { win: obs.key });
                }
                out.push(Effect::Maximize { win: obs.key });
                out.push(Effect::Log(what));
            }
            OpenRule::Place(_) | OpenRule::Centered(_) => {
                let (spot, target) = match rule {
                    OpenRule::Place(p) => (Spot { monitor, placement: p }, self.slot(&monitor, p)),
                    OpenRule::Centered(pct) => (
                        Spot { monitor, placement: Placement::Full },
                        centered_percent(monitor.work, i32::from(pct)),
                    ),
                    OpenRule::Maximized => unreachable!(),
                };
                // Ya se abrió ahí (Rayo esconde su ventana al cerrarla y la
                // vuelve a mostrar donde estaba): `go` no hace nada; se anota.
                if !obs.maximized && edge_error(obs.frame, target) <= super::TOLERANCE_PX {
                    out.push(Effect::Log(format!(
                        "Regla al abrir ({}): ya se abrió {}.",
                        obs.app,
                        rule.describe()
                    )));
                }
                self.record = Some(entry);
                self.go(&quiet, spot, target, what, now_us, now_us, out);
            }
        }
    }
}

/// "Acomodala primero" (plan-v1.2.md §4).
const ARRANGE_FIRST: &str = "Acomodala primero: mitad, esquina, maximizada o centrada";

impl World {
    /// El nombre de la app de `letter` para los avisos.
    fn app_name(&self, letter: u8) -> Option<String> {
        self.bindings
            .iter()
            .find(|b| b.letter == letter)
            .map(|b| b.label.clone().unwrap_or_else(|| crate::mapview::display_name(&b.app)))
    }

    /// v1.2: "Abrir siempre así" desde la bandeja: mirar cómo está `win`.
    /// Funciona también en pausa (no mueve nada).
    pub(super) fn keep_like_this(&mut self, win: WinKey, letter: u8, out: &mut Vec<Effect>) {
        if self.app_name(letter).is_none() {
            out.push(Effect::Log(format!(
                "Abrir siempre así: la {} no tiene app.",
                letter as char
            )));
            return;
        }
        let req = self.req();
        self.pending.insert(req, Pending::Keep { letter });
        out.push(Effect::Inspect { req, win: Some(win) });
    }

    /// La ventana observada: su regla, si está acomodada, y a guardar.
    pub(super) fn keep_seen(
        &mut self,
        letter: u8,
        result: Result<Obs, String>,
        out: &mut Vec<Effect>,
    ) {
        let Some(name) = self.app_name(letter) else { return };
        let obs = match result {
            Ok(obs) => obs,
            Err(e) => {
                out.push(Effect::Log(format!(
                    "Abrir siempre así ({name}): la ventana ya no está ({e})."
                )));
                out.push(Effect::Notice("Esa ventana ya no está".into()));
                return;
            }
        };
        let m = monitor_of(&obs.monitors, obs.frame);
        let rule = match m {
            Some(m) if !obs.minimized && !is_fullscreen(&obs) => {
                let (spot, rect) = self.locate(&obs, m);
                let real_max = looks_maximized(&obs);
                // Maximizada de mentira (Steam): cuenta su marco.
                let placement = if obs.maximized && !real_max {
                    crate::place::slot_at(&obs.monitors[m], obs.frame, self.options.gap)
                        .unwrap_or(Placement::Full)
                } else {
                    spot.placement
                };
                crate::rules::rule_like(rect, real_max, placement, obs.monitors[m].work)
            }
            _ => None,
        };
        let Some(rule) = rule else {
            out.push(Effect::Log(format!(
                "Abrir siempre así ({name}): la ventana está libre, minimizada o a pantalla completa ({:?})."
                , obs.frame
            )));
            out.push(Effect::Notice(ARRANGE_FIRST.into()));
            return;
        };
        let current = self.bindings.iter().find(|b| b.letter == letter).and_then(|b| b.open);
        if current == Some(rule) {
            out.push(Effect::Log(format!(
                "Abrir siempre así ({name}): ya se abre {}.",
                rule.describe()
            )));
            out.push(Effect::Notice(format!("{name} ya se abre así")));
            return;
        }
        out.push(Effect::Log(format!(
            "Abrir siempre así → {name} ({}): {}.",
            letter as char,
            rule.format()
        )));
        out.push(Effect::SaveRule { letter, open: Some(rule) });
        self.keeping = Some((letter, rule));
    }

    /// `apps.txt` volvió: si trae la regla recién guardada, avisarlo (así
    /// el aviso confirma todo el camino: archivo, `wm-watch` y modelo).
    pub(super) fn kept(&mut self, out: &mut Vec<Effect>) {
        let Some((letter, rule)) = self.keeping else { return };
        if !self.bindings.iter().any(|b| b.letter == letter && b.open == Some(rule)) {
            return;
        }
        self.keeping = None;
        if let Some(name) = self.app_name(letter) {
            out.push(Effect::Notice(format!("{name} se va a abrir siempre {}", rule.describe())));
        }
    }
}
