//! Actualizaciones (hito U, `plan-update.md`): la parte pura. Versiones, el
//! manifiesto que publica cada release, la decisión de actualizar y el plan
//! de renombres para cambiar los ejecutables sin quedar nunca a medias.
//!
//! Nada de esto toca la red ni el disco: lo usa `app/src/update.rs`.

use std::fmt;

/// Los tres ejecutables, en el orden en que se reemplazan: `wmw.exe` (el
/// guardián que arranca todo) al final.
pub const FILES: [&str; 3] = ["wm-settings.exe", "wm.exe", "wmw.exe"];

/// Tope del manifiesto (y de su firma) al bajarlo.
pub const MANIFEST_MAX: usize = 16 * 1024;

/// Tope de cada ejecutable al bajarlo.
pub const FILE_MAX: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Estricto: `N.N.N`, sin `v`, sin sufijos.
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.trim().split('.');
        let mut n = || -> Option<u32> {
            let p = it.next()?;
            if p.is_empty() || p.len() > 9 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let v = Self { major: n()?, minor: n()?, patch: n()? };
        if it.next().is_some() {
            return None;
        }
        Some(v)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Nombre instalado (uno de [`FILES`]).
    pub name: String,
    /// Nombre del archivo en el release.
    pub asset: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    pub arch: String,
    pub notes: String,
    /// Los tres, en el orden de [`FILES`].
    pub files: Vec<FileEntry>,
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn valid_asset(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('.')
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// Lee un manifiesto. Rechaza cualquier cosa rara: es lo que decide qué se
/// baja y se ejecuta (aunque antes se verifica su firma).
pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest, String> {
    if bytes.len() > MANIFEST_MAX {
        return Err("manifiesto demasiado grande".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "manifiesto que no es texto")?;
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
    if lines.next() != Some("wm-update 1") {
        return Err("formato de manifiesto desconocido".into());
    }
    let (mut version, mut arch, mut notes) = (None, None, String::new());
    let mut files: Vec<FileEntry> = Vec::new();
    for line in lines {
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("línea inválida: {line}"));
        };
        let value = value.trim();
        match key.trim() {
            "version" => version = Some(Version::parse(value).ok_or("versión inválida")?),
            "arch" => {
                if !matches!(value, "x64" | "arm64") {
                    return Err(format!("arquitectura desconocida: {value}"));
                }
                arch = Some(value.to_owned());
            }
            "notes" => notes = value.chars().take(300).collect(),
            "file" => {
                let parts: Vec<&str> = value.split('|').map(str::trim).collect();
                let [name, asset, size, sha] = parts[..] else {
                    return Err(format!("archivo inválido: {value}"));
                };
                if !FILES.contains(&name) {
                    return Err(format!("archivo no permitido: {name}"));
                }
                if files.iter().any(|f| f.name == name) {
                    return Err(format!("archivo repetido: {name}"));
                }
                if !valid_asset(asset) {
                    return Err(format!("nombre de asset inválido: {asset}"));
                }
                let size: u64 = size.parse().map_err(|_| "tamaño inválido")?;
                if size == 0 || size > FILE_MAX {
                    return Err("tamaño fuera de rango".into());
                }
                let sha256 = hex32(sha).ok_or("hash inválido")?;
                files.push(FileEntry {
                    name: name.to_owned(),
                    asset: asset.to_owned(),
                    size,
                    sha256,
                });
            }
            _ => {} // claves desconocidas: se ignoran (formatos futuros)
        }
    }
    let version = version.ok_or("falta la versión")?;
    let arch = arch.ok_or("falta la arquitectura")?;
    let mut ordered = Vec::new();
    for name in FILES {
        let i = files.iter().position(|f| f.name == name).ok_or(format!("falta {name}"))?;
        ordered.push(files.swap_remove(i));
    }
    Ok(Manifest { version, arch, notes, files: ordered })
}

/// Escribe un manifiesto (lo usa `wm release-pack`).
pub fn format_manifest(m: &Manifest) -> String {
    let mut s = format!("wm-update 1\nversion = {}\narch = {}\n", m.version, m.arch);
    if !m.notes.is_empty() {
        s.push_str(&format!("notes = {}\n", m.notes.replace('\n', " ")));
    }
    for f in &m.files {
        s.push_str(&format!("file = {} | {} | {} | {}\n", f.name, f.asset, f.size, hex(&f.sha256)));
    }
    s
}

/// Firma DER de openssl (`SEQUENCE { INTEGER r, INTEGER s }`) a `r ‖ s` de
/// 32 + 32 bytes, que es lo que verifica CNG. Estricto.
pub fn der_to_raw(der: &[u8]) -> Option<[u8; 64]> {
    let (&tag, rest) = der.split_first()?;
    if tag != 0x30 {
        return None;
    }
    let (&len, mut rest) = rest.split_first()?;
    if usize::from(len) != rest.len() || len >= 0x80 {
        return None;
    }
    let mut out = [0u8; 64];
    for half in 0..2 {
        let (&t, r) = rest.split_first()?;
        let (&l, r) = r.split_first()?;
        let l = usize::from(l);
        if t != 0x02 || l == 0 || l > 33 || r.len() < l {
            return None;
        }
        let mut int = &r[..l];
        if int.len() == 33 {
            if int[0] != 0 {
                return None;
            }
            int = &int[1..];
        }
        out[half * 32 + 32 - int.len()..half * 32 + 32].copy_from_slice(int);
        rest = &r[l..];
    }
    rest.is_empty().then_some(out)
}

/// Lo que se recuerda entre búsquedas (`%LOCALAPPDATA%\wm\update.txt`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// Descargar y avisar sola (si no, solo cuando ella lo pide).
    pub auto: bool,
    /// Última búsqueda (segundos Unix).
    pub last_check: u64,
    /// Versiones que no arrancaron: no se vuelven a ofrecer.
    pub bad: Vec<Version>,
    /// La última versión avisada y cuándo.
    pub notified: Option<(Version, u64)>,
}

