//! Las apps instaladas, para elegir en los ajustes (plan-apps.md §4): la
//! carpeta virtual `shell:AppsFolder` ("Todas las aplicaciones" del menú
//! Inicio), que junta los accesos directos de los dos menús Inicio y las
//! apps de la Store. Solo COM (`ole32`): `wm.exe` no importa `shell32`.

use windows::Win32::Storage::EnhancedStorage::{PKEY_AppUserModel_ID, PKEY_Link_TargetParsingPath};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IKnownFolderManager, IShellItem,
    IShellItem2, KF_FLAG_DEFAULT, KnownFolderManager, SIGDN_NORMALDISPLAY,
};
use windows::core::{Interface, PWSTR};

/// Una app de "Todas las aplicaciones".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Nombre del menú Inicio ("Brave", "AutoCAD 2025 - Español").
    pub name: String,
    /// AppID: una ruta (o una carpeta conocida + ruta) para los accesos
    /// directos sin argumentos, o un AUMID (`Empresa.App_hash!App`).
    pub app_id: String,
    /// Destino del acceso directo, si lo tiene.
    pub target: Option<String>,
}

/// Lo que guarda `apps.txt` para una app elegida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Ejecutables (el primero es el principal).
    pub names: Vec<String>,
    /// Cómo abrirla.
    pub launch: Option<String>,
    /// Nombre visible.
    pub label: String,
}

fn take(p: PWSTR) -> String {
    // SAFETY: cadena devuelta por el shell, terminada en 0; se libera abajo.
    let s = unsafe { p.to_string() }.unwrap_or_default();
    // SAFETY: memoria del shell (CoTaskMemAlloc).
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

/// Lee "Todas las aplicaciones". Tarda unos cientos de ms: llamar fuera del
/// hilo de la interfaz.
pub fn installed() -> Result<Vec<Installed>, String> {
    // SAFETY: COM en este hilo; se libera al final.
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let result = (|| -> windows::core::Result<Vec<Installed>> {
        // SAFETY: CLSID e interfaz documentadas.
        let kfm: IKnownFolderManager =
            unsafe { CoCreateInstance(&KnownFolderManager, None, CLSCTX_INPROC_SERVER) }?;
        // SAFETY: carpeta conocida documentada.
        let folder = unsafe { kfm.GetFolder(&FOLDERID_AppsFolder) }?;
        // SAFETY: ídem.
        let root: IShellItem = unsafe { folder.GetShellItem(KF_FLAG_DEFAULT.0 as u32) }?;
        // SAFETY: enumerar los hijos de la carpeta.
        let items: IEnumShellItems = unsafe { root.BindToHandler(None, &BHID_EnumItems) }?;
        let mut out = Vec::new();
        loop {
            let mut batch: [Option<IShellItem>; 32] = Default::default();
            let mut fetched = 0u32;
            // SAFETY: arreglo local; `fetched` dice cuántos llenó.
            let hr = unsafe { items.Next(&mut batch, Some(&mut fetched)) };
            for item in batch.iter().take(fetched as usize).flatten() {
                // SAFETY: lecturas de propiedades del ítem.
                let name = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY) }.map(take);
                let Ok(item2) = item.cast::<IShellItem2>() else { continue };
                // SAFETY: lecturas de propiedades; fallan sin efectos si no están.
                let app_id = unsafe { item2.GetString(&PKEY_AppUserModel_ID) }.map(take);
                // SAFETY: ídem.
                let target = unsafe { item2.GetString(&PKEY_Link_TargetParsingPath) }
                    .map(take)
                    .ok()
                    .filter(|t| !t.is_empty());
                if let (Ok(name), Ok(app_id)) = (name, app_id) {
                    out.push(Installed { name, app_id, target });
                }
            }
            if hr.is_err() || fetched == 0 {
                break;
            }
        }
        Ok(out)
    })();
    if init.is_ok() {
        // SAFETY: balancea el CoInitializeEx de arriba.
        unsafe { CoUninitialize() };
    }
    result.map_err(|e| e.message())
}

/// Ejecutable sin extensión, en minúsculas.
fn stem(path: &str) -> String {
    crate::process_info::stem(path)
}

/// Nombre corto para una app sin ejecutable conocido (Store): "Microsoft
/// To Do" → "microsoft-to-do".
fn slug(name: &str) -> String {
    let s: String =
        name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// Qué se guarda al elegir `app` (`None`: no es una app que se pueda abrir:
/// páginas web, ayudas, desinstaladores).
pub fn choice(app: &Installed) -> Option<Choice> {
    let lower = app.name.to_lowercase();
    if ["uninstall", "desinstalar", "readme", "léame", "help", "ayuda"]
        .iter()
        .any(|w| lower.contains(w))
    {
        return None;
    }
    let label = app.name.clone();
    match &app.target {
        Some(t) if t.to_lowercase().ends_with(".exe") => {
            // AppID con forma de ruta: un acceso directo sin argumentos (se
            // abre el exe); si no, el acceso directo (conserva argumentos).
            let path_like = app.app_id.contains('\\');
            let launch =
                if path_like { t.clone() } else { format!("shell:AppsFolder\\{}", app.app_id) };
            Some(Choice { names: vec![stem(t)], launch: Some(launch), label })
        }
        Some(_) => None,
        // Sin destino: apps de la Store (AUMID con "!").
        None if app.app_id.contains('!') => Some(Choice {
            names: vec![slug(&app.name)],
            launch: Some(format!("shell:AppsFolder\\{}", app.app_id)),
            label,
        }),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, app_id: &str, target: Option<&str>) -> Installed {
        Installed { name: name.into(), app_id: app_id.into(), target: target.map(Into::into) }
    }

    #[test]
    fn choices_for_shortcuts_store_apps_and_junk() {
        let brave = app(
            "Brave",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            Some(r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"),
        );
        let c = choice(&brave).expect("brave");
        assert_eq!(c.names, vec!["brave".to_owned()]);
        assert!(c.launch.unwrap().ends_with("brave.exe"));

        let acad = app("AutoCAD 2025", "Autodesk.AutoCAD.2025", Some(r"C:\Autodesk\acad.exe"));
        assert_eq!(
            choice(&acad).unwrap().launch.as_deref(),
            Some("shell:AppsFolder\\Autodesk.AutoCAD.2025")
        );

        let todo = app("Microsoft To Do", "Microsoft.Todos_8wekyb3d8bbwe!App", None);
        let c = choice(&todo).unwrap();
        assert_eq!(c.names, vec!["microsoft-to-do".to_owned()]);
        assert_eq!(
            c.launch.as_deref(),
            Some("shell:AppsFolder\\Microsoft.Todos_8wekyb3d8bbwe!App")
        );

        assert!(
            choice(&app("Uninstall Foo", r"C:\foo\uninst.exe", Some(r"C:\foo\uninst.exe")))
                .is_none()
        );
        assert!(choice(&app("Sitio", "https://x", Some("https://x"))).is_none());
        assert!(choice(&app("Manual", r"C:\a\m.chm", Some(r"C:\a\m.chm"))).is_none());
    }
}
