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
//!
//! v1.2 (plan-v1.2.md §8): lo que Geist no tiene (emoji, chino, coreano,
//! símbolos) sale de fuentes de Windows abiertas por archivo recién cuando
//! hacen falta ([`FALLBACK_FILES`]); antes salían cuadraditos.

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

/// Tamaño del mapa real (hito 4): la fila 1–4 arriba y las letras corridas
/// media tecla, como en un teclado.
fn view_size() -> (f32, f32) {
    (10.0 * KEY + 9.0 * GAP + (KEY + GAP) / 2.0, 4.0 * KEY + 3.0 * GAP)
}

/// Posiciones del mapa real: primero las cuatro teclas 1–4, después las
/// letras en el orden de [`ROWS`].
fn view_keys() -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = (0..4).map(|c| (c as f32 * (KEY + GAP), 0.0)).collect();
    for (r, row) in ROWS.iter().enumerate() {
        let indent = (r + 1) as f32 * (KEY + GAP) / 2.0;
        for c in 0..row.len() {
            out.push((indent + c as f32 * (KEY + GAP), (r + 1) as f32 * (KEY + GAP)));
        }
    }
    out
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

#[derive(Clone)]
struct Font {
    face: IDWriteFontFace,
    units_per_em: f32,
    ascent: f32,
}

// ---------------------------------------------------------------------------
// Respaldo de glifos (v1.2)
// ---------------------------------------------------------------------------

/// Las fuentes de respaldo, en orden, de `%WINDIR%\Fonts`: Segoe UI (latín
/// extendido, griego, cirílico), símbolos, emoji (en contorno: un color),
/// chino y coreano.
const FALLBACK_FILES: [&str; 5] =
    ["segoeui.ttf", "seguisym.ttf", "seguiemj.ttf", "msyh.ttc", "malgun.ttf"];

/// Caras de respaldo ya abiertas (`None` sin probar, `Some(None)` si falló),
/// del hilo de los overlays.
struct Fallback {
    factory: Option<IDWriteFactory5>,
    faces: [Option<Option<Font>>; FALLBACK_FILES.len()],
}

thread_local! {
    static FALLBACK: std::cell::RefCell<Fallback> =
        const { std::cell::RefCell::new(Fallback { factory: None, faces: [const { None }; 5] }) };
}

/// La cara de respaldo `k`, abriéndola la primera vez.
fn fallback_face(k: usize) -> Option<Font> {
    FALLBACK.with(|f| {
        let mut f = f.borrow_mut();
        if let Some(face) = &f.faces[k] {
            return face.clone();
        }
        if f.factory.is_none() {
            f.factory = dwrite_factory().ok();
        }
        let face = f.factory.as_ref().and_then(|factory| open_face(factory, FALLBACK_FILES[k]));
        f.faces[k] = Some(face.clone());
        face
    })
}

fn open_face(factory: &IDWriteFactory5, file: &str) -> Option<Font> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Graphics::DirectWrite::{DWRITE_FONT_FACE_TYPE, DWRITE_FONT_FILE_TYPE};
    let path = std::path::PathBuf::from(std::env::var_os("WINDIR")?).join("Fonts").join(file);
    if !path.exists() {
        return None;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: ruta terminada en cero que vive durante la llamada; salidas
    // locales. DirectWrite mapea el archivo: solo se leen las páginas usadas.
    unsafe {
        let f = factory.CreateFontFileReference(PCWSTR(wide.as_ptr()), None).ok()?;
        let mut supported = windows::core::BOOL(0);
        let mut file_type = DWRITE_FONT_FILE_TYPE::default();
        let mut face_type = DWRITE_FONT_FACE_TYPE::default();
        let mut faces = 0u32;
        f.Analyze(&mut supported, &mut file_type, Some(&mut face_type), &mut faces).ok()?;
        if !supported.as_bool() || faces == 0 {
            return None;
        }
        let face =
            factory.CreateFontFace(face_type, &[Some(f)], 0, DWRITE_FONT_SIMULATIONS_NONE).ok()?;
        let mut m = DWRITE_FONT_METRICS::default();
        face.GetMetrics(&mut m);
        Some(Font {
            face,
            units_per_em: f32::from(m.designUnitsPerEm),
            ascent: f32::from(m.ascent),
        })
    }
}