impl Default for State {
    fn default() -> Self {
        Self { auto: true, last_check: 0, bad: Vec::new(), notified: None }
    }
}

impl State {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "auto" => s.auto = v != "0",
                "last_check" => s.last_check = v.parse().unwrap_or(0),
                "bad" => s.bad = v.split(',').filter_map(Version::parse).collect(),
                "notified" => {
                    s.notified = v
                        .split_once(' ')
                        .and_then(|(a, b)| Some((Version::parse(a)?, b.trim().parse().ok()?)))
                }
                _ => {}
            }
        }
        s
    }

    pub fn format(&self) -> String {
        let bad: Vec<String> = self.bad.iter().map(Version::to_string).collect();
        let mut s = format!(
            "auto = {}\nlast_check = {}\nbad = {}\n",
            u8::from(self.auto),
            self.last_check,
            bad.join(",")
        );
        if let Some((v, t)) = self.notified {
            s.push_str(&format!("notified = {v} {t}\n"));
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    UpToDate,
    /// La versión publicada ya falló en esta PC.
    Bad(Version),
    WrongArch,
    Update(Version),
}

/// ¿Hay que bajar este manifiesto? Solo una versión mayor (nunca se baja de
/// versión), de la misma arquitectura y que no haya fallado antes.
pub fn decide(current: Version, arch: &str, m: &Manifest, state: &State) -> Decision {
    if m.arch != arch {
        return Decision::WrongArch;
    }
    if m.version <= current {
        return Decision::UpToDate;
    }
    if state.bad.contains(&m.version) {
        return Decision::Bad(m.version);
    }
    Decision::Update(m.version)
}

/// Un paso del cambio de archivos (rutas relativas a la carpeta instalada).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Rename { from: String, to: String },
    Remove(String),
}

/// Colocar la versión preparada en `update\<v>\`: cada archivo actual pasa a
/// `.old` y el nuevo ocupa su lugar, `wmw.exe` al final.
pub fn swap_plan(staged_dir: &str) -> Vec<Step> {
    let mut out = Vec::new();
    for name in FILES {
        out.push(Step::Rename { from: name.to_owned(), to: format!("{name}.old") });
        out.push(Step::Rename { from: format!("{staged_dir}\\{name}"), to: name.to_owned() });
    }
    out
}

