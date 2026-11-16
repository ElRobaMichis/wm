//! Actualizaciones (hito U, `plan-update.md` U3-U4): buscar, bajar,
//! verificar y preparar una versión nueva (`wm update run`, rol efímero:
//! la red nunca entra al guardián ni al core), y colocarla o volver atrás
//! (lo llama el guardián al arrancar).
//!
//! Solo se ejecuta lo que pasó la firma del manifiesto (clave embebida) y el
//! tamaño y el SHA-256 de cada archivo. Nada se baja a `%TEMP%`. Una versión
//! a medio bajar es invisible: `ready` se escribe al final.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wm_model::update::{
    self, Decision, FILES, MANIFEST_MAX, Manifest, State, Step, Version, decide, der_to_raw,
    parse_manifest,
};

/// El repositorio de donde salen las versiones.
const RELEASES: &str = "https://github.com/ElRobaMichis/wm/releases";

/// Clave pública de los releases (X ‖ Y en hex; la privada es el secret
/// `WM_UPDATE_KEY` de GitHub). Vacía: el actualizador no acepta nada.
const RELEASE_KEY: &str = include_str!("../../update-pubkey.txt");

/// Error de "no hay versión publicada" (404 en GitHub).
const NO_RELEASE: &str = "todavía no hay una versión publicada";

/// Mínimo entre búsquedas automáticas.
const MIN_INTERVAL: Duration = Duration::from_secs(20 * 3600);

/// Código con que el core le pide al guardián "aplicar la actualización y
/// relanzarme".
pub const EXIT_APPLY_UPDATE: u32 = 10;

pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("versión del Cargo.toml")
}

pub fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" }
}

fn log(msg: &str) {
    eprintln!("[actualización] {msg}");
    crate::brain::append_line(&format!("[actualización] {msg}"));
}

/// La carpeta de instalación: `%LOCALAPPDATA%\Programs\wm` (o
/// `WM_INSTALL_ROOT`, para pruebas).
pub fn install_dir() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("WM_INSTALL_ROOT").filter(|r| !r.is_empty()) {
        return Some(PathBuf::from(root));
    }
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Programs").join("wm"))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// ¿Este ejecutable es la copia instalada? Solo esa busca actualizaciones:
/// la de `cargo build` o una portable no hace tráfico (architecture.md §12.4).
pub fn installed_here() -> bool {
    let (Ok(exe), Some(dir)) = (std::env::current_exe(), install_dir()) else { return false };
    exe.parent().is_some_and(|p| same_dir(p, &dir))
}

fn state_path() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("WM_INSTALL_ROOT").filter(|r| !r.is_empty()) {
        return Some(PathBuf::from(root).join("update-state.txt"));
    }
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("wm").join("update.txt"))
}

pub fn load_state() -> State {
    state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| State::parse(&t))
        .unwrap_or_default()
}

