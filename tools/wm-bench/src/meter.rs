//! Medidor de cuadros (spike 5, architecture.md §11): Desktop Duplication
//! sobre cada monitor. Por cada cuadro que DWM compone, copia una sola fila de
//! píxeles a una textura chica y busca ahí el cuadrado magenta de la demo.
//!
//! - Entrega un cuadro por present de DWM si el lector llega a tiempo; si no,
//!   los combina y lo avisa con `AccumulatedFrames > 1` ("salteados").
//! - Mide presents de DWM, no el scanout (que llega hasta 1 refresco después).
//! - Si Windows corta la duplicación (`DXGI_ERROR_ACCESS_LOST`, por ejemplo al
//!   cambiar la frecuencia), la vuelve a crear y sigue.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows::core::Interface;
use wm_model::Rect;
use wm_platform::clock;

/// Qué píxeles cuentan en la fila.
#[derive(Debug, Clone, Copy)]
pub enum ColorKey {
    /// Un color (BGR) con tolerancia por canal.
    Near { b: u8, g: u8, r: u8, tolerance: u8 },
    /// Todo lo que **no** es el telón magenta de `wm ghost-demo … curtain`.
    /// El magenta oscurecido por la sombra de una ventana sigue contando como
    /// telón (G bajo, R y B altos y parecidos).
    NotCurtain,
}

impl ColorKey {
    /// El cuadrado de la demo de overlays.
    pub const MAGENTA: ColorKey = ColorKey::Near { b: 255, g: 0, r: 255, tolerance: 70 };

    fn matches(&self, p: &[u8; 4]) -> bool {
        match *self {
            ColorKey::Near { b, g, r, tolerance } => {
                let near = |a: u8, c: u8| a.abs_diff(c) <= tolerance;
                near(p[0], b) && near(p[1], g) && near(p[2], r)
            }
            ColorKey::NotCurtain => {
                let curtain = p[1] < 40 && p[0] > 100 && p[2] > 100 && p[0].abs_diff(p[2]) < 30;
                !curtain
            }
        }
    }
}

/// Un cuadro entregado por Desktop Duplication.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// `LastPresentTime` (QPC) del cuadro.
    pub qpc: i64,
    /// Centro del tramo más largo del color en la fila medida, si se ve.
    pub x: Option<u32>,
    /// Píxeles de la fila con el color (para detectar "sin ventana" y
    /// "doble imagen").
    pub covered: u32,
    /// Cuadros compuestos desde el anterior que entregamos (> 1 = salteados).
    pub accumulated: u32,
}

pub struct Measured {
    pub samples: Vec<Sample>,
    /// Veces que hubo que recrear la duplicación.
    pub lost: u32,
}

/// Medidor de un monitor.
pub struct OutputMeter {
    pub name: String,
    pub desktop: Rect,
    pub refresh_hz: f64,
    handle: JoinHandle<Result<Measured, String>>,
}

impl OutputMeter {
    pub fn finish(self) -> Result<Measured, String> {
        self.handle.join().map_err(|_| "el hilo del medidor entró en pánico".to_owned())?
    }
}

/// Arranca un medidor por monitor. `row` es la fila (relativa al monitor)
/// donde pasa el cuadrado. Miden hasta que `stop` sea `true`.
pub fn start(
    row: u32,
    key: ColorKey,
    stop: Arc<AtomicBool>,
) -> Result<Vec<OutputMeter>, String> {
    let mut meters = Vec::new();
    for (a, o, name, desktop) in outputs()? {
        let stop = Arc::clone(&stop);
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::Builder::new()
            .name(format!("meter-{name}"))
            .spawn(move || measure(a, o, row, key, &stop, &tx))
            .map_err(|e| e.to_string())?;
        let refresh_hz = rx.recv().map_err(|_| format!("{name}: no arrancó"))?;
        meters.push(OutputMeter { name, desktop, refresh_hz, handle });
    }
    Ok(meters)
}

fn outputs() -> Result<Vec<(u32, u32, String, Rect)>, String> {
    // SAFETY: crear la fábrica DXGI no tiene precondiciones.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    let mut a = 0;
    // SAFETY: enumeración estándar; termina con DXGI_ERROR_NOT_FOUND.
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a) } {
        let mut o = 0;
        // SAFETY: ídem.
        while let Ok(output) = unsafe { adapter.EnumOutputs(o) } {
            // SAFETY: interfaz válida.
            if let Ok(desc) = unsafe { output.GetDesc() }
                && desc.AttachedToDesktop.as_bool()
            {
                let len = desc.DeviceName.iter().position(|c| *c == 0).unwrap_or(32);
                let name = String::from_utf16_lossy(&desc.DeviceName[..len]);
                let d = desc.DesktopCoordinates;
                let rect = Rect { left: d.left, top: d.top, right: d.right, bottom: d.bottom };
                found.push((a, o, name, rect));
            }
            o += 1;
        }
        a += 1;
    }
    Ok(found)
}

