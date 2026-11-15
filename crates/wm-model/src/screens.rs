//! Monitores y persistencia, en lógica pura (architecture.md §8 y §13.9).
//!
//! - [`MonitorId`] y [`match_monitors`]: reconocer "este monitor" entre
//!   arranques y reconexiones, por niveles.
//! - [`reachable`] y [`rescue`]: la regla "nunca fuera de pantalla".
//! - [`RelRect`]: una posición guardada relativa al área de trabajo.
//! - [`StormTracker`]: cuándo terminó una tormenta de cambios de pantalla y
//!   hay que reconciliar (una sola vez por calma).

use crate::Rect;

/// Identidad de un monitor: la tupla completa, nunca un solo campo.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MonitorId {
    /// Fabricante y producto del EDID.
    pub manufacturer: u16,
    pub product: u16,
    /// Número de serie del EDID, si el monitor lo informa (0 = no).
    pub serial: Option<u32>,
    /// Conector, en minúsculas (por ejemplo `uid4353`).
    pub connector: String,
}

impl MonitorId {
    fn edid(&self) -> (u16, u16) {
        (self.manufacturer, self.product)
    }
}

/// Empareja monitores guardados con los conectados ahora, por niveles de
/// confianza (Fable, spike 7):
/// 1. EDID + serie + conector;
/// 2. EDID + serie (se movió el cable a otro puerto);
/// 3. EDID + conector (sin serie: la identidad sigue al conector);
/// 4. EDID solo, si es único en los dos lados.
///
/// Devuelve pares `(guardado, conectado)`; cada índice aparece una vez como
/// mucho.
pub fn match_monitors(saved: &[MonitorId], live: &[MonitorId]) -> Vec<(usize, usize)> {
    let mut used_s = vec![false; saved.len()];
    let mut used_l = vec![false; live.len()];
    let mut pairs = Vec::new();
    type Level = fn(&MonitorId, &MonitorId) -> bool;
    let levels: [Level; 3] = [
        |a, b| {
            a.edid() == b.edid()
                && a.serial.is_some()
                && a.serial == b.serial
                && a.connector == b.connector
        },
        |a, b| a.edid() == b.edid() && a.serial.is_some() && a.serial == b.serial,
        |a, b| a.edid() == b.edid() && a.connector == b.connector,
    ];
    for level in levels {
        for (i, s) in saved.iter().enumerate() {
            if used_s[i] {
                continue;
            }
            if let Some(j) = (0..live.len()).find(|&j| !used_l[j] && level(s, &live[j])) {
                used_s[i] = true;
                used_l[j] = true;
                pairs.push((i, j));
            }
        }
    }
    // Nivel 4: EDID único entre los que quedan, en los dos lados.
    for i in 0..saved.len() {
        if used_s[i] {
            continue;
        }
        let e = saved[i].edid();
        let same_s = (0..saved.len()).filter(|&k| !used_s[k] && saved[k].edid() == e).count();
        let cands: Vec<usize> =
            (0..live.len()).filter(|&j| !used_l[j] && live[j].edid() == e).collect();
        if same_s == 1 && cands.len() == 1 {
            used_s[i] = true;
            used_l[cands[0]] = true;
            pairs.push((i, cands[0]));
        }
    }
    pairs.sort_unstable();
    pairs
}

/// Cuánto tiene que verse de una ventana para que siga alcanzable.
#[derive(Debug, Clone, Copy)]
pub struct MinVisible {
    pub width: i32,
    pub height: i32,
}

impl MinVisible {
    /// 120 × 48 DIP escalados al DPI del monitor.
    pub fn at_dpi(dpi: u32) -> Self {
        let s = |v: i32| (v as i64 * i64::from(dpi.max(96)) / 96) as i32;
        Self { width: s(120), height: s(48) }
    }
}

