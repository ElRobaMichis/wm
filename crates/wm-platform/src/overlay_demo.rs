//! Escena de prueba del spike 5: un cuadrado magenta en cada monitor, en la
//! fila [`MARK_ROW`], que el medidor (`wm-bench`) busca en cada cuadro; y,
//! si se pide, el mapa de teclado con texto ([`crate::overlay_text`]).
//!
//! Todo se crea oculto. `show` arma el mapa (la primera vez), arranca las
//! animaciones y muestra; `hide` detiene las animaciones (una animación
//! `Forever` mantiene a DWM componiendo aunque no se vea) y esconde.

use std::time::Duration;

use windows::Foundation::TimeSpan;
use windows::UI::Color;
use windows::UI::Composition::{
    AnimationDirection, AnimationIterationBehavior, AnimationStopBehavior, SpriteVisual,
};
use windows::core::{HSTRING, IUnknown};
use windows_numerics::{Vector2, Vector3};

use crate::overlay::Overlays;
use crate::overlay_text::{self, TextPath};

/// Fila (relativa a cada monitor) por la que pasa el cuadrado.
pub const MARK_ROW: u32 = 400;
const SIZE: f32 = 80.0;
/// Una pasada de borde a borde.
const SWEEP: Duration = Duration::from_millis(1000);

/// Movimiento del cuadrado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// Diente de sierra (izquierda → derecha, vuelve al inicio), sin fin.
    Loop,
    /// Una sola pasada de esta duración; queda en el borde derecho.
    Once(Duration),
}

pub struct Demo {
    overlays: Overlays,
    squares: Vec<(SpriteVisual, f32)>,
    motion: Motion,
    text: Option<TextPath>,
    keep: Vec<IUnknown>,
}

impl Demo {
    pub fn new(motion: Motion, text: Option<TextPath>) -> windows::core::Result<Self> {
        let overlays = Overlays::new()?;
        let c = overlays.compositor();
        let brush = c.CreateColorBrushWithColor(Color { A: 255, R: 255, G: 0, B: 255 })?;
        let mut squares = Vec::new();
        for s in overlays.screens() {
            let sprite = c.CreateSpriteVisual()?;
            sprite.SetSize(Vector2 { X: SIZE, Y: SIZE })?;
            sprite.SetBrush(&brush)?;
            let y = MARK_ROW as f32 - SIZE / 2.0;
            sprite.SetOffset(Vector3 { X: 0.0, Y: y, Z: 0.0 })?;
            s.root.Children()?.InsertAtTop(&sprite)?;
            squares.push((sprite, s.bounds.width() as f32 - SIZE));
        }
        Ok(Self { overlays, squares, motion, text, keep: Vec::new() })
    }

    pub fn show(&mut self) -> windows::core::Result<()> {
        if let Some(path) = self.text
            && self.keep.is_empty()
        {
            let screens = self.overlays.screens();
            let screen = screens.iter().find(|s| s.primary).unwrap_or(&screens[0]);
            let (w, h) = (screen.bounds.width() as f32, screen.bounds.height() as f32);
            self.keep =
                overlay_text::build_map(self.overlays.compositor(), &screen.root, w, h, path)?;
        }
        let c = self.overlays.compositor();
        let linear = c.CreateLinearEasingFunction()?;
        for (sprite, end) in &self.squares {
            let y = MARK_ROW as f32 - SIZE / 2.0;
            let anim = c.CreateVector3KeyFrameAnimation()?;
            anim.InsertKeyFrameWithEasingFunction(0.0, Vector3 { X: 0.0, Y: y, Z: 0.0 }, &linear)?;
            anim.InsertKeyFrameWithEasingFunction(1.0, Vector3 { X: *end, Y: y, Z: 0.0 }, &linear)?;
            match self.motion {
                Motion::Loop => {
                    anim.SetDuration(span(SWEEP))?;
                    anim.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
                    anim.SetDirection(AnimationDirection::Normal)?;
                }
                Motion::Once(d) => {
                    anim.SetDuration(span(d))?;
                    anim.SetIterationBehavior(AnimationIterationBehavior::Count)?;
                    anim.SetIterationCount(1)?;
                    anim.SetStopBehavior(AnimationStopBehavior::LeaveCurrentValue)?;
                }
            }
            sprite.StartAnimation(&HSTRING::from("Offset"), &anim)?;
        }
        self.overlays.show();
        Ok(())
    }

    pub fn hide(&mut self) -> windows::core::Result<()> {
        for (sprite, _) in &self.squares {
            sprite.StopAnimation(&HSTRING::from("Offset"))?;
        }
        self.overlays.hide();
        Ok(())
    }
}

fn span(d: Duration) -> TimeSpan {
    TimeSpan { Duration: (d.as_nanos() / 100) as i64 }
}