/// Lo que no se dibuja ni con respaldo: controles, uniones y selectores
/// (ZWJ, variaciones, tonos de piel: sin *shaping* saldrían sueltos) y las
/// escrituras que necesitan *shaping* o van de derecha a izquierda (hebreo,
/// árabe, índicas, tailandés…): letras sueltas serían peor que nada.
fn skip(c: char) -> bool {
    c.is_control()
        || matches!(u32::from(c),
            0x0590..=0x08FF        // hebreo, árabe, siríaco, tāna, n'ko…
            | 0x0900..=0x0DFF      // índicas
            | 0x0E00..=0x0FFF      // tailandés, lao, tibetano
            | 0x1000..=0x109F      // birmano
            | 0x1100..=0x11FF      // jamo (las sílabas hechas sí van)
            | 0x1780..=0x18AF      // jemer, mongol
            | 0x200B..=0x200F      // espacios de ancho cero, ZWJ, marcas
            | 0x202A..=0x202E      // incrustaciones bidi
            | 0x2060..=0x206F
            | 0xFB1D..=0xFDFF      // formas de presentación
            | 0xFE00..=0xFE0F      // selectores de variación
            | 0xFE70..=0xFEFF
            | 0x1F3FB..=0x1F3FF    // tonos de piel
            | 0xE0000..=0xE01EF)
}

/// Las caras de respaldo para `c`, en orden (índices en [`FALLBACK_FILES`]):
/// así un título en chino abre solo la fuente china, y un emoji prueba
/// primero Segoe UI Symbol (contorno, más liviana) y la de emoji después.
fn candidates(c: char) -> &'static [usize] {
    match u32::from(c) {
        0xAC00..=0xD7AF | 0x3130..=0x318F => &[4, 3],
        0x2E80..=0x2FDF
        | 0x3000..=0x312F
        | 0x31A0..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFFEF
        | 0x20000..=0x3FFFF => &[3, 4],
        0x2600..=0x27BF | 0x2B00..=0x2BFF | 0x1F000..=0x1FAFF => &[1, 2],
        _ => &[0, 1],
    }
}

/// Un tramo de `text` con una sola cara: sus glifos y avances (a `em`) y
/// dónde empieza.
struct Run {
    font: Font,
    glyphs: Vec<u16>,
    advances: Vec<f32>,
    x: f32,
}

/// Índices de glifo de `codepoints` en `font` (0: no lo tiene).
fn glyph_indices(font: &Font, codepoints: &[u32]) -> windows::core::Result<Vec<u16>> {
    let mut glyphs = vec![0u16; codepoints.len()];
    if !codepoints.is_empty() {
        // SAFETY: buffers locales del largo indicado.
        unsafe {
            font.face.GetGlyphIndices(
                codepoints.as_ptr(),
                codepoints.len() as u32,
                glyphs.as_mut_ptr(),
            )?;
        }
    }
    Ok(glyphs)
}

/// Avances de `glyphs` de `font` a tamaño `em`.
fn glyph_advances(font: &Font, glyphs: &[u16], em: f32) -> windows::core::Result<Vec<f32>> {
    let mut metrics = vec![DWRITE_GLYPH_METRICS::default(); glyphs.len()];
    if !glyphs.is_empty() {
        // SAFETY: buffers locales del largo indicado.
        unsafe {
            font.face.GetDesignGlyphMetrics(
                glyphs.as_ptr(),
                glyphs.len() as u32,
                metrics.as_mut_ptr(),
                false,
            )?;
        }
    }
    let scale = em / font.units_per_em;
    Ok(metrics.iter().map(|m| m.advanceWidth as f32 * scale).collect())
}

/// Un carácter, la cara que lo dibuja (0: la pedida; k + 1: el respaldo k)
/// con su glifo, y su avance.
type Resolved = (char, Option<(usize, u16)>, f32);