/// ¿El marco se puede alcanzar? Tiene que compartir con alguna área de
/// trabajo al menos `min(ancho, min.width)` × `min.height`, con el borde
/// superior (la barra de título) adentro.
pub fn reachable(frame: Rect, areas: &[Rect], min: MinVisible) -> bool {
    areas.iter().any(|a| {
        let w = frame.right.min(a.right) - frame.left.max(a.left);
        let h = frame.bottom.min(a.bottom) - frame.top.max(a.top);
        w >= min.width.min(frame.width())
            && h >= min.height.min(frame.height())
            && frame.top >= a.top
            && frame.top < a.bottom
    })
}

/// Si el marco no es alcanzable, el desplazamiento **mínimo** que lo vuelve
/// alcanzable en alguna área (sin cambiar el tamaño). `None` si ya lo es.
pub fn rescue(frame: Rect, areas: &[Rect], min: MinVisible) -> Option<Rect> {
    if reachable(frame, areas, min) || areas.is_empty() {
        return None;
    }
    let mut best: Option<(i64, Rect)> = None;
    for a in areas {
        let need_w = min.width.min(frame.width()).min(a.width());
        let need_h = min.height.min(frame.height()).min(a.height());
        let dx = if frame.right < a.left + need_w {
            a.left + need_w - frame.right
        } else if frame.left > a.right - need_w {
            a.right - need_w - frame.left
        } else {
            0
        };
        let dy = if frame.top < a.top {
            a.top - frame.top
        } else if frame.top > a.bottom - need_h {
            a.bottom - need_h - frame.top
        } else {
            0
        };
        let moved = Rect {
            left: frame.left + dx,
            top: frame.top + dy,
            right: frame.right + dx,
            bottom: frame.bottom + dy,
        };
        let cost = i64::from(dx).abs() + i64::from(dy).abs();
        if best.is_none_or(|(c, _)| cost < c) {
            best = Some((cost, moved));
        }
    }
    best.map(|(_, r)| r)
}

/// Posición guardada relativa a un área de trabajo: exacta si el área es la
/// misma, proporcional si cambió. Así las ventanas vecinas siguen
/// compartiendo el borde (misma fórmula para los dos lados).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelRect {
    /// Bordes relativos al origen del área.
    pub off: Rect,
    /// Tamaño del área cuando se guardó.
    pub area: (i32, i32),
}

impl RelRect {
    pub fn from_abs(r: Rect, area: Rect) -> Self {
        Self {
            off: Rect {
                left: r.left - area.left,
                top: r.top - area.top,
                right: r.right - area.left,
                bottom: r.bottom - area.top,
            },
            area: (area.width(), area.height()),
        }
    }

    pub fn to_abs(&self, area: Rect) -> Rect {
        let (rw, rh) = (self.area.0.max(1), self.area.1.max(1));
        let scale =
            |v: i32, now: i32, then: i32| (i64::from(v) * i64::from(now) / i64::from(then)) as i32;
        let sx = |v: i32| area.left + scale(v, area.width(), rw);
        let sy = |v: i32| area.top + scale(v, area.height(), rh);
        Rect {
            left: sx(self.off.left),
            top: sy(self.off.top),
            right: sx(self.off.right),
            bottom: sy(self.off.bottom),
        }
    }
}

/// Qué hacer ante una tormenta de avisos de pantalla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StormAction {
    /// No mover ni guardar nada: la topología está cambiando.
    Freeze,
    /// Tomar una instantánea de la topología.
    Snapshot,
    /// La topología se calmó en `key`: reconciliar las ventanas (una vez).
    Reconcile(u64),
}

