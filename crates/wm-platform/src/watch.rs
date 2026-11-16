//! Recarga de `apps.txt` (hito 3c): un hilo `wm-watch` que duerme en una
//! espera del kernel (`FindFirstChangeNotificationW` sobre la carpeta) y,
//! cuando algo cambia, espera 100 ms de silencio (antirrebote: el guardado
//! de los ajustes escribe un temporal y lo renombra) y lee el archivo. Si el
//! texto cambió, se lo manda al brain por canal + `WM_OPS`.
//!
//! En su propio hilo y no en un carril de `ops`: el disco (antivirus, una
//! carpeta redirigida) no puede trabar a `Place` ni a `Inspect`. Costo en
//! reposo: una espera de kernel, 0 % de CPU.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FindCloseChangeNotification,
    FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
use windows::core::HSTRING;

use crate::ops;

/// Silencio que se espera antes de leer (antirrebote).
const QUIET_MS: u32 = 100;

/// El lado del brain: el texto nuevo del archivo cada vez que cambia.
pub struct Watch {
    changes: Receiver<String>,
}

impl Watch {
    /// Vigila `file` (crea su carpeta si no existe). `None`: no se pudo.
    pub fn start(brain: u32, file: PathBuf) -> Option<Self> {
        let dir = file.parent()?.to_path_buf();
        std::fs::create_dir_all(&dir).ok()?;
        let (tx, changes) = mpsc::channel();
        let initial = read(&file);
        std::thread::Builder::new()
            .name("wm-watch".into())
            .spawn(move || {
                let mut last = initial;
                // SAFETY: carpeta existente; el handle se cierra al terminar.
                let Ok(h) = (unsafe {
                    FindFirstChangeNotificationW(
                        &HSTRING::from(dir.as_os_str()),
                        false,
                        FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
                    )
                }) else {
                    return;
                };
                loop {
                    // SAFETY: handle válido.
                    if unsafe { WaitForSingleObject(h, INFINITE) } != WAIT_OBJECT_0 {
                        break;
                    }
                    // Antirrebote: rearmar hasta que haya 100 ms de silencio.
                    loop {
                        // SAFETY: handle válido.
                        if unsafe { FindNextChangeNotification(h) }.is_err() {
                            break;
                        }
                        // SAFETY: ídem.
                        if unsafe { WaitForSingleObject(h, QUIET_MS) } == WAIT_TIMEOUT {
                            break;
                        }
                    }
                    let now = read(&file);
                    if now != last {
                        last = now.clone();
                        if tx.send(now.unwrap_or_default()).is_err() {
                            break; // el brain terminó
                        }
                        ops::wake(brain);
                    }
                }
                // SAFETY: handle propio.
                let _ = unsafe { FindCloseChangeNotification(h) };
            })
            .ok()?;
        Some(Self { changes })
    }

    /// Los cambios pendientes (el último gana).
    pub fn drain(&mut self) -> Option<String> {
        self.changes.try_iter().last()
    }
}

fn read(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok()
}