fn duplicate(
    output: &IDXGIOutput1,
    device: &ID3D11Device,
) -> Result<IDXGIOutputDuplication, String> {
    // SAFETY: interfaces válidas.
    unsafe { output.DuplicateOutput(device) }.map_err(|e| format!("DuplicateOutput: {e}"))
}

fn measure(
    adapter_index: u32,
    output_index: u32,
    row: u32,
    key: ColorKey,
    stop: &AtomicBool,
    ready: &std::sync::mpsc::Sender<f64>,
) -> Result<Measured, String> {
    let err = |e: windows::core::Error| e.to_string();
    // SAFETY: prioridad del hilo actual.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) };
    // SAFETY: llamadas DXGI/D3D11 estándar con punteros a variables locales;
    // todos los objetos viven en este hilo.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(err)?;
        let adapter: IDXGIAdapter1 = factory.EnumAdapters1(adapter_index).map_err(err)?;
        let output: IDXGIOutput1 =
            adapter.EnumOutputs(output_index).map_err(err)?.cast().map_err(err)?;

        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(err)?;
        let device = device.ok_or("sin dispositivo D3D11")?;
        let context = context.ok_or("sin contexto D3D11")?;
        let mut dup = duplicate(&output, &device)?;
        let mode = dup.GetDesc().ModeDesc;
        let hz =
            f64::from(mode.RefreshRate.Numerator) / f64::from(mode.RefreshRate.Denominator.max(1));
        let width = mode.Width;
        let _ = ready.send(hz);

        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: 1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging: Option<ID3D11Texture2D> = None;
        device.CreateTexture2D(&staging_desc, None, Some(&mut staging)).map_err(err)?;
        let staging = staging.ok_or("sin textura staging")?;

        let mut out = Measured { samples: Vec::with_capacity(65_536), lost: 0 };
        while !stop.load(Ordering::Relaxed) {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            match dup.AcquireNextFrame(50, &mut info, &mut resource) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    out.lost += 1;
                    drop(resource);
                    loop {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        if let Ok(d) = duplicate(&output, &device) {
                            dup = d;
                            break;
                        }
                        if stop.load(Ordering::Relaxed) {
                            return Ok(out);
                        }
                    }
                    continue;
                }
                Err(e) => return Err(format!("AcquireNextFrame: {e}")),
            }
            if info.LastPresentTime != 0 {
                let mut x = None;
                let mut covered = 0;
                if let Some(tex) = resource.and_then(|r| r.cast::<ID3D11Texture2D>().ok()) {
                    let region = D3D11_BOX {
                        left: 0,
                        top: row,
                        front: 0,
                        right: width,
                        bottom: row + 1,
                        back: 1,
                    };
                    context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &tex, 0, Some(&region));
                    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                    if context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).is_ok() {
                        let pixels = std::slice::from_raw_parts(
                            mapped.pData as *const u8,
                            width as usize * 4,
                        );
                        (x, covered) = find_color(pixels, key);
                        context.Unmap(&staging, 0);
                    }
                }
                out.samples.push(Sample {
                    qpc: info.LastPresentTime,
                    x,
                    covered,
                    accumulated: info.AccumulatedFrames,
                });
            }
            let _ = dup.ReleaseFrame();
        }
        Ok(out)
    }
}

