//! Quién es la app de un proceso (plan-apps.md §2): ejecutable, ruta,
//! **familia** y AUMID, con caché por pid.
//!
//! La familia es el ejecutable del ancestro más alto (hasta 3 niveles) que
//! abrió al proceso **desde su propia carpeta**: la ventana de Steam es de
//! `steamwebhelper.exe` (`…\Steam\bin\cef\…`), hijo de `steam.exe`
//! (`…\Steam\`), así que su familia es `steam`. Un Bloc de notas abierto por
//! Rayo no es de la familia de Rayo (otra carpeta). Solo lecturas de
//! proceso: ningún mensaje a ventanas.

use std::collections::HashMap;
use std::sync::Mutex;

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

/// Lo que se sabe de un proceso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    /// Ejecutable sin extensión, en minúsculas.
    pub exe: String,
    /// Ruta completa del ejecutable.
    pub path: String,
    /// El ejecutable de su familia, si es otro.
    pub family: Option<String>,
    /// AUMID (apps MSIX y de la Store).
    pub aumid: Option<String>,
}

/// pid → (hora de creación, datos). La hora detecta un pid reutilizado.
static CACHE: Mutex<Option<HashMap<u32, (u64, ProcInfo)>>> = Mutex::new(None);

struct Proc(HANDLE);

impl Drop for Proc {
    fn drop(&mut self) {
        // SAFETY: handle propio.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn open(pid: u32) -> Option<Proc> {
    // SAFETY: permiso mínimo; el handle se cierra con `Proc`.
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok().map(Proc)
}

fn image_path(p: &Proc) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: buffer local válido con su tamaño en caracteres.
    unsafe {
        QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
    }
    .ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

fn creation(p: &Proc) -> Option<u64> {
    let (mut c, mut e, mut k, mut u) =
        (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: estructuras locales.
    unsafe { GetProcessTimes(p.0, &mut c, &mut e, &mut k, &mut u) }.ok()?;
    Some((u64::from(c.dwHighDateTime) << 32) | u64::from(c.dwLowDateTime))
}

fn aumid(p: &Proc) -> Option<String> {
    use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    // SAFETY: buffer local con su tamaño; falla sin efectos si el proceso no
    // tiene identidad de paquete.
    let r = unsafe { GetApplicationUserModelId(p.0, &mut len, Some(PWSTR(buf.as_mut_ptr()))) };
    if r.0 != 0 || len == 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]);
    (!s.is_empty()).then_some(s)
}

/// Ejecutable sin extensión, en minúsculas.
pub fn stem(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let lower = file.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
}

fn dir_of(path: &str) -> String {
    let lower = path.to_lowercase();
    match lower.rfind('\\') {
        Some(i) => lower[..=i].to_owned(),
        None => lower,
    }
}

/// pid → pid del padre, de una sola foto de los procesos.
fn parents() -> HashMap<u32, (u32, String)> {
    let mut map = HashMap::new();
    // SAFETY: foto de procesos; el handle se cierra abajo.
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return map;
    };
    let mut e =
        PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    // SAFETY: estructura local con su tamaño.
    let mut ok = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    while ok {
        let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
        let name = String::from_utf16_lossy(&e.szExeFile[..len]);
        map.insert(e.th32ProcessID, (e.th32ParentProcessID, name));
        // SAFETY: ídem.
        ok = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    // SAFETY: handle propio.
    let _ = unsafe { CloseHandle(snap) };
    map
}

fn system_root() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).to_lowercase() + "\\"
}

/// El ejecutable de la familia de un proceso (`None` si es él mismo).
fn family_of(
    pid: u32,
    path: &str,
    born: u64,
    tree: &mut Option<HashMap<u32, (u32, String)>>,
) -> Option<String> {
    let tree = tree.get_or_insert_with(parents);
    let dir = dir_of(path);
    let root = system_root();
    let (mut cur, mut cur_born) = (pid, born);
    let mut family = None;
    for _ in 0..3 {
        let Some(&(parent, _)) = tree.get(&cur) else { break };
        if parent == 0 || parent == cur {
            break;
        }
        let Some(p) = open(parent) else { break };
        let (Some(ppath), Some(pborn)) = (image_path(&p), creation(&p)) else { break };
        // Un pid reutilizado: el "padre" nació después que el hijo.
        if pborn > cur_born {
            break;
        }
        let pdir = dir_of(&ppath);
        if pdir.starts_with(&root) {
            break;
        }
        if dir.starts_with(&pdir) {
            family = Some(stem(&ppath));
        }
        cur = parent;
        cur_born = pborn;
    }
    family.filter(|f| *f != stem(path))
}

/// Los datos de `pid` (de la caché si el proceso es el mismo).
pub fn info(pid: u32) -> Option<ProcInfo> {
    info_with(pid, &mut None)
}

fn info_with(pid: u32, tree: &mut Option<HashMap<u32, (u32, String)>>) -> Option<ProcInfo> {
    let p = open(pid)?;
    let born = creation(&p).unwrap_or(0);
    if let Ok(guard) = CACHE.lock()
        && let Some((b, i)) = guard.as_ref().and_then(|m| m.get(&pid))
        && *b == born
    {
        return Some(i.clone());
    }
    let path = image_path(&p)?;
    let i = ProcInfo {
        exe: stem(&path),
        family: family_of(pid, &path, born, tree),
        aumid: aumid(&p),
        path,
    };
    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() > 512 {
            map.clear();
        }
        map.insert(pid, (born, i.clone()));
    }
    Some(i)
}

/// Para `user_windows`: los datos de varios pids con una sola foto de
/// procesos (solo si alguno es nuevo).
pub fn infos(pids: &[u32]) -> HashMap<u32, ProcInfo> {
    let mut tree = None;
    pids.iter().filter_map(|&pid| info_with(pid, &mut tree).map(|i| (pid, i))).collect()
}

/// La ruta del ejecutable de un proceso vivo con alguno de estos nombres
/// (sin extensión, en minúsculas): para abrir de nuevo una app que está en
/// la bandeja.
pub fn running_exe(names: &[String]) -> Option<String> {
    let tree = parents();
    tree.iter()
        .filter(|(_, (_, name))| names.iter().any(|n| stem(name) == *n))
        .filter_map(|(&pid, _)| open(pid).and_then(|p| image_path(&p)))
        .next()
}

/// ¿Corre algún proceso con alguno de estos ejecutables?
pub fn running(names: &[String]) -> bool {
    !names.is_empty() && parents().values().any(|(_, name)| names.iter().any(|n| stem(name) == *n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_and_dirs() {
        assert_eq!(stem(r"C:\Program Files (x86)\Steam\steam.exe"), "steam");
        assert_eq!(stem("Brave.EXE"), "brave");
        assert_eq!(dir_of(r"C:\A\B\c.exe"), r"c:\a\b\");
    }

    #[test]
    fn our_own_process_has_a_path_and_no_family_in_system32() {
        let i = info(std::process::id()).expect("el propio proceso");
        assert!(!i.exe.is_empty());
        assert!(i.path.to_lowercase().ends_with(".exe"));
    }
}
