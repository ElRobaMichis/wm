//! Instalar y desinstalar sin terminal (hito U5, `plan-update.md`).
//!
//! - **Instalador:** el mismo binario que `wmw.exe`, llamado `wm-setup…`. Al
//!   abrirlo pregunta, baja la versión publicada (firmada y verificada, con
//!   el mismo código que las actualizaciones) a
//!   `%LOCALAPPDATA%\Programs\wm`, y la arranca. Volver a abrirlo repara.
//! - `wmw.exe install-finish [--no-autostart]`: lo corre la copia instalada:
//!   entrada en "Aplicaciones instaladas", acceso directo, inicio con Windows,
//!   y sigue como guardián (arranca wm).
//! - `wm uninstall [--quiet] [--purge]`: cierra wm, muestra todas las
//!   ventanas, quita todo; las letras elegidas (`%APPDATA%\wm`) quedan salvo
//!   con `--purge`.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wm_model::update::FILES;
use wm_platform::session::{ask_box, info_box, warning_box};
use wm_platform::tray::{Order, send_order};

const TITLE: &str = "Instalar wm";

/// ¿Este ejecutable se llama `wm-setup…`? (modo instalador)
pub fn is_setup_name() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .is_some_and(|n| n.starts_with("wm-setup"))
}

fn log(msg: &str) {
    eprintln!("[instalación] {msg}");
    crate::brain::append_line(&format!("[instalación] {msg}"));
}

/// Cierra el wm que esté corriendo (con "Salir") y espera hasta `limit` a que
/// termine. `true` si no queda ninguno.
fn stop_running(limit: Duration) -> bool {
    let free = || !wm_platform::instance::running("guardian");
    if free() {
        return true;
    }
    send_order(Order::Quit);
    let until = Instant::now() + limit;
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
        if free() {
            return true;
        }
    }
    false
}

/// Coloca los archivos preparados en la carpeta (primera instalación o
/// reparación: no hay versión en prueba).
fn place(dir: &Path, stage: &Path) -> Result<(), String> {
    for name in FILES {
        let dest = dir.join(name);
        if dest.exists() && std::fs::remove_file(&dest).is_err() {
            // En uso: apartarlo (se borra en el próximo arranque).
            let aside = dir.join(format!("{name}.del-{}", std::process::id()));
            std::fs::rename(&dest, &aside).map_err(|e| format!("{name}: {e}"))?;
        }
        std::fs::rename(stage.join(name), &dest).map_err(|e| format!("{name}: {e}"))?;
    }
    let _ = std::fs::remove_dir_all(stage);
    Ok(())
}

/// Un aviso de error: en un cuadro, o (con `--quiet`, pruebas y la CI) solo
/// en el log: un cuadro sin nadie que lo cierre trabaría el proceso.
fn warn(quiet: bool, text: &str) {
    log(text);
    if !quiet {
        warning_box(TITLE, text);
    }
}

