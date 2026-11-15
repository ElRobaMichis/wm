//! Colocación exacta por marco visible (architecture.md §8, spike 7).
//!
//! `SetWindowPos` recibe el rectángulo de la ventana **con** los bordes
//! invisibles de redimensionado (7 px a los lados y abajo a 100 %), pero lo
//! que el usuario ve es el marco (`DWMWA_EXTENDED_FRAME_BOUNDS`). El margen se
//! mide **en cada ventana y en el momento** (cambia con el DPI y con la app);
//! se aplica, se verifica y, solo si el error pasa de 1 px, se corrige una
//! vez más con el margen nuevo. Nunca en bucle.
//!
//! **Cruce a un monitor con otro DPI** (Fable, spike 7): la app recibe
//! `WM_DPICHANGED` y se reescala sola, después de nuestro `SetWindowPos`. Para
//! no dar por terminado antes de tiempo: la primera pasada usa el margen
//! escalado al DPI de destino, y la espera exige que la ventana ya informe
//! ese DPI (`GetDpiForWindow`, legible entre procesos sin mensajes) y que el
//! marco quede quieto. Un control a +250 ms mide si la app se movió tarde.

use std::time::{Duration, Instant};

use wm_model::Rect;

use crate::ghost::{frame_bounds, window_rect};
use crate::windows_list::{self, WindowId};

/// Error aceptado por borde.
pub const TOLERANCE_PX: i32 = 1;
/// Pasadas como máximo (la segunda solo si la primera quedó lejos).
pub const MAX_PASSES: u32 = 2;

#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub passes: u32,
    /// Mayor diferencia entre un borde del marco y el pedido, en px.
    pub error: i32,
    pub frame: Rect,
    pub elapsed: Duration,
    /// La ventana cambió de DPI (cruzó a un monitor con otra escala).
    pub dpi_changed: bool,
    /// Cuánto se movió la app sola entre el final y +250 ms (0 = nada).
    pub drift: i32,
}

/// DPI con el que la ventana se dibuja ahora.
pub fn window_dpi(id: WindowId) -> u32 {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    // SAFETY: consulta de solo lectura; no envía mensajes.
    unsafe { GetDpiForWindow(windows::Win32::Foundation::HWND(id.0 as *mut _)) }
}

/// ¿La ventana es "per-monitor" (se reescala sola al cambiar de monitor)?
pub fn per_monitor(id: WindowId) -> bool {
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_PER_MONITOR_AWARE, GetAwarenessFromDpiAwarenessContext,
        GetWindowDpiAwarenessContext,
    };
    // SAFETY: consultas de solo lectura.
    unsafe {
        let ctx = GetWindowDpiAwarenessContext(windows::Win32::Foundation::HWND(id.0 as *mut _));
        GetAwarenessFromDpiAwarenessContext(ctx) == DPI_AWARENESS_PER_MONITOR_AWARE
    }
}

/// DPI efectivo del monitor donde cae (la mayor parte de) `r`.
pub fn dpi_at(r: Rect) -> u32 {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromRect};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    let rect = RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    let (mut x, mut y) = (96u32, 96u32);
    // SAFETY: consultas de solo lectura con variables locales.
    unsafe {
        let m = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    }
    x
}

/// Mayor diferencia por borde entre dos rectángulos.
pub fn edge_error(a: Rect, b: Rect) -> i32 {
    [a.left - b.left, a.top - b.top, a.right - b.right, a.bottom - b.bottom]
        .iter()
        .map(|d| d.abs())
        .max()
        .unwrap_or(0)
}

/// Rectángulo de ventana para que el marco visible quede en `target`, con
/// los márgenes que tiene la ventana ahora, escalados de `from_dpi` a
/// `to_dpi` (los bordes invisibles crecen con la escala).
pub fn window_rect_for(id: WindowId, target: Rect, from_dpi: u32, to_dpi: u32) -> Option<Rect> {
    let (win, frame) = (window_rect(id)?, frame_bounds(id)?);
    let s = |m: i32| (i64::from(m) * i64::from(to_dpi.max(1)) / i64::from(from_dpi.max(1))) as i32;
    Some(Rect {
        left: target.left - s(frame.left - win.left),
        top: target.top - s(frame.top - win.top),
        right: target.right + s(win.right - frame.right),
        bottom: target.bottom + s(win.bottom - frame.bottom),
    })
}

/// Espera (hasta `timeout`) a que el marco deje de cambiar después de un
/// `SetWindowPos` asíncrono. Si la ventana tiene que terminar en `want_dpi`,
/// además espera a que lo informe. Devuelve el marco final.
fn settle(
    id: WindowId,
    before: Option<Rect>,
    target: Rect,
    want_dpi: Option<u32>,
    timeout: Duration,
) -> Option<Rect> {
    let start = Instant::now();
    let mut last = frame_bounds(id);
    let mut stable = 0;
    let dpi_ok = || want_dpi.is_none_or(|d| window_dpi(id) == d);
    while start.elapsed() < timeout {
        std::thread::sleep(Duration::from_millis(8));
        let now = frame_bounds(id);
        if !dpi_ok() {
            last = now;
            continue;
        }
        if want_dpi.is_none() && now.is_some_and(|f| edge_error(f, target) <= TOLERANCE_PX) {
            return now;
        }
        if now == last && now != before {
            stable += 1;
            if stable >= 3 {
                return now;
            }
        } else {
            stable = 0;
        }
        last = now;
    }
    last
}

/// Coloca la ventana para que su marco visible quede en `target`.
pub fn place_frame(id: WindowId, target: Rect) -> Option<Placed> {
    let started = Instant::now();
    // Una ventana minimizada o maximizada tiene márgenes que no sirven.
    let v = crate::hide::visibility(id);
    if v.minimized || crate::hide::is_maximized(id) {
        crate::hide::set_maximized_for_test(id, false);
        std::thread::sleep(Duration::from_millis(150));
    }
    let from_dpi = window_dpi(id);
    let to_dpi = dpi_at(target);
    let crossing = from_dpi != to_dpi && per_monitor(id);
    let mut passes = 0;
    let mut frame = frame_bounds(id)?;
    while passes < MAX_PASSES {
        passes += 1;
        // Primera pasada con el margen escalado; la segunda, con el real.
        let current = window_dpi(id);
        let scaled = if passes == 1 { to_dpi } else { current };
        let rect = window_rect_for(id, target, current, scaled)?;
        let before = Some(frame);
        windows_list::place_async(id, rect).ok()?;
        let want = crossing.then_some(to_dpi);
        frame = settle(id, before, target, want, Duration::from_millis(600))?;
        if edge_error(frame, target) <= TOLERANCE_PX {
            break;
        }
    }
    let elapsed = started.elapsed();
    // Control tardío: ¿la app se movió sola después?
    std::thread::sleep(Duration::from_millis(250));
    let late = frame_bounds(id).unwrap_or(frame);
    Some(Placed {
        passes,
        error: edge_error(late, target),
        frame: late,
        elapsed,
        dpi_changed: crossing,
        drift: edge_error(late, frame),
    })
}