/// Cada carácter de `text` con la cara que lo dibuja (`None`: no se
/// dibuja), su glifo y su avance a `em`. Si `font` los tiene todos, no se
/// abre ningún respaldo (el camino de siempre).
fn resolve_chars(
    font: &Font,
    text: &str,
    em: f32,
) -> windows::core::Result<Vec<Resolved>> {
    let chars: Vec<char> = text.chars().collect();
    let codepoints: Vec<u32> = chars.iter().map(|&c| u32::from(c)).collect();
    let glyphs = glyph_indices(font, &codepoints)?;
    let mut out: Vec<(char, Option<(usize, u16)>)> = Vec::with_capacity(chars.len());
    for (&c, &g) in chars.iter().zip(&glyphs) {
        let face = if g != 0 {
            Some((0, g))
        } else if skip(c) {
            None
        } else {
            candidates(c).iter().find_map(|&k| {
                let f = fallback_face(k)?;
                let g = glyph_indices(&f, &[u32::from(c)]).ok()?[0];
                (g != 0).then_some((k + 1, g))
            })
        };
        out.push((c, face));
    }
    // Avances por cara, en un pedido por cara.
    let mut adv = vec![0.0f32; out.len()];
    for k in 0..=FALLBACK_FILES.len() {
        let idx: Vec<usize> =
            (0..out.len()).filter(|&i| out[i].1.is_some_and(|(f, _)| f == k)).collect();
        if idx.is_empty() {
            continue;
        }
        let f = if k == 0 { font.clone() } else { fallback_face(k - 1).expect("ya abierta") };
        let gs: Vec<u16> = idx.iter().map(|&i| out[i].1.expect("filtrada").1).collect();
        for (i, a) in idx.into_iter().zip(glyph_advances(&f, &gs, em)?) {
            adv[i] = a;
        }
    }
    Ok(out.into_iter().zip(adv).map(|((c, f), a)| (c, f, a)).collect())
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
    let mut runs: Vec<Run> = Vec::new();
    let mut width = 0.0;
    let mut last = None;
    for (_, face, a) in resolve_chars(font, text, em)? {
        let Some((k, g)) = face else { continue };
        if last != Some(k) {
            let f = if k == 0 { font.clone() } else { fallback_face(k - 1).expect("ya abierta") };
            runs.push(Run { font: f, glyphs: Vec::new(), advances: Vec::new(), x: width });
            last = Some(k);
        }
        let run = runs.last_mut().expect("recién agregado");
        run.glyphs.push(g);
        run.advances.push(a);
        width += a;
    }

    // SAFETY: geometría propia; el sink se cierra antes de usarla.
    let path = unsafe { d2d.CreatePathGeometry() }?;
    // SAFETY: ídem. Cada tramo se escribe en el mismo sink, corrido a su x.
    unsafe {
        let sink = path.Open()?;
        for run in &runs {
            let n = run.glyphs.len();
            let offsets = vec![
                windows::Win32::Graphics::DirectWrite::DWRITE_GLYPH_OFFSET {
                    advanceOffset: run.x,
                    ascenderOffset: 0.0,
                };
                n
            ];
            run.font.face.GetGlyphRunOutline(
                em,
                run.glyphs.as_ptr(),
                Some(run.advances.as_ptr()),
                Some(offsets.as_ptr()),
                n as u32,
                false,
                false,
                &sink,
            )?;
        }
        sink.Close()?;
    }
    // Los contornos salen con la línea base en y = 0 (hacia arriba negativo).
    let baseline = font.ascent * (em / font.units_per_em);
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
    let (visual, _) = shape_keys(compositor, &items, None, map_size())?;
    container.Children()?.InsertAtTop(&visual)?;
    keep.push(visual.cast()?);
    Ok(())
}

use wm_model::mapview::Tone;

/// Tokens oscuros de design.md §3.2 (el mapa es siempre oscuro).
pub(crate) mod tokens {
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
    /// `fill-strong` (10 %): la fila elegida de la lista.
    pub const FILL_STRONG: Color = c(0x1A, 0xFF, 0xFF, 0xFF);
}

/// Margen del panel alrededor de las teclas, y alto del pie.
const PAD: f32 = 24.0;
const FOOTER: f32 = 36.0;
const FOOTER_EM: f32 = 15.0;

/// Las teclas de una capa ya dibujadas: su fondo, para que la recién usada
/// destelle en `accent` ([`MapVisual::light`]).
pub struct MapVisual {
    pub container: ContainerVisual,
    keys: Vec<(char, windows::UI::Composition::CompositionSpriteShape)>,
    key_brush: CompositionColorBrush,
    lit: Option<char>,
}

