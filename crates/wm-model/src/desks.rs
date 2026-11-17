//! Escritorios 1–4 (hito 4, `plan-hito4.md` §2): a qué escritorio pertenece
//! cada ventana y qué hay que mostrar, enfocar y esconder para llegar a un
//! objetivo. Puro: decide sobre lo **observado** en un `Scan`, así que cada
//! cambio también reconcilia lo que se haya desviado.

use std::collections::HashMap;

use crate::world::{WinInfo, WinKey};

/// Cantidad de escritorios.
pub const DESKS: u8 = 4;

/// Lo que se quiere lograr con una transacción.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    /// Arranque del core. `recover`: relanzado tras una caída (D4), confiar
    /// en `wm.desk` de cada ventana; si no, todas al 1.
    Boot { recover: bool },
    /// Ir al escritorio N (`Caps` + N). Si ya es el activo, solo reconcilia.
    Desk(u8),
    /// Ir al escritorio de esta ventana y enfocarla (`Caps` + letra).
    Window(WinKey),
    /// Esta ventana de otro escritorio pasó al frente (barra de tareas,
    /// Alt+Tab): saltar a su escritorio o devolverla (rebote).
    Foreign(WinKey),
    /// Llevar la ventana del frente al escritorio N (`Caps` + Alt + N).
    Carry(u8),
    /// Deshacer un "llevar": la ventana vuelve al escritorio `to`.
    UndoCarry { win: WinKey, to: u8 },
    /// Solo reconciliar (Explorer reinició, un destape inesperado).
    Verify,
    /// "Mostrar todas las ventanas": todas al escritorio activo.
    ShowAll,
}

/// A quién dar el foco entre mostrar y esconder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTo {
    Win(WinKey),
    /// Al escritorio de Windows (el destino está vacío).
    Shell,
    /// No tocar el foco.
    Keep,
}

/// Lo que hay que hacer, en orden: anotar escritorios, mostrar, enfocar,
/// esconder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub active: u8,
    /// Escritorio anterior (para volver atrás si mostrar falla entero).
    pub prev_active: u8,
    pub set: Vec<(WinKey, u8)>,
    pub show: Vec<WinKey>,
    pub focus: FocusTo,
    pub hide: Vec<WinKey>,
    /// Para "llevar": la ventana y el escritorio de donde salió (diario).
    pub carried: Option<(WinKey, u8)>,
    /// Qué se hizo, para el log.
    pub note: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct DeskWin {
    desk: u8,
    /// No se esconde nunca (no se pudo marcar o esconder, o fuera del
    /// alcance de prueba): está en todos los escritorios.
    sticky: bool,
    /// Destapes inesperados seguidos (R4: con 2, `sticky`).
    strikes: u8,
}

#[derive(Debug, Default)]
pub struct Desks {
    /// 0 = todavía no arrancó (cuenta como 1).
    active: u8,
    wins: HashMap<WinKey, DeskWin>,
    last_focus: [Option<WinKey>; DESKS as usize],
    /// Última ventana conocida al frente del escritorio activo (para decidir
    /// entre salto y rebote).
    last_fg: Option<WinKey>,
    /// Solo se gestionan las ventanas de esta app (el `ci-probe`, para no
    /// esconder las del usuario).
    scope: Option<String>,
}

impl Desks {
    pub fn active(&self) -> u8 {
        self.active.max(1)
    }

    /// ¿Esta app está dentro del alcance (sin alcance: todas)?
    pub fn in_scope(&self, app: &str) -> bool {
        self.scope.as_ref().is_none_or(|s| s == app)
    }

    pub fn set_scope(&mut self, app: Option<String>) {
        self.scope = app;
    }

    /// Escritorio de una ventana conocida.
    pub fn desk_of(&self, win: WinKey) -> Option<u8> {
        self.wins.get(&win).map(|d| d.desk)
    }

    pub fn is_sticky(&self, win: WinKey) -> bool {
        self.wins.get(&win).is_some_and(|d| d.sticky)
    }

    /// No se pudo marcar o esconder: queda en todos los escritorios.
    pub fn make_sticky(&mut self, win: WinKey) {
        if let Some(d) = self.wins.get_mut(&win) {
            d.sticky = true;
        }
    }