pub fn save_state(s: &State) {
    if let Some(p) = state_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, s.format()).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn unhex64(s: &str) -> Option<[u8; 64]> {
    let s = s.trim();
    if s.len() != 128 {
        return None;
    }
    let mut out = [0u8; 64];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Las claves aceptadas: la de los releases y, si el binario se compiló con
/// `WM_UPDATE_PUBKEY` (pruebas), esa.
fn keys() -> Vec<[u8; 64]> {
    [option_env!("WM_UPDATE_PUBKEY"), Some(RELEASE_KEY)]
        .into_iter()
        .flatten()
        .filter_map(unhex64)
        .collect()
}

fn verify(text: &[u8], sig: &[u8]) -> Result<(), String> {
    let keys = keys();
    if keys.is_empty() {
        return Err("este binario no tiene clave para verificar actualizaciones".into());
    }
    let raw = der_to_raw(sig).ok_or("firma con formato inválido")?;
    if keys.iter().any(|k| wm_platform::sign::verify_p256(k, text, &raw)) {
        Ok(())
    } else {
        Err("la firma del manifiesto no es válida".into())
    }
}

/// De dónde se baja: GitHub, o una carpeta (`WM_UPDATE_FEED`, pruebas: igual
/// exige la firma).
enum Source {
    Folder(PathBuf),
    Web { tag: Option<String> },
}

fn source(tag: Option<String>) -> Source {
    match std::env::var_os("WM_UPDATE_FEED").filter(|f| !f.is_empty()) {
        Some(f) => Source::Folder(PathBuf::from(f)),
        None => Source::Web { tag },
    }
}

fn fetch(
    src: &Source,
    name: &str,
    tag_for_assets: Option<&str>,
    max: u64,
    out: &mut dyn std::io::Write,
) -> Result<u64, String> {
    match src {
        Source::Folder(dir) => {
            let mut f = std::fs::File::open(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
            let n = std::io::copy(&mut std::io::Read::take(&mut f, max + 1), out)
                .map_err(|e| e.to_string())?;
            if n > max {
                return Err(format!("{name}: más grande de lo esperado"));
            }
            Ok(n)
        }
        Source::Web { tag } => {
            // El manifiesto sale de "latest" (o del tag pedido); los archivos,
            // siempre de la versión que dice el manifiesto, en el mismo host.
            let url = match (tag_for_assets, tag.as_deref()) {
                (Some(t), _) | (None, Some(t)) => format!("{RELEASES}/download/{t}/{name}"),
                (None, None) => format!("{RELEASES}/latest/download/{name}"),
            };
            wm_platform::net::get(&url, max, Duration::from_secs(30), out).map_err(|e| match e {
                wm_platform::net::NetError::Status(404) => NO_RELEASE.to_owned(),
                e => e.to_string(),
            })
        }
    }
}

fn manifest_name() -> String {
    format!("latest-{}.txt", arch())
}

/// Baja y verifica el manifiesto.
fn fetch_manifest(src: &Source) -> Result<(Vec<u8>, Vec<u8>, Manifest), String> {
    let name = manifest_name();
    let mut text = Vec::new();
    fetch(src, &name, None, MANIFEST_MAX as u64, &mut text)?;
    let mut sig = Vec::new();
    fetch(src, &format!("{name}.sig"), None, 256, &mut sig)?;
    verify(&text, &sig)?;
    let m = parse_manifest(&text)?;
    Ok((text, sig, m))
}

fn staged_dir(dir: &Path, v: Version) -> PathBuf {
    dir.join("update").join(v.to_string())
}

/// Verifica una versión preparada: firma del manifiesto guardado y tamaño y
/// SHA-256 de cada archivo.
fn verify_staged(stage: &Path) -> Result<Manifest, String> {
    let name = manifest_name();
    let text = std::fs::read(stage.join(&name)).map_err(|e| e.to_string())?;
    let sig = std::fs::read(stage.join(format!("{name}.sig"))).map_err(|e| e.to_string())?;
    verify(&text, &sig)?;
    let m = parse_manifest(&text)?;
    for f in &m.files {
        let (sha, size) = wm_platform::sign::sha256_file(&stage.join(&f.name))
            .ok_or(format!("no se pudo leer {}", f.name))?;
        if size != f.size || sha != f.sha256 {
            return Err(format!("{} no coincide con el manifiesto", f.name));
        }
    }
    Ok(m)
}

/// Corre `exe self-check` con plazo: un binario que ni arranca no se coloca.
fn self_check_ok(exe: &Path) -> bool {
    let Ok(mut child) = std::process::Command::new(exe)
        .arg("self-check")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let until = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.code() == Some(0),
            Ok(None) if std::time::Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => {
                let _ = child.kill();
                return false;
            }
        }
    }
}

/// Resultado de buscar y preparar.
pub enum Prepared {
    UpToDate,
    Ready(Version),
    Skipped(String),
}

/// Busca y, si hay una versión nueva, la baja, verifica y deja lista en
/// `update\<v>\` (con `ready` al final). `into`: carpeta destino (la de
/// instalación). No toca los ejecutables en uso.
pub fn prepare(into: &Path, force: bool, tag: Option<String>) -> Result<Prepared, String> {
    prepare_with(into, force, tag, false)
}

