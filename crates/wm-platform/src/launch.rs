//! Abrir programas para el usuario (los ajustes desde la bandeja, las apps
//! de `Caps` + letra), sin pasarles las variables propias de `wm`
//! (`WM_TEST`, `WM_INSTANCE`, `WM_RELAUNCH`…): heredarlas cambiaría cómo se
//! comporta, por ejemplo, `wm-settings.exe`.
//!
//! El plan (Fable) prevé además partir del entorno limpio del usuario
//! (`CreateEnvironmentBlock`) para que no hereden lo de la terminal desde la
//! que se lanzó `wm core`; con `wmw.exe` al iniciar sesión el entorno ya es
//! el del usuario, así que queda para cuando haga falta.

use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Lanza `exe` sin esperar, sin consola heredada y sin las variables `WM_*`.
pub fn spawn_user(exe: &Path) -> std::io::Result<Child> {
    let mut cmd = Command::new(exe);
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().to_ascii_uppercase().starts_with("WM_") {
            cmd.env_remove(&k);
        }
    }
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
}

/// Abrir una app como "un clic en su ícono": la ruta de un `.exe`, o (con
/// `explorer.exe`) un acceso directo `.lnk` o `shell:AppsFolder\<AppID>`
/// (apps de la Store y accesos directos con argumentos; plan-apps.md §3).
pub fn open(target: &str) -> std::io::Result<()> {
    let lower = target.to_lowercase();
    if lower.starts_with("shell:") || lower.ends_with(".lnk") {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let explorer = Path::new(&root).join("explorer.exe");
        return spawn_user_with(&explorer, &[target]).map(|_| ());
    }
    spawn_user(Path::new(target)).map(|_| ())
}

/// Como [`spawn_user`], con argumentos.
pub fn spawn_user_with(exe: &Path, args: &[&str]) -> std::io::Result<Child> {
    let mut cmd = Command::new(exe);
    cmd.args(args);
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().to_ascii_uppercase().starts_with("WM_") {
            cmd.env_remove(&k);
        }
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
}
