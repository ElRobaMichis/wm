//! Identidad estable de los monitores (architecture.md §8, spike 7).
//!
//! `\\.\DISPLAYn` y `HMONITOR` cambian entre arranques y al reconectar, así
//! que no sirven para recordar "este monitor". `QueryDisplayConfig` +
//! `DisplayConfigGetDeviceInfo` dan, por monitor activo, la ruta del
//! dispositivo, el fabricante y producto del EDID y la instancia del
//! conector; con eso se arma la clave.
//!
//! La identidad es la **tupla** ([`wm_model::screens::MonitorId`]): EDID,
//! número de serie (del EDID en el registro, sin admin) y conector. La ruta
//! del dispositivo sola no alcanza: lleva el conector pero no la serie, y
//! cambia si se mueve el cable de puerto (Fable, spike 7).

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{E_FAIL, ERROR_INSUFFICIENT_BUFFER};
use wm_model::Rect;
use wm_model::screens::MonitorId;

/// Un monitor activo con su identidad y su geometría actual.
#[derive(Debug, Clone)]
pub struct Display {
    /// Clave estable: ruta del dispositivo del monitor (incluye el id de
    /// hardware del EDID y la instancia).
    pub key: String,
    /// Nombre amigable del EDID (por ejemplo "AOC 24G2W1G4").
    pub name: String,
    /// Identidad para recordar este monitor.
    pub id: MonitorId,
    /// `false` si Windows dice que los ids del EDID no son confiables.
    pub edid_valid: bool,
    pub edid_manufacturer: u16,
    pub edid_product: u16,
    pub connector: u32,
    /// Tecnología de salida (HDMI, DisplayPort, interna...), código de Windows.
    pub output_technology: i32,
    /// Nombre GDI actual (`\\.\DISPLAYn`): solo para unir con `monitors()`.
    pub gdi_name: String,
    pub primary: bool,
    pub bounds: Rect,
    pub work_area: Rect,
    pub dpi: u32,
    pub refresh_hz: Option<u32>,
}

fn wide(s: &[u16]) -> String {
    let len = s.iter().position(|c| *c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..len])
}

/// Número de serie del EDID guardado en el registro para esta ruta de
/// dispositivo (bytes 12–15). `None` si no hay o es 0.
fn edid_serial(device_path: &str) -> Option<u32> {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY, RegGetValueW};
    let parts: Vec<&str> = device_path.split('#').collect();
    let (hw, inst) = (parts.get(1)?, parts.get(2)?);
    let key = format!(r"SYSTEM\CurrentControlSet\Enum\DISPLAY\{hw}\{inst}\Device Parameters");
    let key: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u8; 512];
    let mut len = buf.len() as u32;
    // SAFETY: cadena terminada en 0, buffer local con su tamaño en bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            windows::core::PCWSTR(key.as_ptr()),
            windows::core::w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if status.is_err() || len < 16 {
        return None;
    }
    let serial = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    (serial != 0).then_some(serial)
}

/// La parte del conector de la ruta (`uid4353`), en minúsculas.
fn connector_of(device_path: &str) -> String {
    let inst = device_path.split('#').nth(2).unwrap_or("");
    inst.rsplit('&').next().unwrap_or(inst).to_lowercase()
}

/// Monitores activos, con identidad estable. Si la configuración está a
/// medio cambiar (un camino sin su monitor), devuelve error en vez de una
/// lista incompleta: una topología "más chica" falsa haría mover ventanas.
pub fn displays() -> windows::core::Result<Vec<Display>> {
    let (paths, _modes) = query()?;
    let monitors = crate::monitors();
    let mut out: Vec<Display> = Vec::new();
    for path in paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        // SAFETY: estructuras locales con su encabezado y tamaño correctos.
        let (rs, rt) = unsafe {
            (
                DisplayConfigGetDeviceInfo(&mut source.header),
                DisplayConfigGetDeviceInfo(&mut target.header),
            )
        };
        if rs != 0 || rt != 0 {
            continue;
        }
        let gdi_name = wide(&source.viewGdiDeviceName);
        let Some(m) = monitors.iter().find(|m| m.device == gdi_name) else {
            return Err(windows::core::Error::new(E_FAIL, "configuración de pantallas incoherente"));
        };
        let key = wide(&target.monitorDevicePath).to_lowercase();
        // Espejo: dos caminos a la misma fuente; se cuenta una vez.
        if out.iter().any(|d| d.gdi_name == gdi_name) {
            continue;
        }
        // SAFETY: unión de banderas; `value` cubre todos los bits.
        let edid_valid = unsafe { target.flags.Anonymous.value } & 0b100 != 0;
        let id = MonitorId {
            manufacturer: target.edidManufactureId,
            product: target.edidProductCodeId,
            serial: edid_serial(&key),
            connector: connector_of(&key),
        };
        out.push(Display {
            key,
            name: wide(&target.monitorFriendlyDeviceName),
            id,
            edid_valid,
            edid_manufacturer: target.edidManufactureId,
            edid_product: target.edidProductCodeId,
            connector: target.connectorInstance,
            output_technology: target.outputTechnology.0,
            gdi_name,
            primary: m.primary,
            bounds: m.bounds,
            work_area: m.work_area,
            dpi: m.dpi,
            refresh_hz: m.refresh_hz,
        });
    }
    Ok(out)
}

fn query() -> windows::core::Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>)> {
    // Si la topología cambia entre las dos llamadas, se reintenta; con tope.
    for _ in 0..4 {
        let (mut np, mut nm) = (0u32, 0u32);
        // SAFETY: punteros a variables locales.
        unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) }.ok()?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        // SAFETY: buffers del tamaño informado.
        let r = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut np,
                paths.as_mut_ptr(),
                &mut nm,
                modes.as_mut_ptr(),
                None,
            )
        };
        if r == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        r.ok()?;
        paths.truncate(np as usize);
        modes.truncate(nm as usize);
        return Ok((paths, modes));
    }
    Err(windows::core::Error::new(E_FAIL, "la configuración de pantallas no se estabilizó"))
}
