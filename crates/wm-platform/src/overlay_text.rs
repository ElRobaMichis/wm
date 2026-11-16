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
//!
//! La tipografía es Geist, la misma de los ajustes (design.md §3.1). Va
//! dentro de `wm.exe` y se carga desde memoria en una fábrica de DirectWrite
//! aislada: no depende de las fuentes instaladas ni toca la caché del sistema.

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
    DWRITE_FACTORY_TYPE_ISOLATED, DWRITE_FONT_FACE_TYPE_TRUETYPE, DWRITE_FONT_METRICS,
    DWRITE_FONT_SIMULATIONS_NONE, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_GLYPH_METRICS,
    DWRITE_MEASURING_MODE_NATURAL, IDWriteFactory5, IDWriteFontCollection, IDWriteFontFace,
    IDWriteFontFile,
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

use wm_model::mapview::ROWS;
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

/// Fábrica aislada: lo que se registra en ella (el cargador en memoria) se
/// libera con ella, y no se comparte la caché de fuentes del sistema.
fn dwrite_factory() -> windows::core::Result<IDWriteFactory5> {
    type Create = unsafe extern "system" fn(i32, *const GUID, *mut *mut c_void) -> HRESULT;
    let f = crate::dynlib::proc(w!("dwrite.dll"), s!("DWriteCreateFactory"))?;
    // SAFETY: firma documentada de DWriteCreateFactory.
    let create: Create = unsafe { std::mem::transmute(f) };
    let mut raw = std::ptr::null_mut();
    // SAFETY: IID válido y puntero de salida local.
    unsafe { create(DWRITE_FACTORY_TYPE_ISOLATED.0, &IDWriteFactory5::IID, &mut raw) }.ok()?;
    // SAFETY: referencia propia recién creada.
    Ok(unsafe { IDWriteFactory5::from_raw(raw) })
}

// ---------------------------------------------------------------------------
// Geist, desde memoria
// ---------------------------------------------------------------------------

static GEIST_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-Regular.ttf");
static GEIST_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/geist/Geist-SemiBold.ttf");

/// Los dos pesos de Geist como archivos de DirectWrite, en una fábrica propia.
struct Geist {
    factory: IDWriteFactory5,
    regular: IDWriteFontFile,
    semibold: IDWriteFontFile,
}

impl Geist {
    fn load() -> windows::core::Result<Self> {
        let factory = dwrite_factory()?;
        // SAFETY: el cargador se registra en la fábrica que lo usa; sin dueño,
        // DirectWrite copia los bytes y los suelta junto con la fábrica.
        unsafe {
            let loader = factory.CreateInMemoryFontFileLoader()?;
            factory.RegisterFontFileLoader(&loader)?;
            let file = |data: &'static [u8]| {
                loader.CreateInMemoryFontFileReference(
                    &factory,
                    data.as_ptr().cast(),
                    data.len() as u32,
                    None::<&IUnknown>,
                )
            };
            let (regular, semibold) = (file(GEIST_REGULAR)?, file(GEIST_SEMIBOLD)?);
            Ok(Self { factory, regular, semibold })
        }
    }

    /// Colección con los dos pesos, para `CreateTextFormat` (camino D2D).
    fn collection(&self) -> windows::core::Result<IDWriteFontCollection> {
        // SAFETY: archivos de esta misma fábrica.
        unsafe {
            let builder = self.factory.CreateFontSetBuilder()?;
            builder.AddFontFile(&self.regular)?;
            builder.AddFontFile(&self.semibold)?;
            let set = builder.CreateFontSet()?;
            self.factory.CreateFontCollectionFromFontSet(&set)?.cast()
        }
    }
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

