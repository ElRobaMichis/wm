//! Texto en overlays: tres caminos a comparar en el spike 5 (architecture.md
//! §13.7). Los tres dibujan el mismo "mapa de teclado": 27 teclas con su
//! letra y el nombre de una app.
//!
//! - [`TextPath::Shapes`]: DirectWrite da los **contornos** de los glifos, que
//!   se vuelven `CompositionPath` → `ShapeVisual`. Sin dispositivo D3D en
//!   nuestro proceso: lo rasteriza DWM.
//! - [`TextPath::D2dHardware`] / [`TextPath::D2dWarp`]: D2D dibuja en una
//!   `CompositionDrawingSurface` con un dispositivo D3D11 por hardware o por
//!   software (WARP).
//!
//! Las funciones de `dwrite.dll`, `d2d1.dll` y `d3d11.dll` se resuelven en
//! tiempo de ejecución ([`crate::dynlib`]).

use std::ffi::c_void;

use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::Graphics::IGeometrySource2D;
use windows::UI::Color;
use windows::UI::Composition::{
    CompositionColorBrush, CompositionPath, Compositor, ContainerVisual,
};
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT,
    ID2D1DeviceContext, ID2D1Factory, ID2D1Factory1, ID2D1Geometry,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_METRICS, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_GLYPH_METRICS, DWRITE_MEASURING_MODE_NATURAL,
    IDWriteFactory, IDWriteFontFace,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Composition::{
    ICompositionDrawingSurfaceInterop, ICompositorInterop,
};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGeometrySource2DInterop, IGeometrySource2DInterop_Impl,
};
use windows::core::{GUID, HRESULT, IUnknown, Interface, PCWSTR, implement, s, w};
use windows_numerics::{Matrix3x2, Vector2, Vector3};

/// Cómo se dibuja el texto del mapa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPath {
    Shapes,
    D2dHardware,
    D2dWarp,
}

impl TextPath {
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "shape" => Some(Self::Shapes),
            "d2d" => Some(Self::D2dHardware),
            "warp" => Some(Self::D2dWarp),
            _ => None,
        }
    }
}

const ROWS: [&str; 3] = ["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"];
const APPS: [&str; 9] =
    ["brave", "code", "terminal", "spotify", "discord", "notion", "figma", "slack", "outlook"];
const KEY: f32 = 96.0;
const GAP: f32 = 12.0;
const LETTER_EM: f32 = 40.0;
const NAME_EM: f32 = 14.0;

fn map_size() -> (f32, f32) {
    (10.0 * KEY + 9.0 * GAP, 3.0 * KEY + 2.0 * GAP)
}

/// Posición (x, y) de cada tecla dentro del mapa, con su letra y app.
fn keys() -> Vec<(f32, f32, char, &'static str)> {
    let mut out = Vec::new();
    for (r, row) in ROWS.iter().enumerate() {
        let indent = r as f32 * (KEY + GAP) / 2.0;
        for (c, ch) in row.chars().enumerate() {
            let x = indent + c as f32 * (KEY + GAP);
            let y = r as f32 * (KEY + GAP);
            out.push((x, y, ch, APPS[out.len() % APPS.len()]));
        }
    }
    out
}

/// Construye el mapa y lo agrega a `parent`, centrado en un área de
/// `area_w` × `area_h`. Devuelve los objetos que hay que mantener vivos.
pub fn build_map(
    compositor: &Compositor,
    parent: &ContainerVisual,
    area_w: f32,
    area_h: f32,
    path: TextPath,
) -> windows::core::Result<Vec<IUnknown>> {
    let (w, h) = map_size();
    let container = compositor.CreateContainerVisual()?;
    container.SetSize(Vector2 { X: w, Y: h })?;
    container.SetOffset(Vector3 { X: (area_w - w) / 2.0, Y: (area_h - h) / 2.0, Z: 0.0 })?;
    parent.Children()?.InsertAtTop(&container)?;
    let mut keep: Vec<IUnknown> = vec![container.cast()?];
    match path {
        TextPath::Shapes => build_shapes(compositor, &container, &mut keep)?,
        TextPath::D2dHardware => {
            build_d2d(compositor, &container, D3D_DRIVER_TYPE_HARDWARE, &mut keep)?;
        }
        TextPath::D2dWarp => build_d2d(compositor, &container, D3D_DRIVER_TYPE_WARP, &mut keep)?,
    }
    Ok(keep)
}

