//! Capa de plataforma (Win32).
//!
//! - `input`: hook de teclado para Caps = Hyper (spike 1).
//! - `windows_list`: enumerar ventanas de usuario y traerlas al frente (spike 1).
//! - Información del sistema para `wm doctor` (spike 0).

#[cfg(windows)]
pub mod anim;
#[cfg(windows)]
pub mod autostart;
#[cfg(windows)]
pub mod clock;
#[cfg(windows)]
pub mod console;
#[cfg(windows)]
pub mod display_watch;
#[cfg(windows)]
pub mod displays;
#[cfg(windows)]
mod dynlib;
#[cfg(windows)]
pub mod geometry;
#[cfg(windows)]
pub mod ghost;
#[cfg(windows)]
pub mod ghost_curtain;
#[cfg(windows)]
pub mod hide;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod instance;
#[cfg(windows)]
pub mod launch;
#[cfg(windows)]
pub mod native_settings;
#[cfg(windows)]
pub mod ops;
#[cfg(windows)]
pub mod overlay;
#[cfg(windows)]
pub mod overlay_demo;
#[cfg(windows)]
pub mod overlay_text;
#[cfg(windows)]
pub mod probe;
#[cfg(windows)]
pub mod puppet;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod ui;
#[cfg(windows)]
mod win;
#[cfg(windows)]
pub mod windows_list;

#[cfg(windows)]
pub use win::{dpi_awareness, monitors, os_build, os_edition};

use wm_model::Rect;

/// Datos de un monitor conectado.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// Nombre de dispositivo GDI (`\\.\DISPLAY1`). Solo para diagnóstico:
    /// no es una identidad estable (architecture.md §8).
    pub device: String,
    pub primary: bool,
    /// Rectángulo completo del monitor, en píxeles físicos.
    pub bounds: Rect,
    /// Área de trabajo (sin barra de tareas), en píxeles físicos.
    pub work_area: Rect,
    /// DPI efectivo (96 = 100 %).
    pub dpi: u32,
    /// Frecuencia de refresco en Hz, si Windows la informa.
    pub refresh_hz: Option<u32>,
}