/// Como [`prepare`]; `install`: baja la versión publicada aunque sea la misma
/// que este binario (el instalador).
pub fn prepare_with(
    into: &Path,
    force: bool,
    tag: Option<String>,
    install: bool,
) -> Result<Prepared, String> {
    let Some(_lock) = wm_platform::instance::acquire("update") else {
        return Ok(Prepared::Skipped("ya hay otra búsqueda en curso".into()));
    };
    let mut state = load_state();
    let now = now_secs();
    if !force && now.saturating_sub(state.last_check) < MIN_INTERVAL.as_secs() {
        return Ok(Prepared::Skipped("se buscó hace menos de 20 h".into()));
    }
    let src = source(tag);
    let (text, sig, m) = match fetch_manifest(&src) {
        Ok(x) => x,
        // Todavía no hay una versión oficial (solo pre-releases): nada que
        // bajar, no es un error.
        Err(e) if e.contains(NO_RELEASE) => return Ok(Prepared::UpToDate),
        Err(e) => return Err(e),
    };
    state.last_check = now;
    save_state(&state);
    let v = match decide(current(), arch(), &m, &state) {
        Decision::Update(v) => v,
        Decision::UpToDate if install && m.version == current() => m.version,
        Decision::UpToDate => return Ok(Prepared::UpToDate),
        Decision::Bad(v) => return Ok(Prepared::Skipped(format!("la {v} ya falló en esta PC"))),
        Decision::WrongArch => return Err("el manifiesto es de otra arquitectura".into()),
    };
    let stage = staged_dir(into, v);
    if stage.join("ready").exists() && verify_staged(&stage).is_ok() {
        return Ok(Prepared::Ready(v));
    }
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let tag = format!("v{v}");
        for f in &m.files {
            let part = stage.join(format!("{}.part", f.name));
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            fetch(&src, &f.asset, Some(&tag), f.size, &mut out)?;
            out.sync_all().map_err(|e| e.to_string())?;
            drop(out);
            let (sha, size) =
                wm_platform::sign::sha256_file(&part).ok_or("no se pudo leer la descarga")?;
            if size != f.size || sha != f.sha256 {
                return Err(format!("{} llegó distinto de lo esperado", f.name));
            }
            std::fs::rename(&part, stage.join(&f.name)).map_err(|e| e.to_string())?;
        }
        let name = manifest_name();
        std::fs::write(stage.join(&name), &text).map_err(|e| e.to_string())?;
        std::fs::write(stage.join(format!("{name}.sig")), &sig).map_err(|e| e.to_string())?;
        if !self_check_ok(&stage.join("wmw.exe")) {
            return Err("la versión nueva no arranca (self-check)".into());
        }
        std::fs::write(stage.join("ready"), v.to_string()).map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    Ok(Prepared::Ready(v))
}

/// La versión preparada (cualquiera, también la misma), para el instalador.
pub fn staged_dir_for(dir: &Path, v: Version) -> PathBuf {
    staged_dir(dir, v)
}

/// La versión preparada más nueva que la actual, si hay.
pub fn staged(dir: &Path) -> Option<(Version, PathBuf)> {
    let entries = std::fs::read_dir(dir.join("update")).ok()?;
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let v = Version::parse(&e.file_name().to_string_lossy())?;
            (v > current() && e.path().join("ready").exists()).then(|| (v, e.path()))
        })
        .max_by_key(|(v, _)| *v)
}

fn journal(dir: &Path) -> PathBuf {
    dir.join("update").join("journal")
}

fn run_steps(dir: &Path, steps: &[Step]) -> Result<(), String> {
    for s in steps {
        match s {
            Step::Rename { from, to } => std::fs::rename(dir.join(from), dir.join(to))
                .map_err(|e| format!("{from} → {to}: {e}"))?,
            Step::Remove(p) => {
                let path = dir.join(p);
                if std::fs::remove_file(&path).is_err() {
                    // En uso: se aparta, se borra en el próximo arranque.
                    let aside = path.with_extension(format!("del-{}", now_secs()));
                    std::fs::rename(&path, &aside).map_err(|e| format!("{p}: {e}"))?;
                }
            }
        }
    }
    Ok(())
}

/// Coloca la versión preparada (la llama el guardián, con el core cerrado).
/// Devuelve la versión colocada.
pub fn apply_staged(dir: &Path) -> Result<Option<Version>, String> {
    let Some((v, stage)) = staged(dir) else { return Ok(None) };
    if let Err(e) = verify_staged(&stage) {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(format!("la {v} preparada no pasó la verificación: {e}"));
    }
    let rel = format!("update\\{v}");
    std::fs::write(journal(dir), format!("applying {} {v}", current()))
        .map_err(|e| e.to_string())?;
    if let Err(e) = run_steps(dir, &update::swap_plan(&rel)) {
        rollback(dir, Some(v));
        return Err(format!("no se pudo colocar la {v}: {e}"));
    }
    std::fs::write(journal(dir), format!("swapped {} {v}", current()))
        .map_err(|e| e.to_string())?;
    Ok(Some(v))
}