// ---------------------------------------------------------------------------
// Fábricas (resueltas en tiempo de ejecución)
// ---------------------------------------------------------------------------

fn dwrite_factory() -> windows::core::Result<IDWriteFactory> {
    type Create = unsafe extern "system" fn(i32, *const GUID, *mut *mut c_void) -> HRESULT;
    let f = crate::dynlib::proc(w!("dwrite.dll"), s!("DWriteCreateFactory"))?;
    // SAFETY: firma documentada de DWriteCreateFactory.
    let create: Create = unsafe { std::mem::transmute(f) };
    let mut raw = std::ptr::null_mut();
    // SAFETY: IID válido y puntero de salida local.
    unsafe { create(DWRITE_FACTORY_TYPE_SHARED.0, &IDWriteFactory::IID, &mut raw) }.ok()?;
    // SAFETY: referencia propia recién creada.
    Ok(unsafe { IDWriteFactory::from_raw(raw) })
}

fn d2d_factory<T: Interface>() -> windows::core::Result<T> {
    type Create =
        unsafe extern "system" fn(i32, *const GUID, *const c_void, *mut *mut c_void) -> HRESULT;
    let f = crate::dynlib::proc(w!("d2d1.dll"), s!("D2D1CreateFactory"))?;
    // SAFETY: firma documentada de D2D1CreateFactory.
    let create: Create = unsafe { std::mem::transmute(f) };
    let mut raw = std::ptr::null_mut();
    // SAFETY: IID válido, sin opciones, puntero de salida local.
    unsafe { create(D2D1_FACTORY_TYPE_SINGLE_THREADED.0, &T::IID, std::ptr::null(), &mut raw) }
        .ok()?;
    // SAFETY: referencia propia recién creada.
    Ok(unsafe { T::from_raw(raw) })
}

fn d3d11_device(driver: D3D_DRIVER_TYPE) -> windows::core::Result<IDXGIDevice> {
    type Create = unsafe extern "system" fn(
        *mut c_void,
        D3D_DRIVER_TYPE,
        HMODULE,
        u32,
        *const c_void,
        u32,
        u32,
        *mut *mut c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT;
    let f = crate::dynlib::proc(w!("d3d11.dll"), s!("D3D11CreateDevice"))?;
    // SAFETY: firma documentada de D3D11CreateDevice.
    let create: Create = unsafe { std::mem::transmute(f) };
    let mut raw = std::ptr::null_mut();
    // SAFETY: sin adaptador (el predeterminado), sin niveles explícitos;
    // solo pedimos el dispositivo.
    unsafe {
        create(
            std::ptr::null_mut(),
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT.0,
            std::ptr::null(),
            0,
            D3D11_SDK_VERSION,
            &mut raw,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    }
    .ok()?;
    // SAFETY: referencia propia a un ID3D11Device; se pide su IDXGIDevice.
    let device = unsafe { IUnknown::from_raw(raw) };
    device.cast()
}

// ---------------------------------------------------------------------------
// Camino 1: contornos → ShapeVisual (sin D3D)
// ---------------------------------------------------------------------------

/// Expone una geometría de D2D como `IGeometrySource2D` para `CompositionPath`.
#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct GeometrySource(ID2D1Geometry);

impl windows::Graphics::IGeometrySource2D_Impl for GeometrySource_Impl {}

impl IGeometrySource2DInterop_Impl for GeometrySource_Impl {
    fn GetGeometry(&self) -> windows::core::Result<ID2D1Geometry> {
        Ok(self.0.clone())
    }

    fn TryGetGeometryUsingFactory(
        &self,
        _factory: windows::core::Ref<'_, ID2D1Factory>,
    ) -> windows::core::Result<ID2D1Geometry> {
        Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_NOTIMPL))
    }
}

struct Font {
    face: IDWriteFontFace,
    units_per_em: f32,
    ascent: f32,
}

fn font(dwrite: &IDWriteFactory, weight: DWRITE_FONT_WEIGHT) -> windows::core::Result<Font> {
    let mut collection = None;
    // SAFETY: llamadas DirectWrite estándar con salidas locales.
    unsafe {
        dwrite.GetSystemFontCollection(&mut collection, false)?;
        let collection = collection.ok_or(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_FAIL,
        ))?;
        let mut index = 0u32;
        let mut exists = windows::core::BOOL::default();
        collection.FindFamilyName(w!("Segoe UI"), &mut index, &mut exists)?;
        let family = collection.GetFontFamily(if exists.as_bool() { index } else { 0 })?;
        let (stretch, style) = (DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL);
        let font = family.GetFirstMatchingFont(weight, stretch, style)?;
        let face = font.CreateFontFace()?;
        let mut m = DWRITE_FONT_METRICS::default();
        face.GetMetrics(&mut m);
        Ok(Font { face, units_per_em: f32::from(m.designUnitsPerEm), ascent: f32::from(m.ascent) })
    }
}

