//! Cuándo se ve el mapa de teclado (plan-core.md §5, hito 2a). Puro: lo usa
//! [`crate::world::World`] con los mismos eventos de capa y acción que ya
//! recibe, y su plazo entra en `World::deadline`.
//!
//! - Mantener Hyper **sin acción** [`MAP_DELAY_MS`] → se muestra la capa
//!   actual (el pulgar cambia de capa, antes o después de mostrarse).
//! - Una acción **antes** del plazo suprime el mapa toda esa pulsación: los
//!   atajos rápidos nunca lo ven.
//! - Con el mapa visible, una acción no lo cierra (D7); Esc sí, y en ese caso
//!   no sale del programa.
//! - Se oculta al soltar Hyper, con cualquier reinicio del teclado y, como
//!   tope, a los [`MAP_MAX_US`] (además pide reiniciar el teclado: si se perdió
//!   "Caps arriba", Hyper no puede quedar trabado).
//!
//! Invariante (prueba de propiedad en `world_map_tests`): mapa visible ⇒
//! Hyper abajo; toda secuencia termina con el mapa oculto.

use crate::keyboard::{Layer, MAP_DELAY_MS};

/// Tope de un mapa visible.
pub const MAP_MAX_US: u64 = 30_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MapState {
    /// Hyper arriba (o mapa deshabilitado).
    #[default]
    Off,
    /// Hyper abajo sin acción: se muestra en `due_us`.
    Armed {
        due_us: u64,
    },
    Visible {
        until_us: u64,
    },
    /// Hubo una acción antes del plazo (o Esc): nada hasta soltar Hyper.
    Suppressed,
}

/// Lo que hay que hacer con el mapa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapOut {
    Show(Layer),
    /// Ya visible: mostrar otra capa.
    Layer(Layer),
    Hide,
    /// Tope vencido: reiniciar la máquina de teclado del hook.
    ResetKeyboard,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Map {
    state: MapState,
    enabled: bool,
}

impl Map {
    pub fn state(&self) -> MapState {
        self.state
    }

    pub fn visible(&self) -> bool {
        matches!(self.state, MapState::Visible { .. })
    }

    /// Habilitar (el hilo que lo dibuja está listo) o no. Deshabilitar oculta.
    pub fn set_enabled(&mut self, on: bool, out: &mut Vec<MapOut>) {
        self.enabled = on;
        if !on {
            self.reset(out);
        }
    }

    pub fn deadline(&self) -> Option<u64> {
        match self.state {
            MapState::Armed { due_us } => Some(due_us),
            MapState::Visible { until_us } => Some(until_us),
            _ => None,
        }
    }

    /// Cambió la capa de `prev` a `layer`.
    pub fn on_layer(&mut self, prev: Layer, layer: Layer, now_us: u64, out: &mut Vec<MapOut>) {
        if layer == Layer::Off {
            self.reset(out);
            return;
        }
        if !self.enabled {
            return;
        }
        let armed = MapState::Armed { due_us: now_us + u64::from(MAP_DELAY_MS) * 1000 };
        if prev == Layer::Off {
            self.state = armed;
        } else if prev == layer {
            // La misma capa otra vez: el hook se reinició sin avisar (modo
            // seguro, bloqueo) y esta es una pulsación nueva.
            self.reset(out);
            self.state = armed;
        } else if self.visible() {
            out.push(MapOut::Layer(layer));
        }
    }

    /// Llegó una acción de Hyper. Devuelve `true` si el mapa la consumió (Esc
    /// con el mapa visible: lo cierra y no sale del programa).
    pub fn on_action(&mut self, escape: bool, out: &mut Vec<MapOut>) -> bool {
        match self.state {
            MapState::Armed { .. } => {
                self.state = MapState::Suppressed;
                false
            }
            MapState::Visible { .. } if escape => {
                self.state = MapState::Suppressed;
                out.push(MapOut::Hide);
                true
            }
            _ => false,
        }
    }

    /// Venció el plazo. `layer`: la capa actual.
    pub fn tick(&mut self, layer: Layer, now_us: u64, out: &mut Vec<MapOut>) {
        match self.state {
            MapState::Armed { due_us } if due_us <= now_us => {
                self.state = MapState::Visible { until_us: now_us + MAP_MAX_US };
                out.push(MapOut::Show(layer));
            }
            MapState::Visible { until_us } if until_us <= now_us => {
                self.state = MapState::Off;
                out.push(MapOut::Hide);
                out.push(MapOut::ResetKeyboard);
            }
            _ => {}
        }
    }

    /// Hyper arriba o reinicio del teclado.
    pub fn reset(&mut self, out: &mut Vec<MapOut>) {
        if self.visible() {
            out.push(MapOut::Hide);
        }
        self.state = MapState::Off;
    }
}
