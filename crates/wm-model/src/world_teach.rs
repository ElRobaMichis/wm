//! v1.2 (plan-v1.2.md §6): "Sugerir atajos". Si una ventana del selector
//! de Windows (Alt+Tab, la barra de tareas, Win+Tab) pasa al frente y en
//! menos de 5 s llega otra ventana de usuario, la eligió a mano: si su app
//! tiene letra, se anota en el log y, con los topes de `teach.rs`, un aviso
//! le recuerda la letra.

use super::{Effect, Layer, Pending, WinInfo, WinKey, World};
use crate::apps::{Target, resolve};

/// Cuánto puede tardar en llegar la ventana elegida (µs).
const CHOSEN_WITHIN_US: u64 = 5_000_000;

impl World {
    /// Lo que se aprendió en otras sesiones (`aprendizaje.txt`).
    pub fn set_learning(&mut self, text: &str) {
        self.teach = crate::teach::Teach::parse(text);
    }

    /// ¿Lo está haciendo wm (o el usuario con `Caps`)? Entonces no es a mano.
    fn ours_now(&self) -> bool {
        self.paused
            || self.layer != Layer::Off
            // Reconciliar (lo dispara el mismo selector al frente) no cuenta.
            || self.user_desk_busy()
            || self.pending.values().any(|p| matches!(p, Pending::Focus { .. }))
    }

    /// Una ventana del selector de Windows pasó al frente.
    pub(super) fn on_shell_foreground(&mut self, win: WinKey, now_us: u64) {
        self.shell_at = (!self.ours_now()).then_some((now_us, win));
    }

    /// Al principio de `on_foreground`: ¿la eligió a mano?
    pub(super) fn maybe_chosen(&mut self, win: WinKey, now_us: u64, out: &mut Vec<Effect>) {
        if self.shell_at.is_some_and(|(_, shell)| shell == win) {
            return; // el selector mismo
        }
        let shell = self.shell_at.take();
        let prev = self.prev_fg.replace(win);
        let chosen = shell.is_some_and(|(t, _)| now_us.saturating_sub(t) <= CHOSEN_WITHIN_US)
            && prev != Some(win)
            && !self.ours_now();
        if !chosen {
            return;
        }
        // Su app y su letra salen de la lista de ventanas.
        let req = self.req();
        self.pending.insert(req, Pending::TeachScan { win });
        out.push(Effect::Scan { req });
    }

    /// La lista, para saber la letra de la ventana elegida a mano.
    pub(super) fn teach_seen(
        &mut self,
        win: WinKey,
        windows: &[WinInfo],
        now_us: u64,
        out: &mut Vec<Effect>,
    ) {
        let Some(w) = windows.iter().find(|w| w.key == win) else { return };
        let id = w.app_id();
        let letter =
            self.bindings.iter().find(|b| b.matches(&id)).map(|b| b.letter).or_else(|| {
                // Sin asignar: la inicial, si `Caps` + esa letra llega a ella.
                let l = id.name().bytes().next()?.to_ascii_uppercase();
                let list: Vec<(WinKey, crate::apps::AppId)> =
                    windows.iter().map(|w| (w.key, w.app_id())).collect();
                match resolve(l, &self.bindings, &list) {
                    Target::Focus { windows, .. } if windows.contains(&win) => Some(l),
                    _ => None,
                }
            });
        let Some(letter) = letter.filter(u8::is_ascii_uppercase) else { return };
        let key = letter as char;
        out.push(Effect::Log(format!(
            "A mano (Alt+Tab o la barra) → {}: tiene la {key}.",
            id.name()
        )));
        if self.options.tips && self.teach.switched(letter, now_us) {
            out.push(Effect::Notice(format!("La próxima vez: Caps + {key}")));
            out.push(Effect::SaveLearning(self.teach.format()));
        }
    }

    /// `Caps` + letra llegó a una ventana: cuenta para dejar de sugerirla
    /// (con la opción apagada no se cuenta: la sonda no escribe nada).
    pub(super) fn teach_used(&mut self, letter: u8, out: &mut Vec<Effect>) {
        if self.options.tips && self.teach.used(letter.to_ascii_uppercase()) {
            out.push(Effect::SaveLearning(self.teach.format()));
        }
    }
}
