//! La transacción de escritorios (hito 4, `plan-hito4.md` §3): Scan → plan →
//! anotar → mostrar → foco → esconder. Una a la vez; si llega otra mientras
//! tanto, queda una sola en espera (gana la última).

use super::*;
use crate::desks::{FocusTo, Goal, Plan};

#[derive(Debug)]
pub(super) struct DeskSync {
    goal: Goal,
    key_us: u64,
    plan: Option<Plan>,
    /// Ya se intentó dar el foco al escritorio de Windows tras un fallo.
    shell_tried: bool,
}

/// Cómo se nombra cada objetivo en el log.
fn label(goal: Goal) -> String {
    match goal {
        Goal::Boot { .. } => "Escritorios".to_owned(),
        Goal::Desk(n) => format!("Caps + {n}"),
        Goal::Carry(n) => format!("Caps + Alt + {n}"),
        Goal::Window(_) => "Caps + letra".to_owned(),
        Goal::Foreign(_) => "Ventana de otro escritorio al frente".to_owned(),
        Goal::UndoCarry { .. } => "Caps + Alt + Z → deshacer".to_owned(),
        Goal::Verify => "Escritorios (reconciliar)".to_owned(),
        Goal::ShowAll => "Mostrar todas las ventanas".to_owned(),
    }
}

impl World {
    /// Solo se gestionan (esconden) las ventanas de esta app: para el
    /// `ci-probe`, que no puede tocar las ventanas del usuario.
    pub fn set_desk_scope(&mut self, app: Option<String>) {
        self.desks.set_scope(app);
    }

    /// El escritorio activo (1-4).
    pub fn active_desk(&self) -> u8 {
        self.desks.active()
    }

    /// Escritorio de una ventana conocida.
    pub fn desk_of(&self, win: WinKey) -> Option<u8> {
        self.desks.desk_of(win)
    }

    /// La ventana está en todos los escritorios (no se pudo esconder).
    pub fn desks_sticky(&self, win: WinKey) -> bool {
        self.desks.is_sticky(win)
    }

    /// Hay una transacción de escritorios en curso.
    pub fn syncing(&self) -> bool {
        self.sync.is_some()
    }

    /// La ventana es de otro escritorio (conocida, no sticky).
    pub(super) fn elsewhere(&self, win: WinKey) -> bool {
        !self.desks.is_sticky(win)
            && self.desks.desk_of(win).is_some_and(|d| d != self.desks.active())
    }

    /// Cualquier `Scan` adopta lo nuevo al escritorio activo y repara las
    /// props `wm.desk` que falten (salvo con una transacción en curso, que
    /// lo hace ella con su propio `Scan`).
    pub(super) fn adopt_scan(&mut self, windows: &[WinInfo], out: &mut Vec<Effect>) {
        if self.sync.is_some() {
            return;
        }
        let set = self.desks.adopt(windows);
        self.send_set(set, out);
    }

    fn send_set(&mut self, items: Vec<(WinKey, u8)>, out: &mut Vec<Effect>) {
        if items.is_empty() {
            return;
        }
        let req = self.req();
        self.pending.insert(req, Pending::DeskSet);
        out.push(Effect::SetDesks { req, items });
    }

    /// No se pudo anotar el escritorio: esa ventana no se puede esconder
    /// (elevada, por ejemplo) y queda en todos.
    pub(super) fn desks_set(&mut self, failed: Vec<WinKey>, out: &mut Vec<Effect>) {
        for k in failed {
            if !self.desks.is_sticky(k) {
                self.desks.make_sticky(k);
                out.push(Effect::Log(format!(
                    "Escritorios: no se pudo anotar la ventana {:#x}; queda en todos.",
                    k.hwnd
                )));
            }
        }
    }

    /// Empieza una transacción (o la deja en espera si hay otra).
    pub(super) fn start_sync(&mut self, goal: Goal, key_us: u64, out: &mut Vec<Effect>) {
        if self.queue_if_busy(goal, key_us) {
            return;
        }
        self.sync = Some(DeskSync { goal, key_us, plan: None, shell_tried: false });
        let req = self.req();
        self.pending.insert(req, Pending::DeskScan);
        out.push(Effect::Scan { req });
    }

    /// Como [`World::start_sync`], con un `Scan` que ya está (`Caps` + letra).
    pub(super) fn start_sync_with(
        &mut self,
        goal: Goal,
        key_us: u64,
        windows: &[WinInfo],
        fg: Option<WinKey>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        if self.queue_if_busy(goal, key_us) {
            return;
        }
        self.sync = Some(DeskSync { goal, key_us, plan: None, shell_tried: false });
        self.sync_plan(windows, fg, now_us, out);
    }

    fn queue_if_busy(&mut self, goal: Goal, key_us: u64) -> bool {
        if self.sync.is_none() {
            return false;
        }
        // Reconciliar no pisa un pedido del usuario.
        if goal != Goal::Verify || self.queued.is_none() {
            self.queued = Some((goal, key_us));
        }
        true
    }

    pub(super) fn sync_scanned(
        &mut self,
        result: Result<(Vec<WinInfo>, Option<WinKey>), String>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(s) = self.sync.as_ref() else { return };
        match result {
            Ok((windows, fg)) => self.sync_plan(&windows, fg, now_us, out),
            Err(e) => {
                out.push(Effect::Log(format!("{}: {e}", label(s.goal))));
                self.finish_sync(now_us, out);
            }
        }
    }