/// Cara de fuente de un archivo de Geist (sin buscar por nombre).
fn font(geist: &Geist, file: &IDWriteFontFile) -> windows::core::Result<Font> {
    // SAFETY: archivo de la misma fábrica; salidas locales.
    unsafe {
        let face = geist.factory.CreateFontFace(
            DWRITE_FONT_FACE_TYPE_TRUETYPE,
            &[Some(file.clone())],
            0,
            DWRITE_FONT_SIMULATIONS_NONE,
        )?;
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
    let items: Vec<(f32, f32, char, String, Tone)> =
        keys().into_iter().map(|(x, y, l, app)| (x, y, l, app.to_owned(), Tone::Open)).collect();
    let (visual, _) = shape_keys(compositor, &items, None)?;
    container.Children()?.InsertAtTop(&visual)?;
    keep.push(visual.cast()?);
    Ok(())
}

use wm_model::mapview::Tone;

/// Tokens oscuros de design.md §3.2 (el mapa es siempre oscuro).
mod tokens {
    use windows::UI::Color;
    const fn c(a: u8, r: u8, g: u8, b: u8) -> Color {
        Color { A: a, R: r, G: g, B: b }
    }
    /// `bg`, apenas translúcido (más transparente, el texto de las ventanas
    /// de atrás se leía a través del mapa).
    pub const PANEL: Color = c(0xF8, 0x0E, 0x0E, 0x0F);
    pub const SURFACE: Color = c(0xFF, 0x16, 0x16, 0x18);
    /// `border` (7 %).
    pub const BORDER: Color = c(0x12, 0xFF, 0xFF, 0xFF);
    pub const TEXT: Color = c(0xFF, 0xED, 0xED, 0xEE);
    pub const TEXT_2: Color = c(0xFF, 0x8E, 0x8E, 0x93);
    pub const TEXT_3: Color = c(0xFF, 0x5A, 0x5A, 0x5F);
    pub const ACCENT: Color = c(0xFF, 0x5B, 0x8C, 0xFF);
}

/// Margen del panel alrededor de las teclas, y alto del pie.
const PAD: f32 = 24.0;
const FOOTER: f32 = 36.0;
const FOOTER_EM: f32 = 15.0;

/// Las teclas de una capa ya dibujadas: su fondo, para pintar la elegida en
/// `accent` ([`MapVisual::light`]).
pub struct MapVisual {
    pub container: ContainerVisual,
    keys: Vec<(char, windows::UI::Composition::CompositionSpriteShape)>,
    key_brush: CompositionColorBrush,
    accent_brush: CompositionColorBrush,
    lit: Option<char>,
}

impl MapVisual {
    /// Pinta `letter` en `accent` (y despinta la anterior). `None`: ninguna.
    pub fn light(&mut self, letter: Option<char>) -> windows::core::Result<()> {
        if self.lit == letter {
            return Ok(());
        }
        for (l, shape) in &self.keys {
            if Some(*l) == self.lit {
                shape.SetFillBrush(&self.key_brush)?;
            }
            if Some(*l) == letter {
                shape.SetFillBrush(&self.accent_brush)?;
            }
        }
        self.lit = letter;
        Ok(())
    }
}

type KeyShapes = Vec<(char, windows::UI::Composition::CompositionSpriteShape)>;

/// Dibuja las teclas `(x, y, letra, etiqueta, tono)` como formas (texto en
/// contornos, lo rasteriza DWM) y, si hay, el pie debajo. Devuelve el visual
/// y el fondo de cada tecla.
fn shape_keys(
    compositor: &Compositor,
    items: &[(f32, f32, char, String, Tone)],
    footer: Option<&str>,
) -> windows::core::Result<(windows::UI::Composition::ShapeVisual, KeyShapes)> {
    let geist = Geist::load()?;
    let d2d: ID2D1Factory = d2d_factory()?;
    let bold = font(&geist, &geist.semibold)?;
    let regular = font(&geist, &geist.regular)?;

    let (w, h) = map_size();
    let visual = compositor.CreateShapeVisual()?;
    visual.SetSize(Vector2 { X: w, Y: h + if footer.is_some() { FOOTER } else { 0.0 } })?;
    let shapes = visual.Shapes()?;
    let brush = |c: Color| compositor.CreateColorBrushWithColor(c);
    let key_brush = brush(tokens::SURFACE)?;
    let border_brush = brush(tokens::BORDER)?;
    let text = brush(tokens::TEXT)?;
    let text_2 = brush(tokens::TEXT_2)?;
    let text_3 = brush(tokens::TEXT_3)?;
    let mut key_shapes = Vec::new();
    for (x, y, letter, label, tone) in items {
        let (x, y) = (*x, *y);
        let rect = compositor.CreateRoundedRectangleGeometry()?;
        rect.SetSize(Vector2 { X: KEY, Y: KEY })?;
        rect.SetCornerRadius(Vector2 { X: 10.0, Y: 10.0 })?;
        let key = compositor.CreateSpriteShapeWithGeometry(&rect)?;
        key.SetFillBrush(&key_brush)?;
        key.SetStrokeBrush(&border_brush)?;
        key.SetStrokeThickness(1.0)?;
        key.SetOffset(Vector2 { X: x, Y: y })?;
        shapes.Append(&key)?;
        key_shapes.push((*letter, key));

        let letter_brush = if *tone == Tone::Dim { &text_3 } else { &text };
        let (g, lw) = text_geometry(&d2d, &bold, &letter.to_string(), LETTER_EM)?;
        shape_text(compositor, &shapes, g, letter_brush, x + (KEY - lw) / 2.0, y + 8.0)?;
        if *tone != Tone::Dim && !label.is_empty() {
            let label_brush = if *tone == Tone::Open { &text } else { &text_2 };
            let (mut g, mut nw) = text_geometry(&d2d, &regular, label, NAME_EM)?;
            // Si no entra en la tecla, más chica.
            let room = KEY - 10.0;
            if nw > room {
                (g, nw) = text_geometry(&d2d, &regular, label, NAME_EM * room / nw)?;
            }
            let nx = x + (KEY - nw) / 2.0;
            shape_text(compositor, &shapes, g, label_brush, nx, y + KEY - 26.0)?;
        }
    }
    if let Some(footer) = footer {
        let (g, fw) = text_geometry(&d2d, &regular, footer, FOOTER_EM)?;
        shape_text(compositor, &shapes, g, &text_2, (w - fw) / 2.0, h + 14.0)?;
    }
    Ok((visual, key_shapes))
}

/// El mapa de una capa: panel, teclas de `view` y pie, centrado en un área
/// de `area_w` × `area_h`, dentro de `parent`. Se muestra u oculta con
/// `SetIsVisible` sobre [`MapVisual::container`].
pub fn build_view(
    compositor: &Compositor,
    parent: &ContainerVisual,
    area_w: f32,
    area_h: f32,
    view: &wm_model::mapview::MapView,
) -> windows::core::Result<MapVisual> {
    let (kw, kh) = map_size();
    let (w, h) = (kw + 2.0 * PAD, kh + FOOTER + 2.0 * PAD);
    let container = compositor.CreateContainerVisual()?;
    container.SetSize(Vector2 { X: w, Y: h })?;
    // Si no entra en el 90 % del monitor, se achica entero.
    let scale = (area_w * 0.9 / w).min(area_h * 0.9 / h).min(1.0);
    container.SetScale(Vector3 { X: scale, Y: scale, Z: 1.0 })?;
    let (sw, sh) = (w * scale, h * scale);
    container.SetOffset(Vector3 { X: (area_w - sw) / 2.0, Y: (area_h - sh) / 2.0, Z: 0.0 })?;

    let panel = compositor.CreateShapeVisual()?;
    panel.SetSize(Vector2 { X: w, Y: h })?;
    let rect = compositor.CreateRoundedRectangleGeometry()?;
    rect.SetSize(Vector2 { X: w, Y: h })?;
    rect.SetCornerRadius(Vector2 { X: 14.0, Y: 14.0 })?;
    let bg = compositor.CreateSpriteShapeWithGeometry(&rect)?;
    bg.SetFillBrush(&compositor.CreateColorBrushWithColor(tokens::PANEL)?)?;
    bg.SetStrokeBrush(&compositor.CreateColorBrushWithColor(tokens::BORDER)?)?;
    bg.SetStrokeThickness(1.0)?;
    panel.Shapes()?.Append(&bg)?;
    container.Children()?.InsertAtTop(&panel)?;

    let items: Vec<(f32, f32, char, String, Tone)> = view
        .cells
        .iter()
        .zip(keys())
        .map(|(c, (x, y, _, _))| (x, y, c.letter, c.label.clone(), c.tone))
        .collect();
    let (keys_visual, key_shapes) = shape_keys(compositor, &items, Some(&view.footer))?;
    keys_visual.SetOffset(Vector3 { X: PAD, Y: PAD, Z: 0.0 })?;
    container.Children()?.InsertAtTop(&keys_visual)?;
    parent.Children()?.InsertAtTop(&container)?;
    Ok(MapVisual {
        container,
        keys: key_shapes,
        key_brush: compositor.CreateColorBrushWithColor(tokens::SURFACE)?,
        accent_brush: compositor.CreateColorBrushWithColor(tokens::ACCENT)?,
        lit: None,
    })
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

    let geist = Geist::load()?;
    let fonts = geist.collection()?;
    // SAFETY: parámetros válidos; cadenas estáticas.
    let (letter_format, name_format) = unsafe {
        (
            geist.factory.CreateTextFormat(
                w!("Geist"),
                &fonts,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                LETTER_EM,
                w!("es"),
            )?,
            geist.factory.CreateTextFormat(
                w!("Geist"),
                &fonts,
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