    /// Un destape inesperado: suma un strike; con 2, `sticky` (devuelve
    /// `true` si pasó a serlo).
    pub fn strike(&mut self, win: WinKey) -> bool {
        match self.wins.get_mut(&win) {
            Some(d) if !d.sticky => {
                d.strikes += 1;
                if d.strikes >= 2 {
                    d.sticky = true;
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// Ventanas por escritorio (para el mapa): `[1, 2, 3, 4]`.
    pub fn counts(&self) -> [usize; DESKS as usize] {
        let mut c = [0; DESKS as usize];
        for d in self.wins.values().filter(|d| !d.sticky) {
            if (1..=DESKS).contains(&d.desk) {
                c[usize::from(d.desk - 1)] += 1;
            }
        }
        c
    }

    /// La ventana `fg` está al frente y es del escritorio activo (o sticky):
    /// recordar a quién volver.
    pub fn note_foreground(&mut self, fg: WinKey) {
        let active = self.active();
        if let Some(d) = self.wins.get(&fg)
            && (d.desk == active || d.sticky)
        {
            if d.desk == active && !d.sticky {
                self.last_focus[usize::from(active - 1)] = Some(fg);
            }
            self.last_fg = Some(fg);
        }
    }

    /// Adopta lo nuevo (al escritorio activo), poda lo que ya no existe y
    /// devuelve las props `wm.desk` que hay que (re)escribir. Lo usa cualquier
    /// `Scan`, también el de `Caps` + letra y el del mapa.
    pub fn adopt(&mut self, wins: &[WinInfo]) -> Vec<(WinKey, u8)> {
        self.adopt_with(wins, false)
    }

    fn adopt_with(&mut self, wins: &[WinInfo], trust_props: bool) -> Vec<(WinKey, u8)> {
        let active = self.active();
        let mut set = Vec::new();
        let mut next: HashMap<WinKey, DeskWin> = HashMap::with_capacity(wins.len());
        for w in wins {
            let out_of_scope = self.scope.as_ref().is_some_and(|s| *s != w.app);
            let mut d = match self.wins.get(&w.key) {
                Some(d) => *d,
                None => {
                    let desk = match w.desk {
                        Some(n) if trust_props && (1..=DESKS).contains(&n) => n,
                        _ => active,
                    };
                    DeskWin { desk, ..DeskWin::default() }
                }
            };
            if out_of_scope {
                d.sticky = true;
            }
            if !d.sticky && w.desk != Some(d.desk) {
                set.push((w.key, d.desk));
            }
            next.insert(w.key, d);
        }
        self.wins = next;
        for f in &mut self.last_focus {
            if f.is_some_and(|k| !self.wins.contains_key(&k)) {
                *f = None;
            }
        }
        if self.last_fg.is_some_and(|k| !self.wins.contains_key(&k)) {
            self.last_fg = None;
        }
        set
    }

    /// El plan para `goal` sobre lo observado (`wins` en orden Z, la primera
    /// arriba; `fg`: la del frente, ya traducida al dueño raíz). `flying`: la
    /// ventana está en vuelo (no se esconde). `Err`: no hay nada que hacer
    /// (texto para el log).
    pub fn plan(
        &mut self,
        goal: Goal,
        wins: &[WinInfo],
        fg: Option<WinKey>,
        flying: &dyn Fn(WinKey) -> bool,
    ) -> Result<Plan, String> {
        let prev_active = self.active();
        let mut set = match goal {
            Goal::Boot { recover } => {
                self.wins.clear();
                self.last_focus = [None; DESKS as usize];
                self.last_fg = None;
                self.active = 1;
                if recover {
                    // D4: activo = el de la ventana al frente; si no, el de la
                    // primera con escritorio en el orden Z; si no, el 1.
                    let desk_of = |k: WinKey| wins.iter().find(|w| w.key == k).and_then(|w| w.desk);
                    self.active = fg
                        .and_then(desk_of)
                        .or_else(|| wins.iter().find_map(|w| w.desk))
                        .filter(|n| (1..=DESKS).contains(n))
                        .unwrap_or(1);
                }
                self.adopt_with(wins, recover)
            }
            _ => self.adopt_with(wins, false),
        };
        // El foco actual es del escritorio que se deja: recordarlo (después
        // de adoptar, para que cuente también una ventana recién vista).
        if let Some(f) = fg {
            self.note_foreground(f);
        }
        let known = |k: WinKey| wins.iter().find(|w| w.key == k);
        let mut focus = FocusTo::Keep;
        let mut carried = None;
        let note;
        match goal {
            Goal::Boot { recover } => {
                note = if recover {
                    format!("arranque tras una caída: escritorio {}", self.active)
                } else {
                    "arranque: todo en el escritorio 1".to_owned()
                };
            }
            Goal::Desk(n) => {
                if !(1..=DESKS).contains(&n) {
                    return Err(format!("no hay escritorio {n}"));
                }
                if n == prev_active {
                    note = format!("ya en el escritorio {n}");
                } else {
                    self.active = n;
                    focus = self.best_focus(n, wins, None);
                    note = format!("escritorio {n}");
                }
            }
            Goal::Window(k) => {
                let Some(d) = self.wins.get(&k).copied() else {
                    return Err("la ventana ya no existe".to_owned());
                };
                if !d.sticky {
                    self.active = d.desk;
                }
                focus = FocusTo::Win(k);
                note = format!("escritorio {} (ventana elegida)", self.active());
            }
            Goal::Foreign(k) => {
                let Some(d) = self.wins.get(&k).copied() else {
                    return Err("la ventana ya no existe".to_owned());
                };
                if d.sticky || d.desk == prev_active {
                    note = "ya es del escritorio activo".to_owned();
                } else if fg != Some(k) {
                    // Aviso viejo: ya no está al frente. Solo reconciliar.
                    note = "aviso viejo: solo reconciliar".to_owned();
                } else if self.last_fg.and_then(known).is_some_and(|w| !w.minimized && w.key != k) {
                    // El usuario la eligió (barra, Alt+Tab): saltar.
                    self.active = d.desk;
                    note = format!("escritorio {} (la eligieron en la barra o Alt+Tab)", d.desk);
                } else {
                    // La que estaba al frente se cerró o minimizó y Windows
                    // activó una escondida: no cambiar de escritorio.
                    focus = self.best_focus(prev_active, wins, Some(k));
                    note = "rebote: Windows activó una ventana de otro escritorio".to_owned();
                }
            }
            Goal::Carry(n) => {
                if !(1..=DESKS).contains(&n) {
                    return Err(format!("no hay escritorio {n}"));
                }
                let Some(win) = fg.filter(|k| self.wins.contains_key(k)) else {
                    return Err("no hay una ventana al frente para llevar".to_owned());
                };
                let d = self.wins[&win];
                if d.sticky {
                    return Err("esa ventana no se puede esconder".to_owned());
                }
                if d.desk == n {
                    return Err(format!("ya está en el escritorio {n}"));
                }
                self.wins.get_mut(&win).expect("recién visto").desk = n;
                set.retain(|(k, _)| *k != win);
                set.push((win, n));
                self.last_focus[usize::from(n - 1)] = Some(win);
                if self.last_fg == Some(win) {
                    self.last_fg = None;
                }
                let slot = &mut self.last_focus[usize::from(prev_active - 1)];
                if *slot == Some(win) {
                    *slot = None;
                }
                carried = Some((win, d.desk));
                if d.desk == prev_active {
                    focus = self.best_focus(prev_active, wins, Some(win));
                }
                note = format!("llevada al escritorio {n}");
            }
            Goal::UndoCarry { win, to } => {
                let Some(d) = self.wins.get_mut(&win) else {
                    return Err("la ventana ya no existe".to_owned());
                };
                d.desk = to;
                set.retain(|(k, _)| *k != win);
                set.push((win, to));
                if to == prev_active {
                    focus = FocusTo::Win(win);
                }
                note = format!("vuelve al escritorio {to}");
            }
            Goal::Verify => note = "reconciliar".to_owned(),
            Goal::ShowAll => {
                let active = self.active();
                for (k, d) in &mut self.wins {
                    d.sticky = self.scope.is_some()
                        && known(*k).is_some_and(|w| Some(&w.app) != self.scope.as_ref());
                    d.strikes = 0;
                    if !d.sticky && d.desk != active {
                        d.desk = active;
                        set.retain(|(x, _)| x != k);
                        set.push((*k, active));
                    }
                }
                note = format!("todas al escritorio {active}");
            }
        }
        let active = self.active();
        if active != prev_active {
            for d in self.wins.values_mut() {
                if d.desk == active {
                    d.strikes = 0;
                }
            }
        }
        let mut show = Vec::new();
        let mut hide = Vec::new();
        for w in wins {
            let Some(d) = self.wins.get(&w.key) else { continue };
            let here = d.sticky || d.desk == active;
            if here {
                if w.marked || (goal == Goal::ShowAll && w.concealed && !w.minimized) {
                    show.push(w.key);
                }
            } else if !flying(w.key) && !(w.marked && w.concealed) {
                hide.push(w.key);
            }
        }
        // Nunca dejar el teclado en una ventana que se va a esconder.
        if focus == FocusTo::Keep
            && let Some(f) = fg
            && hide.contains(&f)
        {
            focus = self.best_focus(active, wins, Some(f));
        }
        // Mostrar primero la que va a tener el foco (llega antes).
        if let FocusTo::Win(k) = focus
            && let Some(i) = show.iter().position(|x| *x == k)
        {
            let k = show.remove(i);
            show.insert(0, k);
        }
        if active != prev_active
            && let FocusTo::Win(k) = focus
        {
            self.last_focus[usize::from(active - 1)] = Some(k);
        }
        Ok(Plan { active, prev_active, set, show, focus, hide, carried, note })
    }

    /// A quién enfocar al llegar a `desk`: la última enfocada ahí (si sigue y
    /// no está minimizada); si no, la de más arriba no minimizada; si no, el
    /// escritorio de Windows.
    fn best_focus(&self, desk: u8, wins: &[WinInfo], except: Option<WinKey>) -> FocusTo {
        let usable = |w: &WinInfo| {
            Some(w.key) != except
                && !w.minimized
                && self.wins.get(&w.key).is_some_and(|d| d.desk == desk && !d.sticky)
        };
        if let Some(k) = self.last_focus[usize::from(desk - 1)]
            && wins.iter().any(|w| w.key == k && usable(w))
        {
            return FocusTo::Win(k);
        }
        wins.iter().find(|w| usable(w)).map_or(FocusTo::Shell, |w| FocusTo::Win(w.key))
    }

    /// Si mostrar falló entero: volver al escritorio anterior.
    pub fn revert(&mut self, prev_active: u8) {
        self.active = prev_active;
    }
}
