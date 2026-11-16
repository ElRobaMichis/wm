//! Instalación sin administrador (hito U5, `plan-update.md`): la entrada en
//! "Aplicaciones instaladas" (`HKCU\…\Uninstall\wm`) y el acceso directo del
//! menú Inicio. Con `WM_INSTANCE` (pruebas), los nombres llevan sufijo para
//! no tocar los de la usuaria.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RegDeleteTreeW, RegSetKeyValueW,
};
use windows::core::{HSTRING, Interface};

fn suffix() -> String {
    std::env::var("WM_INSTANCE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| format!("-{s}"))
        .unwrap_or_default()
}

fn uninstall_key() -> String {
    format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\wm{}", suffix())
}

fn set_sz(key: &str, name: &str, value: &str) -> Result<(), String> {
    let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: datos locales con su tamaño en bytes.
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("registro {name}: {r:?}")) }
}

fn set_dword(key: &str, name: &str, value: u32) -> Result<(), String> {
    // SAFETY: un u32 local.
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            REG_DWORD.0,
            Some((&value as *const u32).cast()),
            4,
        )
    };
    if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("registro {name}: {r:?}")) }
}

/// La entrada de "Aplicaciones instaladas", con el desinstalador.
pub fn register(dir: &Path, version: &str) -> Result<(), String> {
    let key = uninstall_key();
    let wmw = dir.join("wmw.exe");
    let size_kb: u64 = ["wmw.exe", "wm.exe", "wm-settings.exe"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| m.len() / 1024)
        .sum();
    set_sz(&key, "DisplayName", "wm")?;
    set_sz(&key, "DisplayVersion", version)?;
    set_sz(&key, "Publisher", "wm (github.com/ElRobaMichis/wm)")?;
    set_sz(&key, "InstallLocation", &dir.display().to_string())?;
    set_sz(&key, "DisplayIcon", &wmw.display().to_string())?;
    set_sz(&key, "UninstallString", &format!("\"{}\" uninstall", wmw.display()))?;
    set_sz(&key, "QuietUninstallString", &format!("\"{}\" uninstall --quiet", wmw.display()))?;
    set_dword(&key, "NoModify", 1)?;
    set_dword(&key, "NoRepair", 1)?;
    set_dword(&key, "EstimatedSize", size_kb.min(u64::from(u32::MAX)) as u32)
}

pub fn unregister() {
    // SAFETY: clave propia del usuario.
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(uninstall_key())) };
}

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\wm.lnk`.
pub fn shortcut_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join(r"Microsoft\Windows\Start Menu\Programs")
            .join(format!("wm{}.lnk", suffix())),
    )
}

/// Acceso directo en el menú Inicio a `target` (con COM: `IShellLinkW`, sin
/// importar shell32 en forma estática).
pub fn create_shortcut(target: &Path) -> Result<(), String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    let path = shortcut_path().ok_or("sin carpeta del menú Inicio")?;
    // SAFETY: COM en este hilo; objetos del sistema con interfaces documentadas.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(&HSTRING::from(target.as_os_str())).map_err(|e| e.to_string())?;
        if let Some(dir) = target.parent() {
            let _ = link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()));
        }
        let _ = link.SetDescription(&HSTRING::from("wm: atajos de teclado para tus ventanas"));
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(&HSTRING::from(path.as_os_str()), true).map_err(|e| e.to_string())
    }
}

pub fn remove_shortcut() {
    if let Some(p) = shortcut_path() {
        let _ = std::fs::remove_file(p);
    }
}