/// Centro del tramo contiguo más largo del color en una fila BGRA, y cuántos
/// píxeles de la fila tienen ese color.
fn find_color(bgra: &[u8], key: ColorKey) -> (Option<u32>, u32) {
    let is = |p: &[u8; 4]| key.matches(p);
    let (mut best, mut best_len) = (None, 0usize);
    let mut covered = 0u32;
    let mut run_start = None;
    let pixels = bgra.as_chunks::<4>().0;
    for (i, p) in pixels.iter().enumerate() {
        covered += u32::from(is(p));
        match (is(p), run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(s)) => {
                if i - s > best_len {
                    (best, best_len) = (Some((s + i - 1) / 2), i - s);
                }
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = run_start
        && pixels.len() - s > best_len
    {
        best = Some((s + pixels.len() - 1) / 2);
    }
    (best.map(|x| x as u32), covered)
}

/// Resumen de un tramo de muestras.
#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    /// Refrescos que hubo en el tramo (duración × frecuencia).
    pub refreshes: f64,
    /// Cuadros entregados.
    pub frames: usize,
    /// Cuadros con el cuadrado a la vista.
    pub visible: usize,
    /// Cuadros con el cuadrado en otra posición que en el anterior.
    pub moved: usize,
    /// Cuadros compuestos que no llegamos a leer.
    pub skipped: u32,
    /// Avance por cuadro (px) en el tramo, sin contar el salto de vuelta.
    pub step_mean: f64,
    pub step_sd: f64,
    pub step_min: i64,
    pub step_max: i64,
}

impl Stats {
    /// Refrescos con posición nueva (criterio del spike).
    pub fn moved_pct(&self) -> f64 {
        pct(self.moved as f64, self.refreshes)
    }

    /// Cuadros de DWM por refresco (el techo que pone DWM).
    pub fn frames_pct(&self) -> f64 {
        pct(self.frames as f64, self.refreshes)
    }

    /// Medición confiable si salteamos menos del 1 % de los cuadros.
    pub fn trustworthy(&self) -> bool {
        f64::from(self.skipped) <= self.frames as f64 * 0.01
    }
}

fn pct(a: f64, b: f64) -> f64 {
    if b <= 0.0 { 0.0 } else { 100.0 * a / b }
}

/// Analiza las muestras entre `from` y `to` (QPC).
pub fn analyze(samples: &[Sample], from: i64, to: i64, hz: f64) -> Stats {
    let secs = clock::micros(from, to) as f64 / 1e6;
    let mut s = Stats { refreshes: secs * hz, step_min: i64::MAX, ..Default::default() };
    let mut prev: Option<u32> = None;
    let mut steps: Vec<i64> = Vec::new();
    for sample in samples.iter().filter(|x| x.qpc >= from && x.qpc < to) {
        s.frames += 1;
        s.skipped += sample.accumulated.saturating_sub(1);
        if let Some(x) = sample.x {
            s.visible += 1;
            if let Some(p) = prev {
                if p != x {
                    s.moved += 1;
                }
                let d = i64::from(x) - i64::from(p);
                if d > 0 {
                    steps.push(d);
                }
            }
        }
        prev = sample.x;
    }
    if !steps.is_empty() {
        let n = steps.len() as f64;
        s.step_mean = steps.iter().sum::<i64>() as f64 / n;
        let var = steps.iter().map(|d| (*d as f64 - s.step_mean).powi(2)).sum::<f64>() / n;
        s.step_sd = var.sqrt();
        s.step_min = *steps.iter().min().unwrap_or(&0);
        s.step_max = *steps.iter().max().unwrap_or(&0);
    } else {
        s.step_min = 0;
    }
    s
}

/// Primer cuadro con el cuadrado a la vista desde `from`, en ms.
pub fn first_visible(samples: &[Sample], from: i64, until: i64) -> Option<f64> {
    samples
        .iter()
        .find(|s| s.qpc >= from && s.qpc < until && s.x.is_some())
        .map(|s| clock::micros(from, s.qpc) as f64 / 1000.0)
}

/// Captura la imagen actual del monitor principal (BGRA, ancho, alto).
pub fn capture_primary() -> Result<(Vec<u8>, u32, u32), String> {
    let err = |e: windows::core::Error| e.to_string();
    let (a, o, _, _) = outputs()?
        .into_iter()
        .find(|(_, _, _, r)| r.left == 0 && r.top == 0)
        .ok_or("no hay monitor principal")?;
    // SAFETY: igual que `measure`; todo vive en esta función.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(err)?;
        let adapter: IDXGIAdapter1 = factory.EnumAdapters1(a).map_err(err)?;
        let output: IDXGIOutput1 = adapter.EnumOutputs(o).map_err(err)?.cast().map_err(err)?;
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(err)?;
        let device = device.ok_or("sin dispositivo")?;
        let context = context.ok_or("sin contexto")?;
        let dup = duplicate(&output, &device)?;
        let mode = dup.GetDesc().ModeDesc;
        let (w, h) = (mode.Width, mode.Height);
        // El primer cuadro de una duplicación nueva trae la imagen completa.
        for _ in 0..40 {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            if dup.AcquireNextFrame(100, &mut info, &mut resource).is_err() {
                continue;
            }
            if info.LastPresentTime == 0 {
                let _ = dup.ReleaseFrame();
                continue;
            }
            let tex: ID3D11Texture2D = resource.ok_or("sin recurso")?.cast().map_err(err)?;
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging: Option<ID3D11Texture2D> = None;
            device.CreateTexture2D(&desc, None, Some(&mut staging)).map_err(err)?;
            let staging = staging.ok_or("sin staging")?;
            context.CopyResource(&staging, &tex);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(err)?;
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    w as usize * 4,
                );
                out.extend_from_slice(row);
            }
            context.Unmap(&staging, 0);
            let _ = dup.ReleaseFrame();
            return Ok((out, w, h));
        }
        Err("no llegó ningún cuadro".to_owned())
    }
}

/// Guarda un recorte (`x`, `y`, `w`, `h`) de una imagen BGRA como BMP.
pub fn save_bmp(
    path: &std::path::Path,
    img: &[u8],
    stride_px: u32,
    (x, y, w, h): (u32, u32, u32, u32),
) -> std::io::Result<()> {
    let size = 54 + w * h * 4;
    let mut f = Vec::with_capacity(size as usize);
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&size.to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&(w as i32).to_le_bytes());
    f.extend_from_slice(&(-(h as i32)).to_le_bytes()); // de arriba hacia abajo
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&[0; 24]);
    for row in y..y + h {
        let start = ((row * stride_px + x) * 4) as usize;
        f.extend_from_slice(&img[start..start + (w * 4) as usize]);
    }
    std::fs::write(path, f)
}