/// Contorno de `text` en tamaño `em`, con la línea base en y = ascenso.
fn text_geometry(
    d2d: &ID2D1Factory,
    font: &Font,
    text: &str,
    em: f32,
) -> windows::core::Result<(ID2D1Geometry, f32)> {
    let codepoints: Vec<u32> = text.chars().map(u32::from).collect();
    let n = codepoints.len();
    let mut glyphs = vec![0u16; n];
    let mut metrics = vec![DWRITE_GLYPH_METRICS::default(); n];
    // SAFETY: buffers locales del largo indicado.
    unsafe {
        font.face.GetGlyphIndices(codepoints.as_ptr(), n as u32, glyphs.as_mut_ptr())?;
        font.face.GetDesignGlyphMetrics(glyphs.as_ptr(), n as u32, metrics.as_mut_ptr(), false)?;
    }
    let scale = em / font.units_per_em;
    let advances: Vec<f32> = metrics.iter().map(|m| m.advanceWidth as f32 * scale).collect();
    let width: f32 = advances.iter().sum();

    // SAFETY: geometría propia; el sink se cierra antes de usarla.
    let path = unsafe { d2d.CreatePathGeometry() }?;
    // SAFETY: ídem.
    unsafe {
        let sink = path.Open()?;
        font.face.GetGlyphRunOutline(
            em,
            glyphs.as_ptr(),
            Some(advances.as_ptr()),
            None,
            n as u32,
            false,
            false,
            &sink,
        )?;
        sink.Close()?;
    }
    // Los contornos salen con la línea base en y = 0 (hacia arriba negativo).
    let baseline = font.ascent * scale;
    let moved = Matrix3x2::translation(0.0, baseline);
    // SAFETY: geometría válida y matriz local.
    let geometry = unsafe { d2d.CreateTransformedGeometry(&path, &moved) }?;
    Ok((geometry.cast()?, width))
}

fn shape_text(
    compositor: &Compositor,
    shapes: &windows::UI::Composition::CompositionShapeCollection,
    geometry: ID2D1Geometry,
    brush: &CompositionColorBrush,
    x: f32,
    y: f32,
) -> windows::core::Result<()> {
    let source: IGeometrySource2D = GeometrySource(geometry).into();
    let path = CompositionPath::Create(&source)?;
    let geom = compositor.CreatePathGeometryWithPath(&path)?;
    let shape = compositor.CreateSpriteShapeWithGeometry(&geom)?;
    shape.SetFillBrush(brush)?;
    shape.SetOffset(Vector2 { X: x, Y: y })?;
    shapes.Append(&shape)?;
    Ok(())
}