/// Vuelve a la versión anterior (los `.old`) y, si se sabe, marca `bad` la
/// que falló para no volver a ofrecerla.
pub fn rollback(dir: &Path, bad: Option<Version>) {
    let steps = update::rollback_plan(&|p| dir.join(p).exists());
    if let Err(e) = run_steps(dir, &steps) {
        log(&format!("vuelta atrás incompleta: {e}"));
    }
    if let Some(v) = bad {
        let mut st = load_state();
        if !st.bad.contains(&v) {
            st.bad.push(v);
        }
        save_state(&st);
        let _ = std::fs::remove_dir_all(staged_dir(dir, v));
    }
    let _ = std::fs::remove_file(journal(dir));
}

/// La versión nueva arrancó bien: borrar el diario y lo preparado. Los `.old`
/// se borran en el próximo arranque (el guardián viejo todavía corre).
pub fn commit(dir: &Path, v: Version) {
    let _ = std::fs::remove_file(journal(dir));
    let _ = std::fs::remove_dir_all(staged_dir(dir, v));
}

/// Al arrancar el guardián: un cambio que quedó a medias (`applying`) se
/// deshace; uno ya hecho (`swapped`) sigue en prueba (devuelve la versión
/// de antes y la nueva). Además, borra `.old` y restos viejos.
pub fn recover_at_start(dir: &Path) -> Option<(Version, Version)> {
    let text = std::fs::read_to_string(journal(dir)).ok();
    let parsed = text.as_deref().and_then(|t| {
        let mut it = t.split_whitespace();
        let kind = it.next()?.to_owned();
        Some((kind, Version::parse(it.next()?)?, Version::parse(it.next()?)?))
    });
    match parsed {
        Some((kind, _, new)) if kind == "applying" => {
            log(&format!("un cambio a la {new} quedó a medias: vuelvo a la versión anterior"));
            rollback(dir, None);
            None
        }
        Some((kind, old, new)) if kind == "swapped" && current() == new => Some((old, new)),
        Some(_) => {
            // `swapped` pero este binario no es el nuevo: ya se volvió atrás.
            let _ = std::fs::remove_file(journal(dir));
            None
        }
        None => {
            // Sin cambio en curso: limpiar lo viejo.
            for name in FILES {
                let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for e in entries.filter_map(Result::ok) {
                    if e.file_name().to_string_lossy().contains(".del-") {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Línea de comandos
// ---------------------------------------------------------------------------

/// `wm update check|run [--force] [--tag vX.Y.Z]|status`.
pub fn cli(args: &[String]) -> ExitCode {
    let flag = |n: &str| args.iter().any(|a| a == n);
    let tag = args.iter().position(|a| a == "--tag").and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        Some("status") => {
            let st = load_state();
            println!("Versión {} ({})", current(), arch());
            println!("Instalada acá: {}", installed_here());
            println!(
                "Carpeta de instalación: {}",
                install_dir().map(|d| d.display().to_string()).unwrap_or_default()
            );
            println!("Claves de verificación: {}", keys().len());
            println!("Actualizar sola: {}", st.auto);
            println!("Última búsqueda: {} (Unix)", st.last_check);
            if !st.bad.is_empty() {
                let bad: Vec<String> = st.bad.iter().map(Version::to_string).collect();
                println!("Versiones que fallaron: {}", bad.join(", "));
            }
            if let Some((v, _)) = install_dir().and_then(|d| staged(&d)) {
                println!("Lista para colocar: {v}");
            }
            ExitCode::SUCCESS
        }
        Some("check") => match fetch_manifest(&source(tag)) {
            Ok((_, _, m)) => {
                println!("Publicada: {} ({}). Esta: {}.", m.version, m.arch, current());
                match decide(current(), arch(), &m, &load_state()) {
                    Decision::Update(v) => println!("Hay una versión nueva: {v}."),
                    Decision::UpToDate => println!("Ya es la última."),
                    Decision::Bad(v) => println!("La {v} ya falló en esta PC: no se ofrece."),
                    Decision::WrongArch => println!("El manifiesto es de otra arquitectura."),
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("No se pudo buscar: {e}");
                ExitCode::FAILURE
            }
        },
        Some("run") => {
            // Solo la copia instalada (o con `--force`, que la usa el
            // instalador y las pruebas).
            if !installed_here() && !flag("--force") {
                println!("Esta copia no está instalada: no busca actualizaciones.");
                return ExitCode::SUCCESS;
            }
            let Some(dir) = install_dir() else { return ExitCode::FAILURE };
            lower_priority();
            match prepare(&dir, flag("--force"), tag) {
                Ok(Prepared::Ready(v)) => {
                    log(&format!("la versión {v} está lista para colocar"));
                    wm_platform::tray::send_order(wm_platform::tray::Order::UpdateReady);
                    ExitCode::SUCCESS
                }
                Ok(Prepared::UpToDate) => {
                    log(&format!("ya es la última versión ({})", current()));
                    ExitCode::SUCCESS
                }
                Ok(Prepared::Skipped(why)) => {
                    eprintln!("{why}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    log(&format!("no se pudo actualizar: {e}"));
                    ExitCode::from(1)
                }
            }
        }
        _ => {
            eprintln!(
                "Uso: wm update status | check [--tag vX.Y.Z] | run [--force] [--tag vX.Y.Z]"
            );
            ExitCode::from(2)
        }
    }
}

/// Prioridad de fondo: una descarga nunca le saca CPU ni disco a ella.
fn lower_priority() {
    wm_platform::session::background_mode();
}

/// `wm self-check`: arranca sin cargar nada y sale con 0 (prueba de que el
/// binario nuevo funciona antes de colocarlo).
pub fn self_check() -> ExitCode {
    ExitCode::SUCCESS
}

/// `wm release-pack <carpeta> --arch x64|arm64 --notes "…" --out <carpeta>`:
/// arma los archivos de un release (assets con su nombre y el manifiesto sin
/// firmar). Lo usa la CI.
pub fn release_pack(args: &[String]) -> ExitCode {
    let get = |n: &str| args.iter().position(|a| a == n).and_then(|i| args.get(i + 1)).cloned();
    let (Some(src), Some(arch), Some(out)) = (args.first().cloned(), get("--arch"), get("--out"))
    else {
        eprintln!(
            "Uso: wm release-pack <carpeta> --arch x64|arm64 [--notes \"…\"] --out <carpeta>"
        );
        return ExitCode::from(2);
    };
    let notes = get("--notes").unwrap_or_default();
    let (src, out) = (PathBuf::from(src), PathBuf::from(out));
    if std::fs::create_dir_all(&out).is_err() {
        return ExitCode::FAILURE;
    }
    let mut files = Vec::new();
    for name in FILES {
        let asset = format!("{}-{arch}.exe", name.trim_end_matches(".exe"));
        let Some((sha256, size)) = wm_platform::sign::sha256_file(&src.join(name)) else {
            eprintln!("No se pudo leer {name} en {}", src.display());
            return ExitCode::FAILURE;
        };
        if std::fs::copy(src.join(name), out.join(&asset)).is_err() {
            eprintln!("No se pudo copiar {name}");
            return ExitCode::FAILURE;
        }
        files.push(update::FileEntry { name: name.to_owned(), asset, size, sha256 });
    }
    let m = Manifest { version: current(), arch: arch.clone(), notes, files };
    if let Err(e) = parse_manifest(update::format_manifest(&m).as_bytes()) {
        eprintln!("Manifiesto inválido: {e}");
        return ExitCode::FAILURE;
    }
    let path = out.join(format!("latest-{arch}.txt"));
    if std::fs::write(&path, update::format_manifest(&m)).is_err() {
        return ExitCode::FAILURE;
    }
    // El instalador: el mismo binario que `wmw.exe`, con otro nombre.
    let _ = std::fs::copy(src.join("wmw.exe"), out.join(format!("wm-setup-{arch}.exe")));
    println!("{}", path.display());
    ExitCode::SUCCESS
}