/// Volver a la versión anterior (tras una falla, o si se cortó a mitad de
/// `swap_plan`): por cada archivo con `.old`, el actual se borra y el `.old`
/// vuelve a su nombre. `exists` dice qué hay en el disco.
pub fn rollback_plan(exists: &dyn Fn(&str) -> bool) -> Vec<Step> {
    let mut out = Vec::new();
    for name in FILES.iter().rev() {
        let old = format!("{name}.old");
        if exists(&old) {
            if exists(name) {
                out.push(Step::Remove((*name).to_owned()));
            }
            out.push(Step::Rename { from: old, to: (*name).to_owned() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const SHA: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn manifest_text(extra: &str) -> String {
        format!(
            "wm-update 1\nversion = 0.2.0\narch = x64\nnotes = Algo.\n\
             file = wmw.exe | wmw-x64.exe | 10 | {SHA}\n\
             file = wm.exe | wm-x64.exe | 20 | {SHA}\n\
             file = wm-settings.exe | wm-settings-x64.exe | 30 | {SHA}\n{extra}"
        )
    }

    #[test]
    fn versions() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.to_string(), "1.2.3");
        assert!(Version::parse("0.10.0").unwrap() > Version::parse("0.9.9").unwrap());
        for bad in ["v1.2.3", "1.2", "1.2.3.4", "1.2.x", "", "1..3", "-1.2.3", "1.2.3-beta"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_good_manifest_round_trips() {
        let m = parse_manifest(manifest_text("futuro = algo\n").as_bytes()).unwrap();
        assert_eq!(m.version, Version::parse("0.2.0").unwrap());
        assert_eq!(m.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), FILES);
        assert_eq!(parse_manifest(format_manifest(&m).as_bytes()).unwrap(), m);
    }

    #[test]
    fn bad_manifests_are_rejected() {
        let good = manifest_text("");
        let cases = [
            good.replace("wm-update 1", "wm-update 2"),
            good.replace("arch = x64", "arch = x86"),
            good.replace("version = 0.2.0", "version = 0.2"),
            good.replace("wm-settings.exe | wm-settings", "evil.exe | wm-settings"),
            good.replace("wmw-x64.exe", "../wmw.exe"),
            good.replace("wmw-x64.exe", "WMW.exe"),
            good.replace("| 10 |", "| 0 |"),
            good.replace(&format!("| 10 | {SHA}"), "| 10 | 0011"),
            good.replace("file = wm.exe", "# file = wm.exe"),
            manifest_text(&format!("file = wm.exe | otro.exe | 5 | {SHA}\n")),
            "x".repeat(MANIFEST_MAX + 1),
        ];
        for c in cases {
            assert!(parse_manifest(c.as_bytes()).is_err(), "aceptó:\n{c}");
        }
    }

    #[test]
    fn der_signatures() {
        // r con el bit alto (33 bytes con un 0 adelante) y s corto (31 bytes).
        let mut der = vec![0x30, 0, 0x02, 33, 0x00];
        der.extend([0x80; 32]);
        der.extend([0x02, 31]);
        der.extend([0x11; 31]);
        der[1] = (der.len() - 2) as u8;
        let raw = der_to_raw(&der).unwrap();
        assert_eq!(&raw[..32], &[0x80; 32]);
        assert_eq!(raw[32], 0);
        assert_eq!(&raw[33..], &[0x11; 31]);
        assert_eq!(der_to_raw(&der[..der.len() - 1]), None, "truncada");
        let mut extra = der.clone();
        extra.push(0);
        assert_eq!(der_to_raw(&extra), None, "basura al final");
    }

    #[test]
    fn decisions() {
        let m = parse_manifest(manifest_text("").as_bytes()).unwrap();
        let v = |s| Version::parse(s).unwrap();
        let mut st = State::default();
        assert_eq!(decide(v("0.1.0"), "x64", &m, &st), Decision::Update(v("0.2.0")));
        assert_eq!(decide(v("0.2.0"), "x64", &m, &st), Decision::UpToDate);
        assert_eq!(decide(v("0.3.0"), "x64", &m, &st), Decision::UpToDate, "nunca bajar");
        assert_eq!(decide(v("0.1.0"), "arm64", &m, &st), Decision::WrongArch);
        st.bad.push(v("0.2.0"));
        assert_eq!(decide(v("0.1.0"), "x64", &m, &st), Decision::Bad(v("0.2.0")));
    }

    #[test]
    fn state_round_trips() {
        let st = State {
            auto: false,
            last_check: 1_700_000_000,
            bad: vec![Version::parse("0.2.0").unwrap(), Version::parse("0.3.1").unwrap()],
            notified: Some((Version::parse("0.4.0").unwrap(), 1_700_000_100)),
        };
        assert_eq!(State::parse(&st.format()), st);
        assert_eq!(State::parse("basura"), State::default());
    }

    /// Cortar el cambio en cualquier paso y volver atrás deja los tres
    /// archivos de la versión anterior; terminarlo deja los tres nuevos.
    #[test]
    fn a_swap_cut_anywhere_rolls_back_cleanly() {
        let plan = swap_plan("update\\0.2.0");
        for cut in 0..=plan.len() {
            let mut fs: HashMap<String, &str> = HashMap::new();
            for n in FILES {
                fs.insert(n.to_owned(), "old");
                fs.insert(format!("update\\0.2.0\\{n}"), "new");
            }
            for step in &plan[..cut] {
                apply(&mut fs, step);
            }
            if cut == plan.len() {
                assert!(FILES.iter().all(|n| fs[*n] == "new"));
            }
            let snapshot = fs.clone();
            for step in rollback_plan(&|p| snapshot.contains_key(p)) {
                apply(&mut fs, &step);
            }
            for n in FILES {
                assert_eq!(fs.get(n), Some(&"old"), "corte en {cut}: {n}");
                assert!(!fs.contains_key(&format!("{n}.old")));
            }
        }
    }

    fn apply(fs: &mut HashMap<String, &'static str>, step: &Step) {
        match step {
            Step::Rename { from, to } => {
                if let Some(v) = fs.remove(from) {
                    fs.insert(to.clone(), v);
                }
            }
            Step::Remove(p) => {
                fs.remove(p);
            }
        }
    }
}