    fn sync_plan(
        &mut self,
        windows: &[WinInfo],
        fg: Option<WinKey>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(goal) = self.sync.as_ref().map(|s| s.goal) else { return };
        let jobs = &self.jobs;
        let flying = |k: WinKey| jobs.get(&k).is_some_and(|j| j.flying);
        match self.desks.plan(goal, windows, fg, &flying) {
            Err(e) => {
                out.push(Effect::Log(format!("{}: {e}.", label(goal))));
                self.finish_sync(now_us, out);
            }
            Ok(plan) => {
                self.send_set(plan.set.clone(), out);
                if let Some((win, from)) = plan.carried {
                    let tag = self.fresh_tag();
                    self.remember(Entry {
                        win,
                        frame: Rect::new(0, 0, 0, 0),
                        maximized: false,
                        tag,
                        desk: Some(from),
                    });
                }
                if let Some(s) = self.sync.as_mut() {
                    s.plan = Some(plan);
                }
                self.sync_show(now_us, out);
            }
        }
    }

    fn plan_ref(&self) -> Option<&Plan> {
        self.sync.as_ref().and_then(|s| s.plan.as_ref())
    }

    fn sync_show(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        let wins = self.plan_ref().map(|p| p.show.clone()).unwrap_or_default();
        if wins.is_empty() {
            return self.sync_focus(now_us, out);
        }
        let req = self.req();
        self.pending.insert(req, Pending::DeskShow);
        out.push(Effect::Show { req, wins });
    }

    pub(super) fn sync_shown(
        &mut self,
        result: Result<Vec<WinKey>, String>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(s) = self.sync.as_ref() else { return };
        let Some(plan) = s.plan.as_ref() else { return };
        let what = label(s.goal);
        let all_failed = match &result {
            Err(_) => true,
            Ok(failed) => !plan.show.is_empty() && failed.len() == plan.show.len(),
        };
        if all_failed && plan.active != plan.prev_active {
            // Explorer colgado o reiniciando: no se cambia de escritorio ni
            // se esconde nada (nunca dejar la pantalla vacía).
            let why = match result {
                Err(e) => e,
                Ok(_) => "ninguna ventana se pudo mostrar".to_owned(),
            };
            let prev = plan.prev_active;
            self.desks.revert(prev);
            out.push(Effect::Log(format!(
                "{what}: no se pudo mostrar ({why}); sigo en el {prev}."
            )));
            if let Some(s) = self.sync.as_mut() {
                s.plan = None;
            }
            return self.finish_sync(now_us, out);
        }
        if let Ok(failed) = &result
            && !failed.is_empty()
        {
            out.push(Effect::Log(format!(
                "{what}: {} ventanas no se pudieron mostrar.",
                failed.len()
            )));
        }
        self.sync_focus(now_us, out);
    }

    fn sync_focus(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        let focus = self.plan_ref().map_or(FocusTo::Keep, |p| p.focus);
        match focus {
            FocusTo::Keep => self.sync_hide(now_us, out),
            FocusTo::Win(win) => {
                let req = self.req();
                self.pending.insert(req, Pending::DeskFocus);
                out.push(Effect::Focus { req, win });
            }
            FocusTo::Shell => self.focus_shell(out),
        }
    }

    fn focus_shell(&mut self, out: &mut Vec<Effect>) {
        if let Some(s) = self.sync.as_mut() {
            s.shell_tried = true;
        }
        let req = self.req();
        self.pending.insert(req, Pending::DeskFocus);
        out.push(Effect::FocusShell { req });
    }

    pub(super) fn sync_focused(
        &mut self,
        result: Result<FocusReport, String>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(s) = self.sync.as_ref() else { return };
        let ok = result.as_ref().is_ok_and(|f| f.ok);
        if !ok && !s.shell_tried {
            // Nunca dejar el teclado sobre una ventana que se va a esconder.
            return self.focus_shell(out);
        }
        self.sync_hide(now_us, out);
    }

    fn sync_hide(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        let wins = self.plan_ref().map(|p| p.hide.clone()).unwrap_or_default();
        if wins.is_empty() {
            return self.finish_sync(now_us, out);
        }
        let req = self.req();
        self.pending.insert(req, Pending::DeskHide);
        out.push(Effect::Hide { req, wins });
    }

    pub(super) fn sync_hidden(
        &mut self,
        result: Result<Vec<WinKey>, String>,
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(s) = self.sync.as_ref() else { return };
        let what = label(s.goal);
        match result {
            // El lote entero falló (plazo, Explorer): el próximo cambio lo ve.
            Err(e) => out.push(Effect::Log(format!("{what}: no se pudo esconder ({e})."))),
            Ok(failed) => {
                for k in failed {
                    self.desks.make_sticky(k);
                    out.push(Effect::Log(format!(
                        "{what}: la ventana {:#x} no se pudo esconder; queda en todos los escritorios.",
                        k.hwnd
                    )));
                }
            }
        }
        self.finish_sync(now_us, out);
    }

    fn finish_sync(&mut self, now_us: u64, out: &mut Vec<Effect>) {
        if let Some(s) = self.sync.take()
            && let Some(plan) = s.plan
        {
            let quiet = s.goal == Goal::Verify && plan.show.is_empty() && plan.hide.is_empty();
            if !quiet {
                let since = now_us.saturating_sub(s.key_us) as f64 / 1000.0;
                out.push(Effect::Log(format!(
                    "{} → {}: {} mostradas, {} escondidas, {since:.1} ms desde la tecla",
                    label(s.goal),
                    plan.note,
                    plan.show.len(),
                    plan.hide.len()
                )));
            }
        }
        if let Some((goal, key_us)) = self.queued.take() {
            self.start_sync(goal, key_us, out);
        }
    }
}
