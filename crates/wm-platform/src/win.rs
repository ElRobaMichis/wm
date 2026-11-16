use std::mem::size_of;

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
    DPI_AWARENESS_CONTEXT_UNAWARE, GetDpiForMonitor, GetThreadDpiAwarenessContext,
    MDT_EFFECTIVE_DPI,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::MonitorInfo;
use wm_model::Rect;

const MONITORINFOF_PRIMARY: u32 = 1;

fn to_rect(r: RECT) -> Rect {
    Rect::new(r.left, r.top, r.right, r.bottom)
}

/// Número de build de Windows (por ejemplo, "26100"), leído del registro.
pub fn os_build() -> Option<String> {
    current_version("CurrentBuildNumber")
}

/// Tipo, edición y versión ("Server ServerDatacenter 24H2", "Client
/// Professional 24H2"): en la CI, `windows-latest` es Windows Server.
pub fn os_edition() -> Option<String> {
    let parts: Vec<String> = ["InstallationType", "EditionID", "DisplayVersion"]
        .into_iter()
        .filter_map(current_version)
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Un valor de texto de `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
fn current_version(name: &str) -> Option<String> {
    let mut buf = [0u16; 64];
    let mut len = size_of::<[u16; 64]>() as u32;
    let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `buf` y `len` son válidos durante la llamada y `len` refleja el
    // tamaño en bytes del buffer, como exige RegGetValueW; `name` termina en NUL.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            windows::core::PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if status.is_err() {
        return None;
    }
    let chars = (len as usize / 2).saturating_sub(1); // sin el NUL final
    Some(String::from_utf16_lossy(&buf[..chars]))
}

/// Modo de DPI con el que corre el hilo actual. Sirve para verificar que el
/// manifiesto embebido se aplicó (debe ser `PerMonitorV2`).
pub fn dpi_awareness() -> &'static str {
    // SAFETY: funciones sin precondiciones; los contextos son constantes del sistema.
    unsafe {
        let current = GetThreadDpiAwarenessContext();
        let known = [
            (DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, "PerMonitorV2"),
            (DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, "PerMonitor (v1)"),
            (DPI_AWARENESS_CONTEXT_SYSTEM_AWARE, "System"),
            (DPI_AWARENESS_CONTEXT_UNAWARE, "Unaware"),
        ];
        known
            .into_iter()
            .find(|(ctx, _)| AreDpiAwarenessContextsEqual(current, *ctx).as_bool())
            .map_or("desconocido", |(_, name)| name)
    }
}

/// Monitores conectados, en el orden en que los enumera Windows.
pub fn monitors() -> Vec<MonitorInfo> {
    let mut handles: Vec<HMONITOR> = Vec::new();

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` es el puntero a `handles` que pasamos abajo; vive
        // mientras dura EnumDisplayMonitors, que llama a este callback de forma
        // síncrona en el mismo hilo.
        let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        handles.push(monitor);
        true.into()
    }

    // SAFETY: el callback solo usa `data` como el `&mut Vec` que le pasamos.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        )
    };
    if !ok.as_bool() {
        return Vec::new();
    }

    handles.into_iter().filter_map(describe).collect()
}

fn describe(monitor: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` es un MONITORINFOEXW con `cbSize` correcto; la API acepta
    // el puntero a su encabezado MONITORINFO.
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) };
    if !ok.as_bool() {
        return None;
    }

    let name_len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
    let device = String::from_utf16_lossy(&info.szDevice[..name_len]);

    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: punteros a variables locales válidas durante la llamada.
    let dpi_result =
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let dpi = if dpi_result.is_ok() { dpi_x } else { 96 };

    let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    // SAFETY: `szDevice` termina en NUL y `mode` tiene `dmSize` correcto.
    let has_mode = unsafe {
        EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode)
    };
    // 0 y 1 significan "frecuencia por defecto del hardware" (no informada).
    let hz = mode.dmDisplayFrequency;
    let refresh_hz = (has_mode.as_bool() && hz > 1).then_some(hz);

    Some(MonitorInfo {
        device,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        bounds: to_rect(info.monitorInfo.rcMonitor),
        work_area: to_rect(info.monitorInfo.rcWork),
        dpi,
        refresh_hz,
    })
}