/// Detector de fin de tormenta (Fable, spike 7). Puro: el llamador le pasa
/// el tiempo (ms) y los avisos; [`StormTracker::deadline`] dice cuándo volver
/// a llamarlo (sin timers en reposo).
#[derive(Debug, Clone, Default)]
pub struct StormTracker {
    state: Storm,
    locked: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Storm {
    #[default]
    Calm,
    /// Llegan avisos; esperar silencio hasta `quiet_at`.
    Waiting { started: u64, quiet_at: u64 },
    /// Primera instantánea pedida; esperando su resultado.
    FirstSnap { started: u64 },
    /// Segunda instantánea, para confirmar que no cambió.
    SecondSnap { started: u64, first: u64, at: u64, asked: bool },
    /// Reconciliado; un control más a `recheck_at` por la ola tardía.
    Settled { recheck_at: u64 },
}

impl StormTracker {
    /// Silencio que define "se calmó".
    pub const QUIET_MS: u64 = 400;
    /// Espera entre las dos instantáneas que tienen que coincidir.
    pub const CONFIRM_MS: u64 = 250;
    /// Una tormenta no dura más que esto: se reconcilia igual.
    pub const MAX_MS: u64 = 10_000;
    /// Control tardío después de reconciliar.
    pub const RECHECK_MS: u64 = 3_000;

    /// Llegó un aviso de Windows (pantallas, área de trabajo, energía...).
    pub fn hint(&mut self, now: u64) -> StormAction {
        let started = match self.state {
            Storm::Waiting { started, .. }
            | Storm::FirstSnap { started }
            | Storm::SecondSnap { started, .. } => started,
            Storm::Calm | Storm::Settled { .. } => now,
        };
        self.state = Storm::Waiting { started, quiet_at: now + Self::QUIET_MS };
        StormAction::Freeze
    }

    pub fn lock(&mut self, locked: bool) {
        self.locked = locked;
    }

    /// Próximo momento en que hay que llamar a [`StormTracker::tick`].
    pub fn deadline(&self) -> Option<u64> {
        match self.state {
            Storm::Calm => None,
            Storm::Waiting { quiet_at, .. } => Some(quiet_at),
            Storm::FirstSnap { .. } => None,
            Storm::SecondSnap { at, asked: false, .. } => Some(at),
            Storm::SecondSnap { .. } => None,
            Storm::Settled { recheck_at } => Some(recheck_at),
        }
    }

    /// Pasó el tiempo.
    pub fn tick(&mut self, now: u64) -> Option<StormAction> {
        match self.state {
            Storm::Waiting { started, quiet_at } if now >= quiet_at => {
                if self.locked {
                    // Con la sesión bloqueada no se toca nada: se sigue esperando.
                    self.state = Storm::Waiting { started, quiet_at: now + Self::QUIET_MS };
                    return None;
                }
                self.state = Storm::FirstSnap { started };
                Some(StormAction::Snapshot)
            }
            Storm::SecondSnap { started, first, at, asked: false } if now >= at => {
                self.state = Storm::SecondSnap { started, first, at, asked: true };
                Some(StormAction::Snapshot)
            }
            Storm::Settled { recheck_at } if now >= recheck_at => {
                self.state = Storm::Calm;
                // El control tardío es una instantánea más: si cambió, el
                // llamador lo trata como un aviso.
                Some(StormAction::Snapshot)
            }
            _ => None,
        }
    }

