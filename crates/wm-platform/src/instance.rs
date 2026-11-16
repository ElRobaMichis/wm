//! Instancia única (plan-core.md §6, hito 3a): un mutex con nombre por
//! sesión (`Local\…`). Lo toma el guardián que vigila al core y el core.
//!
//! Se toma **antes** de restaurar ventanas: un segundo guardián que
//! arrancara mostraría lo que escondió el primero.
//!
//! `WM_INSTANCE` cambia el nombre (para que `ci-probe` pueda lanzar su propio
//! core aunque el usuario tenga uno abierto).

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::HSTRING;

/// El mutex tomado; se suelta al soltar esto (o al terminar el proceso).
pub struct Instance(HANDLE);

// SAFETY: un handle de mutex se puede cerrar desde cualquier hilo.
unsafe impl Send for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: handle propio.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Nombre completo del mutex para `role` ("guardian", "core").
pub fn mutex_name(role: &str) -> String {
    let suffix = std::env::var("WM_INSTANCE").ok().filter(|s| !s.is_empty());
    match suffix {
        Some(s) => format!("Local\\wm.{role}.{s}"),
        None => format!("Local\\wm.{role}"),
    }
}

/// Toma la instancia `role`. `None`: ya hay otra corriendo en esta sesión.
pub fn acquire(role: &str) -> Option<Instance> {
    let name = HSTRING::from(mutex_name(role));
    // SAFETY: nombre válido; sin atributos de seguridad.
    let handle = unsafe { CreateMutexW(None, false, &name) }.ok()?;
    // SAFETY: sin precondiciones (justo después de CreateMutexW).
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: handle propio.
        let _ = unsafe { CloseHandle(handle) };
        return None;
    }
    Some(Instance(handle))
}

/// ¿Hay una instancia `role` corriendo? Sin crear el mutex (crearlo, aunque
/// sea un instante, haría creer a un guardián que arranca que ya hay otro).
pub fn running(role: &str) -> bool {
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let name = HSTRING::from(mutex_name(role));
    // SAFETY: nombre válido; solo se abre para saber si existe.
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) } {
        Ok(h) => {
            // SAFETY: handle propio.
            let _ = unsafe { CloseHandle(h) };
            true
        }
        Err(_) => false,
    }
}