/// El destello: `accent` firme un momento y vuelve a `surface` (lo anima
/// DWM). Antes quedaba en `accent` hasta la tecla siguiente y parecía
/// trabada (prueba del usuario).
const FLASH: std::time::Duration = std::time::Duration::from_millis(450);
const FLASH_HOLD: f32 = 0.35;

impl MapVisual {
    /// Hace destellar `letter` en `accent`. `None`: ninguna (y corta un
    /// destello en curso).
    pub fn light(&mut self, letter: Option<char>) -> windows::core::Result<()> {
        for (l, shape) in &self.keys {
            if Some(*l) == self.lit && Some(*l) != letter {
                shape.SetFillBrush(&self.key_brush)?;
            }
            if Some(*l) == letter {
                let c = self.key_brush.Compositor()?;
                let brush = c.CreateColorBrushWithColor(tokens::ACCENT)?;
                let anim = c.CreateColorKeyFrameAnimation()?;
                anim.InsertKeyFrame(0.0, tokens::ACCENT)?;
                anim.InsertKeyFrame(FLASH_HOLD, tokens::ACCENT)?;
                anim.InsertKeyFrame(1.0, tokens::SURFACE)?;
                anim.SetDuration(windows::Foundation::TimeSpan {
                    Duration: (FLASH.as_nanos() / 100) as i64,
                })?;
                shape.SetFillBrush(&brush)?;
                brush.StartAnimation(&windows::core::HSTRING::from("Color"), &anim)?;
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
    (w, h): (f32, f32),
) -> windows::core::Result<(windows::UI::Composition::ShapeVisual, KeyShapes)> {
    let geist = Geist::load()?;
    let d2d: ID2D1Factory = d2d_factory()?;
    let bold = font(&geist, &geist.semibold)?;
    let regular = font(&geist, &geist.regular)?;

    let visual = compositor.CreateShapeVisual()?;
    visual.SetSize(Vector2 { X: w, Y: h + if footer.is_some() { FOOTER } else { 0.0 } })?;
    let shapes = visual.Shapes()?;
    let brush = |c: Color| compositor.CreateColorBrushWithColor(c);
    let key_brush = brush(tokens::SURFACE)?;
    let border_brush = brush(tokens::BORDER)?;
    let text = brush(tokens::TEXT)?;
    let text_2 = brush(tokens::TEXT_2)?;
    let text_3 = brush(tokens::TEXT_3)?;
    let accent = brush(tokens::ACCENT)?;
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

        let letter_brush = match tone {
            Tone::Dim => &text_3,
            Tone::Accent => &accent,
            _ => &text,
        };
        let (g, lw) = text_geometry(&d2d, &bold, &letter.to_string(), LETTER_EM)?;
        shape_text(compositor, &shapes, g, letter_brush, x + (KEY - lw) / 2.0, y + 8.0)?;
        if *tone != Tone::Dim && !label.is_empty() {
            let label_brush = match tone {
                Tone::Open => &text,
                Tone::Accent => &accent,
                _ => &text_2,
            };
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
    dpi: u32,
    view: &wm_model::mapview::MapView,
) -> windows::core::Result<MapVisual> {
    let (kw, kh) = view_size();
    let (w, h) = (kw + 2.0 * PAD, kh + FOOTER + 2.0 * PAD);
    let container = compositor.CreateContainerVisual()?;
    container.SetSize(Vector2 { X: w, Y: h })?;
    // Tamaño según la escala del monitor (las medidas están a 96 DPI); si no
    // entra en el 90 % del monitor, se achica entero.
    let dpi_scale = dpi.max(96) as f32 / 96.0;
    let scale = (area_w * 0.9 / w).min(area_h * 0.9 / h).min(dpi_scale);
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
        .digits
        .iter()
        .chain(&view.cells)
        .zip(view_keys())
        .map(|(c, (x, y))| (x, y, c.letter, c.label.clone(), c.tone))
        .collect();
    let (keys_visual, key_shapes) =
        shape_keys(compositor, &items, Some(&view.footer), view_size())?;
    keys_visual.SetOffset(Vector3 { X: PAD, Y: PAD, Z: 0.0 })?;
    container.Children()?.InsertAtTop(&keys_visual)?;
    parent.Children()?.InsertAtTop(&container)?;
    Ok(MapVisual {
        container,
        keys: key_shapes,
        key_brush: compositor.CreateColorBrushWithColor(tokens::SURFACE)?,
        lit: None,
    })
}

/// Un aviso corto (una tecla no hizo nada): una píldora con el texto, abajo
/// al centro de un área de `area_w` × `area_h`, a la escala del monitor.
/// Aparece con un fundido, se queda y se va sola (lo anima DWM): no hace
/// falta que nadie la esconda para que deje de verse.
pub fn build_toast(
    compositor: &Compositor,
    parent: &ContainerVisual,
    area_w: f32,
    area_h: f32,
    dpi: u32,
    text: &str,
    total: std::time::Duration,
) -> windows::core::Result<ContainerVisual> {
    const EM: f32 = 16.0;
    const PAD_X: f32 = 18.0;
    const H: f32 = 40.0;
    let geist = Geist::load()?;
    let d2d: ID2D1Factory = d2d_factory()?;
    let regular = font(&geist, &geist.regular)?;
    let (g, tw) = text_geometry(&d2d, &regular, text, EM)?;
    let (w, h) = (tw + 2.0 * PAD_X, H);
    let container = compositor.CreateContainerVisual()?;
    container.SetSize(Vector2 { X: w, Y: h })?;
    let scale = (dpi.max(96) as f32 / 96.0).min(area_w * 0.9 / w);
    container.SetScale(Vector3 { X: scale, Y: scale, Z: 1.0 })?;
    let (sw, sh) = (w * scale, h * scale);
    container.SetOffset(Vector3 { X: (area_w - sw) / 2.0, Y: area_h * 0.88 - sh, Z: 0.0 })?;

    let visual = compositor.CreateShapeVisual()?;
    visual.SetSize(Vector2 { X: w, Y: h })?;
    let shapes = visual.Shapes()?;
    let rect = compositor.CreateRoundedRectangleGeometry()?;
    rect.SetSize(Vector2 { X: w, Y: h })?;
    rect.SetCornerRadius(Vector2 { X: h / 2.0, Y: h / 2.0 })?;
    let bg = compositor.CreateSpriteShapeWithGeometry(&rect)?;
    bg.SetFillBrush(&compositor.CreateColorBrushWithColor(tokens::PANEL)?)?;
    bg.SetStrokeBrush(&compositor.CreateColorBrushWithColor(tokens::BORDER)?)?;
    bg.SetStrokeThickness(1.0)?;
    shapes.Append(&bg)?;
    let text_brush = compositor.CreateColorBrushWithColor(tokens::TEXT)?;
    // Centrado vertical aproximado: la caja de Geist a 16 px mide ~19 px.
    shape_text(compositor, &shapes, g, &text_brush, PAD_X, (h - EM * 1.2) / 2.0)?;
    container.Children()?.InsertAtTop(&visual)?;

    // Opacidad: entra en 120 ms, se queda, sale en 300 ms.
    let total_ms = total.as_millis().max(800) as f32;
    let anim = compositor.CreateScalarKeyFrameAnimation()?;
    anim.InsertKeyFrame(0.0, 0.0)?;
    anim.InsertKeyFrame(120.0 / total_ms, 1.0)?;
    anim.InsertKeyFrame(1.0 - 300.0 / total_ms, 1.0)?;
    anim.InsertKeyFrame(1.0, 0.0)?;
    anim.SetDuration(windows::Foundation::TimeSpan { Duration: (total.as_nanos() / 100) as i64 })?;
    container.StartAnimation(&windows::core::HSTRING::from("Opacity"), &anim)?;
    parent.Children()?.InsertAtTop(&container)?;
    Ok(container)
}

/// Avance de cada carácter de `text` a tamaño `em` y si se dibuja (con
/// Geist o con un respaldo).
fn advances(font: &Font, text: &str, em: f32) -> windows::core::Result<Vec<(char, f32, bool)>> {
    Ok(resolve_chars(font, text, em)?.into_iter().map(|(c, f, a)| (c, a, f.is_some())).collect())
}

/// `text` sin los caracteres que no se pueden dibujar (ni Geist ni los
/// respaldos los tienen) y cortado con "…" para que entre en `max_w`.
fn fit(font: &Font, text: &str, em: f32, max_w: f32) -> windows::core::Result<String> {
    let all = advances(font, text, em)?;
    let kept: Vec<(char, f32)> =
        all.iter().filter(|(_, _, ok)| *ok).map(|&(c, w, _)| (c, w)).collect();
    let total: f32 = kept.iter().map(|(_, w)| w).sum();
    if total <= max_w {
        return Ok(kept.iter().map(|(c, _)| c).collect::<String>().trim().to_owned());
    }
    let ellipsis = advances(font, "…", em)?.first().map_or(em * 0.6, |a| a.1);
    let mut out = String::new();
    let mut w = 0.0;
    for (c, a) in kept {
        if w + a + ellipsis > max_w {
            break;
        }
        out.push(c);
        w += a;
    }
    Ok(out.trim_end().to_owned() + "…")
}

/// La lista dibujada: mover la selección es solo correr la píldora (sin
/// rearmar los textos: así no parpadea al apretar la letra seguido).
pub struct ListVisual {
    pub container: ContainerVisual,
    pill: windows::UI::Composition::CompositionRoundedRectangleGeometry,
    /// Arriba de cada fila (en `rows`).
    row_y: Vec<f32>,
    /// Dónde quedó el panel en la ventana del monitor y con qué escala
    /// (para saber qué fila hay bajo un clic).
    origin: (f32, f32),
    scale: f32,
}

impl ListVisual {
    /// La fila en `(x, y)`, relativo a la ventana del monitor (`None`:
    /// afuera del panel o en el encabezado o el pie).
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        const W: f32 = 640.0;
        const ROW: f32 = 36.0;
        let (lx, ly) = ((x - self.origin.0) / self.scale, (y - self.origin.1) / self.scale);
        if !(0.0..=W).contains(&lx) {
            return None;
        }
        self.row_y.iter().position(|&top| ly >= top && ly < top + ROW)
    }

    pub fn select(&self, i: usize) -> windows::core::Result<()> {
        let Some(&y) = self.row_y.get(i) else { return Ok(()) };
        self.pill.SetOffset(Vector2 { X: 8.0, Y: y + 2.0 })
    }
}

/// La lista de ventanas de una letra (plan-apps.md §8): panel con la
/// tecla, el nombre y la cantidad; una fila por ventana (título y, a la
/// derecha, datos chicos); la elegida en una píldora con anillo `accent`.
/// Centrada en `area`, escalada por DPI.
pub fn build_list(
    compositor: &Compositor,
    parent: &ContainerVisual,
    area_w: f32,
    area_h: f32,
    dpi: u32,
    view: &wm_model::listview::ListView,
) -> windows::core::Result<ListVisual> {
    const W: f32 = 640.0;
    const PAD: f32 = 16.0;
    const HEAD: f32 = 48.0;
    const ROW: f32 = 36.0;
    const FOOT: f32 = 36.0;
    const EM: f32 = 16.0;
    const SMALL: f32 = 13.0;
    let geist = Geist::load()?;
    let d2d: ID2D1Factory = d2d_factory()?;
    let regular = font(&geist, &geist.regular)?;
    let semibold = font(&geist, &geist.semibold)?;
    let more_rows = usize::from(view.above > 0) + usize::from(view.below > 0);
    let h = HEAD + (view.rows.len() + more_rows) as f32 * ROW + FOOT + PAD;

    let container = compositor.CreateContainerVisual()?;
    container.SetSize(Vector2 { X: W, Y: h })?;
    let scale = (dpi.max(96) as f32 / 96.0).min(area_w * 0.9 / W).min(area_h * 0.9 / h);
    container.SetScale(Vector3 { X: scale, Y: scale, Z: 1.0 })?;
    let (sw, sh) = (W * scale, h * scale);
    container.SetOffset(Vector3 { X: (area_w - sw) / 2.0, Y: (area_h - sh) / 2.0, Z: 0.0 })?;

    let visual = compositor.CreateShapeVisual()?;
    visual.SetSize(Vector2 { X: W, Y: h })?;
    let shapes = visual.Shapes()?;
    let rect = |x: f32, y: f32, w: f32, hh: f32, r: f32| -> windows::core::Result<_> {
        let g = compositor.CreateRoundedRectangleGeometry()?;
        g.SetOffset(Vector2 { X: x, Y: y })?;
        g.SetSize(Vector2 { X: w, Y: hh })?;
        g.SetCornerRadius(Vector2 { X: r, Y: r })?;
        compositor.CreateSpriteShapeWithGeometry(&g)
    };
    let panel = rect(0.0, 0.0, W, h, 14.0)?;
    panel.SetFillBrush(&compositor.CreateColorBrushWithColor(tokens::PANEL)?)?;
    panel.SetStrokeBrush(&compositor.CreateColorBrushWithColor(tokens::BORDER)?)?;
    panel.SetStrokeThickness(1.0)?;
    shapes.Append(&panel)?;
    // La píldora de la elegida, debajo de todos los textos.
    let pill_geom = compositor.CreateRoundedRectangleGeometry()?;
    pill_geom.SetSize(Vector2 { X: W - PAD, Y: ROW - 4.0 })?;
    pill_geom.SetCornerRadius(Vector2 { X: 8.0, Y: 8.0 })?;
    let pill = compositor.CreateSpriteShapeWithGeometry(&pill_geom)?;
    pill.SetFillBrush(&compositor.CreateColorBrushWithColor(tokens::FILL_STRONG)?)?;
    pill.SetStrokeBrush(&compositor.CreateColorBrushWithColor(tokens::ACCENT)?)?;
    pill.SetStrokeThickness(2.0)?;
    shapes.Append(&pill)?;
    let mut row_y = Vec::new();

    let text = compositor.CreateColorBrushWithColor(tokens::TEXT)?;
    let text2 = compositor.CreateColorBrushWithColor(tokens::TEXT_2)?;
    // Centrado vertical aproximado: la caja de Geist mide ~1,2 em.
    let line_y = |top: f32, rowh: f32, em: f32| top + (rowh - em * 1.2) / 2.0;

    // Encabezado: tecla, app y cantidad.
    let key = rect(PAD, (HEAD - 26.0) / 2.0 + 4.0, 26.0, 26.0, 6.0)?;
    key.SetStrokeBrush(&compositor.CreateColorBrushWithColor(tokens::BORDER)?)?;
    key.SetStrokeThickness(1.0)?;
    key.SetFillBrush(&compositor.CreateColorBrushWithColor(tokens::SURFACE)?)?;
    shapes.Append(&key)?;
    let (g, lw) = text_geometry(&d2d, &semibold, &view.letter.to_string(), SMALL)?;
    shape_text(compositor, &shapes, g, &text, PAD + (26.0 - lw) / 2.0, line_y(4.0, HEAD, SMALL))?;
    let count =
        if view.count == 1 { "1 ventana".to_owned() } else { format!("{} ventanas", view.count) };
    let (g, cw) = text_geometry(&d2d, &regular, &count, SMALL)?;
    shape_text(compositor, &shapes, g, &text2, W - PAD - cw, line_y(4.0, HEAD, SMALL))?;
    let app = fit(&semibold, &view.app, EM, W - 3.0 * PAD - 26.0 - cw - 12.0)?;
    let (g, _) = text_geometry(&d2d, &semibold, &app, EM)?;
    shape_text(compositor, &shapes, g, &text, PAD + 26.0 + 12.0, line_y(4.0, HEAD, EM))?;

    let mut y = HEAD;
    let more = |n: usize, y: f32| -> windows::core::Result<()> {
        let (g, _) = text_geometry(&d2d, &regular, &format!("… {n} más"), SMALL)?;
        shape_text(compositor, &shapes, g, &text2, PAD + 12.0, line_y(y, ROW, SMALL))
    };
    if view.above > 0 {
        more(view.above, y)?;
        y += ROW;
    }
    for row in &view.rows {
        row_y.push(y);
        let meta_w = if row.meta.is_empty() {
            0.0
        } else {
            let meta = fit(&regular, &row.meta, SMALL, W * 0.35)?;
            let (g, mw) = text_geometry(&d2d, &regular, &meta, SMALL)?;
            shape_text(compositor, &shapes, g, &text2, W - PAD - 12.0 - mw, line_y(y, ROW, SMALL))?;
            mw + 16.0
        };
        let title = fit(&regular, &row.title, EM, W - 2.0 * PAD - 24.0 - meta_w)?;
        let (g, _) = text_geometry(&d2d, &regular, &title, EM)?;
        shape_text(compositor, &shapes, g, &text, PAD + 12.0, line_y(y, ROW, EM))?;
        y += ROW;
    }
    if view.below > 0 {
        more(view.below, y)?;
        y += ROW;
    }
    let footer = fit(&regular, &view.footer, SMALL, W - 2.0 * PAD)?;
    let (g, fw) = text_geometry(&d2d, &regular, &footer, SMALL)?;
    shape_text(compositor, &shapes, g, &text2, (W - fw) / 2.0, line_y(y + PAD / 2.0, FOOT, SMALL))?;

    container.Children()?.InsertAtTop(&visual)?;
    let list = ListVisual {
        container,
        pill: pill_geom,
        row_y,
        origin: ((area_w - sw) / 2.0, (area_h - sh) / 2.0),
        scale,
    };
    list.select(view.selected)?;
    parent.Children()?.InsertAtTop(&list.container)?;
    Ok(list)
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

/// v1.2 (plan-v1.2.md §8), `wm overlay-demo fuentes`: cuánto cuesta armar un
/// texto latino (el camino de siempre) y textos que necesitan un respaldo,
/// cada uno en un hilo nuevo (en frío: abre sus fuentes) y en caliente, y
/// cuánta memoria suman.
pub fn font_bench() -> windows::core::Result<String> {
    use crate::overlay::process_memory;
    let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
    let case = |text: &'static str| {
        std::thread::spawn(move || -> windows::core::Result<String> {
            let geist = Geist::load()?;
            let d2d: ID2D1Factory = d2d_factory()?;
            let regular = font(&geist, &geist.regular)?;
            text_geometry(&d2d, &regular, "Brave", 16.0)?; // Geist ya cargada
            let (_, p0) = process_memory();
            let t = std::time::Instant::now();
            text_geometry(&d2d, &regular, text, 16.0)?;
            let cold = t.elapsed().as_secs_f64() * 1000.0;
            let (_, p1) = process_memory();
            let t = std::time::Instant::now();
            for _ in 0..200 {
                text_geometry(&d2d, &regular, text, 16.0)?;
            }
            let warm = t.elapsed().as_secs_f64() * 1000.0 / 200.0;
            let opened: Vec<&str> = FALLBACK_FILES
                .iter()
                .enumerate()
                .filter(|&(k, _)| FALLBACK.with(|f| matches!(f.borrow().faces[k], Some(Some(_)))))
                .map(|(_, n)| *n)
                .collect();
            let lost: String = resolve_chars(&regular, text, 16.0)?
                .into_iter()
                .filter(|(c, f, _)| f.is_none() && !c.is_whitespace())
                .map(|(c, _, _)| c)
                .collect();
            let ok = cold <= 3.0 && mb(p1.saturating_sub(p0)) <= 0.5;
            Ok(format!(
                "  {} «{text}»: {cold:.2} ms en frío, {warm:.3} ms en caliente, privada +{:.2} MB; \
                 abre: {}{}\n",
                if ok { "ok  " } else { "MAL " },
                mb(p1.saturating_sub(p0)),
                if opened.is_empty() { "nada".to_owned() } else { opened.join(", ") },
                if lost.is_empty() { String::new() } else { format!("; sin dibujar «{lost}»") },
            ))
        })
        .join()
        .unwrap_or_else(|_| Ok("  (el hilo se cayó)\n".to_owned()))
    };
    let mut out =
        String::from("Respaldo de glifos (compuerta: ≤ 3 ms en frío y ≤ 0,5 MB por texto):\n");
    for text in [
        "Brave — Bandeja de entrada (12) · Gmail",
        "Notas 📝 de la semana",
        "微信 — 聊天",
        "카카오톡",
        "Ωμέγα ★ ✓ → №",
        "Notas 📝 微信 카카오톡 ★ Ωμέγα ✓",
    ] {
        out.push_str(&case(text)?);
    }
    Ok(out)
}
