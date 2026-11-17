//! Núcleo puro del gestor de ventanas.
//!
//! No depende de Windows ni usa `unsafe`: corre y se prueba en cualquier sistema
//! operativo (architecture.md §10).

#![forbid(unsafe_code)]

pub mod apps;
pub mod desks;
pub mod drag;
pub mod i18n;
pub mod keyboard;
pub mod map;
pub mod mapview;
pub mod mouse;
pub mod notices;
pub mod options;
pub mod place;
pub mod screens;
pub mod settings_demo;
pub mod update;
pub mod world;
#[cfg(test)]
mod world_arrange_tests;
#[cfg(test)]
mod world_desk_tests;
#[cfg(test)]
mod world_drag_tests;
#[cfg(test)]
mod world_map_tests;
#[cfg(test)]
mod world_screen_tests;

/// Rectángulo en píxeles físicos de la pantalla virtual.
///
/// Se guarda como bordes (izquierda, arriba, derecha, abajo) y no como
/// posición + tamaño: así dos ventanas vecinas comparten exactamente el mismo
/// borde y nunca queda un hueco de 1 px por redondeo (architecture.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Borde vertical que divide el rectángulo en la proporción `num / den`
    /// desde la izquierda. Usa aritmética entera para que el resultado sea
    /// idéntico en cada llamada.
    pub fn split_x(&self, num: i32, den: i32) -> i32 {
        self.left + (i64::from(self.width()) * i64::from(num) / i64::from(den)) as i32
    }

    pub fn split_y(&self, num: i32, den: i32) -> i32 {
        self.top + (i64::from(self.height()) * i64::from(num) / i64::from(den)) as i32
    }
}

/// Ubicaciones de la capa de ventana del mapa "Apps primero"
/// (keyboard-design.md §6.2: Hyper + pulgar + W/A/S/D y sus combinaciones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placement {
    Full,
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Rectángulo que ocupa `placement` dentro del área de trabajo `area`.
///
/// Las piezas complementarias (izquierda/derecha, arriba/abajo, los cuatro
/// cuartos) comparten bordes exactos: cubren el área sin huecos ni solapes,
/// también con anchos y altos impares.
pub fn place(area: Rect, placement: Placement) -> Rect {
    let mid_x = area.split_x(1, 2);
    let mid_y = area.split_y(1, 2);
    let Rect { left, top, right, bottom } = area;
    match placement {
        Placement::Full => area,
        Placement::LeftHalf => Rect::new(left, top, mid_x, bottom),
        Placement::RightHalf => Rect::new(mid_x, top, right, bottom),
        Placement::TopHalf => Rect::new(left, top, right, mid_y),
        Placement::BottomHalf => Rect::new(left, mid_y, right, bottom),
        Placement::TopLeft => Rect::new(left, top, mid_x, mid_y),
        Placement::TopRight => Rect::new(mid_x, top, right, mid_y),
        Placement::BottomLeft => Rect::new(left, mid_y, mid_x, bottom),
        Placement::BottomRight => Rect::new(mid_x, mid_y, right, bottom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: i32, height: i32) -> Rect {
        // Origen negativo a propósito: los monitores a la izquierda del
        // principal tienen coordenadas negativas en la pantalla virtual.
        Rect::new(-1921, 7, -1921 + width, 7 + height)
    }

    #[test]
    fn halves_share_the_middle_edge_with_odd_sizes() {
        for width in [1, 2, 3, 1919, 1920, 2561, 5120] {
            let a = area(width, 1047);
            let l = place(a, Placement::LeftHalf);
            let r = place(a, Placement::RightHalf);
            assert_eq!(l.left, a.left);
            assert_eq!(l.right, r.left, "sin hueco ni solape (ancho {width})");
            assert_eq!(r.right, a.right);
            assert_eq!(l.width() + r.width(), a.width());
        }
    }

    #[test]
    fn quarters_tile_the_area_exactly() {
        let a = area(3441, 1439);
        let quarters = [
            place(a, Placement::TopLeft),
            place(a, Placement::TopRight),
            place(a, Placement::BottomLeft),
            place(a, Placement::BottomRight),
        ];
        let covered: i64 =
            quarters.iter().map(|q| i64::from(q.width()) * i64::from(q.height())).sum();
        assert_eq!(covered, i64::from(a.width()) * i64::from(a.height()));
        assert_eq!(quarters[0].right, quarters[1].left);
        assert_eq!(quarters[0].bottom, quarters[2].top);
    }

    #[test]
    fn full_is_the_work_area() {
        let a = area(1920, 1040);
        assert_eq!(place(a, Placement::Full), a);
    }
}
