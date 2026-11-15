//! Memoria de cualquier proceso **sin abrirlo** (`NtQuerySystemInformation`):
//! sirve también para `dwm.exe`, que no deja que un proceso sin elevar le
//! pida un handle.

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

/// Memoria de un proceso, en bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Memory {
    /// Working set privado (lo que usa en RAM y no comparte).
    pub private_ws: u64,
    /// Bytes privados comprometidos (commit).
    pub commit: u64,
}

/// Recorre todos los procesos y devuelve los que cumplen `pick(pid, nombre)`.
fn scan(mut pick: impl FnMut(u32, &str) -> bool) -> Vec<(u32, Memory)> {
    let mut buf: Vec<u64> = vec![0; 1 << 17]; // 1 MB, alineado a 8
    loop {
        let mut needed = 0u32;
        // SAFETY: buffer propio con su tamaño en bytes.
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut needed,
            )
        };
        if status.0 == 0 {
            break;
        }
        // STATUS_INFO_LENGTH_MISMATCH: agrandar y reintentar.
        if status.0 as u32 == 0xC000_0004 && buf.len() < (64 << 20) / 8 {
            buf = vec![0; (needed as usize / 8 + 1).max(buf.len() * 2)];
            continue;
        }
        return Vec::new();
    }
    let mut found = Vec::new();
    let base = buf.as_ptr().cast::<u8>();
    let mut offset = 0usize;
    loop {
        // SAFETY: el sistema llenó el buffer con entradas encadenadas por
        // NextEntryOffset; cada una empieza con SYSTEM_PROCESS_INFORMATION.
        let info = unsafe { &*(base.add(offset) as *const SYSTEM_PROCESS_INFORMATION) };
        let pid = info.UniqueProcessId.0 as usize as u32;
        let name = if info.ImageName.Buffer.is_null() {
            String::new()
        } else {
            let len = info.ImageName.Length as usize / 2;
            // SAFETY: cadena del sistema con su largo en bytes.
            let s = unsafe { std::slice::from_raw_parts(info.ImageName.Buffer.0, len) };
            String::from_utf16_lossy(s)
        };
        if pick(pid, &name) {
            // WorkingSetPrivateSize: los primeros 8 bytes de Reserved1.
            let mut ws = [0u8; 8];
            ws.copy_from_slice(&info.Reserved1[..8]);
            found.push((
                pid,
                Memory { private_ws: u64::from_le_bytes(ws), commit: info.PrivatePageCount as u64 },
            ));
        }
        if info.NextEntryOffset == 0 {
            break;
        }
        offset += info.NextEntryOffset as usize;
    }
    found
}

pub fn of_pid(pid: u32) -> Option<Memory> {
    scan(|p, _| p == pid).first().map(|(_, m)| *m)
}

pub fn of_name(name: &str) -> Option<Memory> {
    scan(|_, n| n.eq_ignore_ascii_case(name)).first().map(|(_, m)| *m)
}