fn build_shapes(
    compositor: &Compositor,
    container: &ContainerVisual,
    keep: &mut Vec<IUnknown>,
) -> windows::core::Result<()> {
    let dwrite = dwrite_factory()?;
    let d2d: ID2D1Factory = d2d_factory()?;
    let bold = font(&dwrite, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
    let regular = font(&dwrite, DWRITE_FONT_WEIGHT_NORMAL)?;

    let (w, h) = map_size();
    let visual = compositor.CreateShapeVisual()?;
    visual.SetSize(Vector2 { X: w, Y: h })?;
    let shapes = visual.Shapes()?;
    let color = |a, r, g, b| compositor.CreateColorBrushWithColor(Color { A: a, R: r, G: g, B: b });
    let key_brush = color(230, 32, 32, 36)?;
    let text_brush = color(255, 240, 240, 240)?;
    let name_brush = color(255, 150, 160, 175)?;
    for (x, y, letter, app) in keys() {
        let rect = compositor.CreateRoundedRectangleGeometry()?;
        rect.SetSize(Vector2 { X: KEY, Y: KEY })?;
        rect.SetCornerRadius(Vector2 { X: 10.0, Y: 10.0 })?;
        let key = compositor.CreateSpriteShapeWithGeometry(&rect)?;
        key.SetFillBrush(&key_brush)?;
        key.SetOffset(Vector2 { X: x, Y: y })?;
        shapes.Append(&key)?;

        let (g, lw) = text_geometry(&d2d, &bold, &letter.to_string(), LETTER_EM)?;
        shape_text(compositor, &shapes, g, &text_brush, x + (KEY - lw) / 2.0, y + 8.0)?;
        let (g, nw) = text_geometry(&d2d, &regular, app, NAME_EM)?;
        shape_text(compositor, &shapes, g, &name_brush, x + (KEY - nw) / 2.0, y + KEY - 26.0)?;
    }
    container.Children()?.InsertAtTop(&visual)?;
    keep.push(visual.cast()?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Caminos 2 y 3: D2D en una superficie de Composition
// ---------------------------------------------------------------------------

fn build_d2d(
    compositor: &Compositor,
    container: &ContainerVisual,
    driver: D3D_DRIVER_TYPE,
    keep: &mut Vec<IUnknown>,
) -> windows::core::Result<()> {
    let dxgi = d3d11_device(driver)?;
    let d2d: ID2D1Factory1 = d2d_factory()?;
    // SAFETY: dispositivo DXGI válido.
    let d2d_device = unsafe { d2d.CreateDevice(&dxgi) }?;
    let interop: ICompositorInterop = compositor.cast()?;
    // SAFETY: dispositivo D2D válido.
    let graphics = unsafe { interop.CreateGraphicsDevice(&d2d_device) }?;
    let (w, h) = map_size();
    let surface = graphics.CreateDrawingSurface(
        Size { Width: w, Height: h },
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        DirectXAlphaMode::Premultiplied,
    )?;

    let dwrite = dwrite_factory()?;
    // SAFETY: parámetros válidos; cadenas estáticas.
    let (letter_format, name_format) = unsafe {
        (
            dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                LETTER_EM,
                w!("es"),
            )?,
            dwrite.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                NAME_EM,
                w!("es"),
            )?,
        )
    };
    // SAFETY: formatos válidos.
    unsafe {
        use windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER;
        letter_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        name_format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
    }

    let draw: ICompositionDrawingSurfaceInterop = surface.cast()?;
    let mut offset = POINT::default();
    // SAFETY: BeginDraw/EndDraw emparejados; todo se dibuja en el contexto devuelto.
    unsafe {
        let ctx: ID2D1DeviceContext = draw.BeginDraw(None, &mut offset)?;
        ctx.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
        ctx.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        let color = |r: f32, g: f32, b: f32, a: f32| D2D1_COLOR_F { r, g, b, a };
        let key_brush = ctx.CreateSolidColorBrush(&color(0.125, 0.125, 0.14, 0.9), None)?;
        let text_brush = ctx.CreateSolidColorBrush(&color(0.94, 0.94, 0.94, 1.0), None)?;
        let name_brush = ctx.CreateSolidColorBrush(&color(0.59, 0.63, 0.69, 1.0), None)?;
        for (x, y, letter, app) in keys() {
            let rect = D2D_RECT_F { left: x, top: y, right: x + KEY, bottom: y + KEY };
            ctx.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT { rect, radiusX: 10.0, radiusY: 10.0 },
                &key_brush,
            );
            let l: Vec<u16> = letter.to_string().encode_utf16().collect();
            let top = D2D_RECT_F { left: x, top: y + 8.0, right: x + KEY, bottom: y + 60.0 };
            ctx.DrawText(
                &l,
                &letter_format,
                &top,
                &text_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
            let n: Vec<u16> = app.encode_utf16().collect();
            let (top, right, bottom) = (y + KEY - 28.0, x + KEY, y + KEY);
            let bottom = D2D_RECT_F { left: x, top, right, bottom };
            ctx.DrawText(
                &n,
                &name_format,
                &bottom,
                &name_brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        draw.EndDraw()?;
    }

    let brush = compositor.CreateSurfaceBrushWithSurface(&surface)?;
    let sprite = compositor.CreateSpriteVisual()?;
    sprite.SetSize(Vector2 { X: w, Y: h })?;
    sprite.SetBrush(&brush)?;
    container.Children()?.InsertAtTop(&sprite)?;
    keep.push(sprite.cast()?);
    keep.push(graphics.cast()?);
    keep.push(d2d_device.cast()?);
    keep.push(dxgi.cast()?);
    let _ = PCWSTR::null();
    Ok(())
}