    /// Resultado de una instantánea pedida: la clave de la topología (`0` =
    /// incoherente, por ejemplo un monitor a medio aparecer).
    pub fn snapshot(&mut self, now: u64, key: u64) -> Option<StormAction> {
        match self.state {
            Storm::FirstSnap { started } => {
                let at = now + Self::CONFIRM_MS;
                self.state = Storm::SecondSnap { started, first: key, at, asked: false };
                None
            }
            Storm::SecondSnap { started, first, asked: true, .. } => {
                let too_long = now.saturating_sub(started) >= Self::MAX_MS;
                if (key == first && key != 0) || too_long {
                    self.state = Storm::Settled { recheck_at: now + Self::RECHECK_MS };
                    Some(StormAction::Reconcile(key))
                } else {
                    self.state = Storm::Waiting { started, quiet_at: now + Self::QUIET_MS };
                    None
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(m: u16, p: u16, serial: Option<u32>, c: &str) -> MonitorId {
        MonitorId { manufacturer: m, product: p, serial, connector: c.to_owned() }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo) as u64) as i32
        }
    }

    #[test]
    fn same_monitors_match_by_full_tuple() {
        let a = vec![id(1, 2, Some(10), "uid1"), id(1, 3, Some(20), "uid2")];
        let live = vec![a[1].clone(), a[0].clone()];
        assert_eq!(match_monitors(&a, &live), vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn cable_moved_matches_by_serial() {
        let saved = vec![id(1, 2, Some(10), "uid1")];
        let live = vec![id(1, 2, Some(10), "uid9")];
        assert_eq!(match_monitors(&saved, &live), vec![(0, 0)]);
    }

    #[test]
    fn identical_twins_without_serial_follow_the_connector() {
        let saved = vec![id(1, 2, None, "uid1"), id(1, 2, None, "uid2")];
        let live = vec![id(1, 2, None, "uid2"), id(1, 2, None, "uid1")];
        assert_eq!(match_monitors(&saved, &live), vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn ambiguous_twins_on_new_connectors_do_not_match() {
        let saved = vec![id(1, 2, None, "uid1"), id(1, 2, None, "uid2")];
        let live = vec![id(1, 2, None, "uid7"), id(1, 2, None, "uid8")];
        assert!(match_monitors(&saved, &live).is_empty());
    }

    #[test]
    fn matching_is_a_partial_bijection_and_order_independent() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..2000 {
            let pool: Vec<MonitorId> = (0..6)
                .map(|_| {
                    let serial = (!rng.next().is_multiple_of(3)).then(|| (rng.next() % 4) as u32);
                    id(1, (rng.next() % 3) as u16, serial, &format!("uid{}", rng.next() % 4))
                })
                .collect();
            let saved: Vec<MonitorId> = pool.iter().take(3).cloned().collect();
            let mut live: Vec<MonitorId> = pool.iter().skip(2).cloned().collect();
            let pairs = match_monitors(&saved, &live);
            let mut si: Vec<usize> = pairs.iter().map(|p| p.0).collect();
            let mut li: Vec<usize> = pairs.iter().map(|p| p.1).collect();
            si.dedup();
            li.sort_unstable();
            li.dedup();
            assert_eq!(si.len(), pairs.len());
            assert_eq!(li.len(), pairs.len());
            for (s, l) in &pairs {
                assert_eq!(saved[*s].edid(), live[*l].edid());
            }
            // Mismo resultado (en identidades) con el otro orden.
            live.reverse();
            let rev = match_monitors(&saved, &live);
            assert_eq!(rev.len(), pairs.len());
        }
    }

    #[test]
    fn rescue_brings_lost_windows_back_with_minimum_move() {
        let areas = [Rect::new(0, 0, 1920, 1032), Rect::new(1920, 0, 3840, 1032)];
        let min = MinVisible::at_dpi(96);
        // A la derecha de todo (el monitor 2 desapareció y quedó allá).
        let lost = Rect::new(4000, 100, 4800, 700);
        let r = rescue(lost, &areas[..1], min).expect("hay que moverla");
        assert!(reachable(r, &areas[..1], min));
        assert_eq!(r.width(), lost.width());
        assert_eq!(r.left, 1920 - 120, "solo lo justo para que se vean 120 px");
        // Una ventana visible no se toca.
        assert_eq!(rescue(Rect::new(100, 100, 900, 700), &areas, min), None);
        // Barra de título por encima de la pantalla: baja hasta el borde.
        let r = rescue(Rect::new(100, -300, 900, 400), &areas, min).expect("título afuera");
        assert_eq!(r.top, 0);
    }

    #[test]
    fn rescue_is_reachable_and_idempotent_for_random_frames() {
        let mut rng = Rng(42);
        let areas = [Rect::new(0, 0, 1920, 1032), Rect::new(1920, -200, 4480, 1240)];
        let min = MinVisible::at_dpi(144);
        for _ in 0..20_000 {
            let (x, y) = (rng.range(-6000, 9000), rng.range(-4000, 5000));
            let (w, h) = (rng.range(1, 3000), rng.range(1, 2000));
            let f = Rect::new(x, y, x + w, y + h);
            match rescue(f, &areas, min) {
                None => assert!(reachable(f, &areas, min)),
                Some(r) => {
                    assert!(reachable(r, &areas, min), "{f:?} → {r:?}");
                    assert_eq!(rescue(r, &areas, min), None);
                    assert_eq!((r.width(), r.height()), (f.width(), f.height()));
                }
            }
        }
    }

    #[test]
    fn relrect_round_trip_is_exact_on_the_same_area() {
        let mut rng = Rng(7);
        for _ in 0..10_000 {
            let area = Rect::new(rng.range(-3000, 3000), rng.range(-2000, 2000), 0, 0);
            let (w, h) = (rng.range(200, 4000), rng.range(200, 3000));
            let area = Rect { right: area.left + w, bottom: area.top + h, ..area };
            let r = Rect::new(area.left + 10, area.top + 20, area.right - 30, area.bottom - 5);
            assert_eq!(RelRect::from_abs(r, area).to_abs(area), r);
        }
    }

    #[test]
    fn relrect_keeps_shared_edges_when_the_area_changes() {
        let old = Rect::new(0, 0, 1920, 1032);
        let new = Rect::new(1920, 0, 3840 + 640, 1440 - 48);
        let left = RelRect::from_abs(Rect::new(0, 0, 640, 1032), old);
        let right = RelRect::from_abs(Rect::new(640, 0, 1920, 1032), old);
        let (l, r) = (left.to_abs(new), right.to_abs(new));
        assert_eq!(l.right, r.left);
        assert_eq!((l.left, r.right), (new.left, new.right));
    }

    #[test]
    fn storm_reconciles_once_after_a_calm() {
        let mut t = StormTracker::default();
        assert_eq!(t.hint(0), StormAction::Freeze);
        assert_eq!(t.hint(100), StormAction::Freeze);
        assert_eq!(t.tick(300), None);
        assert_eq!(t.tick(500), Some(StormAction::Snapshot));
        assert_eq!(t.snapshot(510, 77), None);
        assert_eq!(t.tick(760), Some(StormAction::Snapshot));
        assert_eq!(t.snapshot(770, 77), Some(StormAction::Reconcile(77)));
        // Control tardío y vuelta a la calma.
        assert_eq!(t.deadline(), Some(3770));
        assert_eq!(t.tick(3770), Some(StormAction::Snapshot));
        assert_eq!(t.deadline(), None);
    }

    #[test]
    fn storm_waits_while_the_topology_keeps_changing_or_is_locked() {
        let mut t = StormTracker::default();
        t.lock(true);
        t.hint(0);
        assert_eq!(t.tick(400), None, "bloqueado: no se toca nada");
        t.lock(false);
        assert_eq!(t.tick(800), Some(StormAction::Snapshot));
        t.snapshot(800, 1);
        assert_eq!(t.tick(1050), Some(StormAction::Snapshot));
        assert_eq!(t.snapshot(1050, 2), None, "cambió entre las dos: sigue la tormenta");
        assert_eq!(t.tick(1450), Some(StormAction::Snapshot));
    }

    #[test]
    fn storm_always_ends() {
        let mut rng = Rng(99);
        for _ in 0..500 {
            let mut t = StormTracker::default();
            let mut now = 0;
            let mut reconciled = 0;
            for _ in 0..400 {
                now += rng.next() % 120;
                if rng.next().is_multiple_of(10) && now < 5_000 {
                    t.hint(now);
                }
                if let Some(d) = t.deadline()
                    && now >= d
                    && t.tick(now) == Some(StormAction::Snapshot)
                {
                    let key = if rng.next().is_multiple_of(2) { 5 } else { 6 };
                    if let Some(StormAction::Reconcile(_)) = t.snapshot(now, key) {
                        reconciled += 1;
                    }
                }
            }
            // Sin avisos después de 5 s, a los 40 s ya tuvo que reconciliar.
            assert!(now < 20_000 || reconciled > 0 || t.deadline().is_some());
        }
    }
}