/// El modo instalador (doble clic en `wm-setup-x64.exe`).
pub fn run_setup(quiet: bool, tag: Option<String>) -> ExitCode {
    let Some(dir) = crate::update::install_dir() else { return ExitCode::FAILURE };
    if !quiet
        && !ask_box(
            TITLE,
            "¿Instalar wm en esta compu?\n\n\
             Se instala solo para vos, sin permisos de administrador, y se abre solo cuando \
             prendés la compu (lo podés cambiar desde su ícono).\n\n\
             Una vez por día se fija en GitHub si hay una versión nueva; no manda ningún dato tuyo.",
        )
    {
        return ExitCode::SUCCESS;
    }
    if !stop_running(Duration::from_secs(10)) {
        warn(quiet, "Primero cerrá wm: clic en su ícono → Salir. Después probá de nuevo.");
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn(quiet, &format!("No se pudo crear la carpeta de wm ({e})."));
        return ExitCode::FAILURE;
    }
    log(&format!("instalando en {}", dir.display()));
    let prepared = crate::update::prepare_with(&dir, true, tag, true);
    let v = match prepared {
        Ok(crate::update::Prepared::Ready(v)) => v,
        Ok(crate::update::Prepared::UpToDate) => {
            // Publicada más vieja que este instalador: no se baja de versión.
            warning_box(
                TITLE,
                "La versión publicada es más vieja que este instalador. No se instaló nada.",
            );
            return ExitCode::FAILURE;
        }
        Ok(crate::update::Prepared::Skipped(why)) => {
            warn(quiet, &format!("No se instaló nada: {why}."));
            return ExitCode::FAILURE;
        }
        Err(e) => {
            log(&format!("no se pudo bajar: {e}"));
            let text = if e.contains("conexión") || e.contains("servidor") {
                "No se pudo descargar wm. Fijate si tenés internet y probá de nuevo."
            } else {
                "La descarga llegó mal y no se instaló nada. Probá de nuevo en un rato."
            };
            warn(quiet, text);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = place(&dir, &crate::update::staged_dir_for(&dir, v)) {
        log(&format!("no se pudieron colocar los archivos: {e}"));
        warn(quiet, "No se pudieron copiar los archivos. Probá de nuevo.");
        return ExitCode::FAILURE;
    }
    // El registro, el acceso directo y el arranque los hace la copia
    // instalada (la que va a quedar).
    let mut cmd = std::process::Command::new(dir.join("wmw.exe"));
    cmd.arg("install-finish");
    if quiet {
        cmd.arg("--quiet");
    }
    if let Err(e) = cmd.spawn() {
        warn(quiet, &format!("wm se copió pero no arrancó ({e})."));
        return ExitCode::FAILURE;
    }
    log(&format!("versión {v} instalada"));
    if !quiet {
        // Esperar a que aparezca el ícono antes de decir "listo".
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until && !wm_platform::instance::running("guardian") {
            std::thread::sleep(Duration::from_millis(100));
        }
        info_box(
            TITLE,
            "¡Listo! wm ya está andando.\n\n\
             Buscá el cuadradito con un 1 abajo a la derecha, al lado del reloj (si no lo ves, \
             tocá la flechita ^).\n\n\
             Mantené apretado Bloq Mayús para ver los atajos.",
        );
    }
    ExitCode::SUCCESS
}

/// `wmw.exe install-finish`: lo corre la copia recién instalada.
pub fn install_finish(args: &[String]) -> ExitCode {
    let Some(dir) = crate::update::install_dir() else { return ExitCode::FAILURE };
    let version = env!("CARGO_PKG_VERSION");
    if let Err(e) = wm_platform::install::register(&dir, version) {
        log(&format!("no se pudo registrar: {e}"));
    }
    if let Err(e) = wm_platform::install::create_shortcut(&dir.join("wmw.exe")) {
        log(&format!("no se pudo crear el acceso directo: {e}"));
    }
    // El inicio automático es de la usuaria real; las pruebas (con
    // `WM_INSTANCE`) no lo tocan.
    let testing = std::env::var_os("WM_INSTANCE").is_some_and(|v| !v.is_empty());
    if !args.iter().any(|a| a == "--no-autostart")
        && !testing
        && let Err(e) = wm_platform::autostart::set(true)
    {
        log(&format!("no se pudo activar el inicio con Windows: {e}"));
    }
    log(&format!("instalación {version} terminada en {}", dir.display()));
    if args.iter().any(|a| a == "--no-start") {
        return ExitCode::SUCCESS;
    }
    crate::guardian::run(&[])
}

/// `wm uninstall [--quiet] [--purge]`.
pub fn uninstall(args: &[String]) -> ExitCode {
    let quiet = args.iter().any(|a| a == "--quiet");
    let purge = args.iter().any(|a| a == "--purge");
    let Some(dir) = crate::update::install_dir() else { return ExitCode::FAILURE };
    let title = "Desinstalar wm";
    if !quiet
        && !ask_box(
            title,
            "¿Desinstalar wm?\n\nTus ventanas quedan como están. Las letras que elegiste para tus \
             apps se guardan por si volvés a instalarlo.",
        )
    {
        return ExitCode::SUCCESS;
    }
    if !stop_running(Duration::from_secs(10)) {
        if !quiet {
            warning_box(
                title,
                "Primero cerrá wm: clic en su ícono → Salir. Después probá de nuevo.",
            );
        }
        return ExitCode::FAILURE;
    }
    // Nunca perder ventanas (architecture.md §9.7).
    let _ = wm_platform::hide::restore_all();
    let testing = std::env::var_os("WM_INSTANCE").is_some_and(|v| !v.is_empty());
    if !testing {
        let _ = wm_platform::autostart::set(false);
    }
    wm_platform::install::remove_shortcut();
    wm_platform::install::unregister();
    // Este mismo ejecutable puede estar en la carpeta: se aparta a %TEMP%
    // (mismo volumen) para poder borrar la carpeta.
    if let Ok(exe) = std::env::current_exe()
        && exe.parent().is_some_and(|p| p == dir)
    {
        let aside = std::env::temp_dir().join(format!("wm-uninstall-{}.exe", std::process::id()));
        let _ = std::fs::rename(&exe, aside);
    }
    let removed = std::fs::remove_dir_all(&dir).is_ok() || !dir.exists();
    if !testing && let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let _ = std::fs::remove_dir_all(Path::new(&local).join("wm"));
    }
    if purge
        && !testing
        && let Some(roaming) = std::env::var_os("APPDATA")
    {
        let _ = std::fs::remove_dir_all(Path::new(&roaming).join("wm"));
    }
    if !quiet {
        info_box(
            title,
            if removed {
                "wm se desinstaló."
            } else {
                "wm se desinstaló, pero quedaron algunos archivos en uso: se pueden borrar después."
            },
        );
    }
    ExitCode::SUCCESS
}
