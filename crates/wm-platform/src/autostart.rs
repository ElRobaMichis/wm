//! Iniciar con Windows (hito 3a, architecture.md §12.4): el valor `wm` en
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, sin permisos de
//! administrador, apuntando a `wmw.exe` (el ejecutable sin consola).
//!
//! Es **opcional y explícito** (el menú de la bandeja o `wm autostart on`):
//! un ejecutable sin firmar, con un hook de teclado, que se agregara solo al
//! inicio parecería malware.
//!
//! El Administrador de tareas lo deshabilita sin borrar el valor, con una
//! marca en `…\Explorer\StartupApproved\Run`: "activado" exige que no esté
//! esa marca, y activar la borra.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::core::HSTRING;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE: &str = "wm";

/// `wmw.exe`, al lado del ejecutable actual (si existe).
pub fn target() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let wmw = exe.with_file_name("wmw.exe");
    wmw.exists().then_some(wmw)
}

/// Lo que se escribe en el registro: la ruta entre comillas.
pub fn command_for(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn read_string(key: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: buffer local con su tamaño en bytes.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(value),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// ¿El Administrador de tareas lo deshabilitó? (primer byte impar = apagado)
fn disabled_by_task_manager() -> bool {
    let mut buf = [0u8; 12];
    let mut size = buf.len() as u32;
    // SAFETY: buffer local con su tamaño.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(APPROVED),
            &HSTRING::from(VALUE),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    r == ERROR_SUCCESS && size > 0 && buf[0] & 1 == 1
}

/// `None`: no se puede (no hay `wmw.exe` al lado). `Some(true)`: arranca con
/// Windows, con este `wmw.exe`.
pub fn status() -> Option<bool> {
    let wmw = target()?;
    let on = read_string(RUN, VALUE)
        .is_some_and(|v| v.trim().eq_ignore_ascii_case(&command_for(&wmw)))
        && !disabled_by_task_manager();
    Some(on)
}

/// Activa o desactiva el inicio con Windows.
pub fn set(on: bool) -> Result<(), String> {
    if !on {
        // SAFETY: claves y valores propios del usuario.
        let r = unsafe {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN), &HSTRING::from(VALUE))
        };
        return if r == ERROR_SUCCESS || r.0 == 2 { Ok(()) } else { Err(format!("{r:?}")) };
    }
    let wmw = target().ok_or("no está wmw.exe al lado de wm.exe")?;
    let data: Vec<u16> = command_for(&wmw).encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: datos locales con su tamaño en bytes.
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN),
            &HSTRING::from(VALUE),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    if r != ERROR_SUCCESS {
        return Err(format!("{r:?}"));
    }
    // Si el Administrador de tareas lo había apagado, se vuelve a permitir.
    // SAFETY: ídem.
    let _ = unsafe {
        RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED), &HSTRING::from(VALUE))
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_quoted() {
        let p = Path::new(r"C:\Program Files\wm\wmw.exe");
        assert_eq!(command_for(p), r#""C:\Program Files\wm\wmw.exe""#);
    }
}
