//! Descargas HTTPS con WinHTTP (hito U, `plan-update.md` U2), resuelto en
//! tiempo de ejecución: `wm.exe` no importa `winhttp.dll` y el guardián y el
//! core nunca la cargan (solo el rol efímero `update run`).
//!
//! Reglas: solo `https://`; redirecciones como WinHTTP por defecto (nunca de
//! https a http); corta al pasar el máximo; User-Agent `wm` sin versión ni
//! nada que identifique a nadie; proxy automático del sistema.

use std::ffi::c_void;
use std::io::Write;
use std::time::Duration;

use windows::core::{BOOL, HSTRING, PCSTR, PCWSTR, w};

type H = *mut c_void;
type Open = unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, u32) -> H;
type SetTimeouts = unsafe extern "system" fn(H, i32, i32, i32, i32) -> BOOL;
type Connect = unsafe extern "system" fn(H, PCWSTR, u16, u32) -> H;
type OpenRequest =
    unsafe extern "system" fn(H, PCWSTR, PCWSTR, PCWSTR, PCWSTR, *const PCWSTR, u32) -> H;
type Send = unsafe extern "system" fn(H, PCWSTR, u32, *const c_void, u32, u32, usize) -> BOOL;
type Receive = unsafe extern "system" fn(H, *mut c_void) -> BOOL;
type QueryHeaders =
    unsafe extern "system" fn(H, u32, PCWSTR, *mut c_void, *mut u32, *mut u32) -> BOOL;
type ReadData = unsafe extern "system" fn(H, *mut c_void, u32, *mut u32) -> BOOL;
type Close = unsafe extern "system" fn(H) -> BOOL;

type Raw = unsafe extern "system" fn() -> isize;

/// Puntero de `GetProcAddress` a la firma real `T` (otro puntero a función).
///
/// # Safety
/// `T` tiene que ser la firma verdadera de la función.
unsafe fn cast<T: Copy>(f: Raw) -> T {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<Raw>());
    // SAFETY: mismo tamaño (punteros a función); la firma la garantiza quien llama.
    unsafe { std::mem::transmute_copy(&f) }
}

struct Api {
    open: Open,
    timeouts: SetTimeouts,
    connect: Connect,
    request: OpenRequest,
    send: Send,
    receive: Receive,
    headers: QueryHeaders,
    read: ReadData,
    close: Close,
}

fn api() -> Option<Api> {
    let f =
        |name: &[u8]| crate::dynlib::proc(w!("winhttp.dll"), PCSTR(name.as_ptr())).ok().flatten();
    // SAFETY: firmas de winhttp.h; cada puntero viene de la función de ese nombre.
    unsafe {
        Some(Api {
            open: cast(f(b"WinHttpOpen\0")?),
            timeouts: cast(f(b"WinHttpSetTimeouts\0")?),
            connect: cast(f(b"WinHttpConnect\0")?),
            request: cast(f(b"WinHttpOpenRequest\0")?),
            send: cast(f(b"WinHttpSendRequest\0")?),
            receive: cast(f(b"WinHttpReceiveResponse\0")?),
            headers: cast(f(b"WinHttpQueryHeaders\0")?),
            read: cast(f(b"WinHttpReadData\0")?),
            close: cast(f(b"WinHttpCloseHandle\0")?),
        })
    }
}

/// Por qué falló una descarga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// Sin internet, DNS, proxy, TLS o plazo vencido.
    Unreachable(String),
    /// El servidor respondió con otro código (404: no hay release).
    Status(u32),
    /// Más grande que lo permitido.
    TooBig,
    BadUrl,
    Write(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Unreachable(e) => write!(f, "sin conexión ({e})"),
            NetError::Status(c) => write!(f, "el servidor respondió {c}"),
            NetError::TooBig => write!(f, "la descarga es más grande de lo esperado"),
            NetError::BadUrl => write!(f, "dirección inválida"),
            NetError::Write(e) => write!(f, "no se pudo guardar ({e})"),
        }
    }
}

struct Handles<'a> {
    api: &'a Api,
    list: Vec<H>,
}

impl Drop for Handles<'_> {
    fn drop(&mut self) {
        for h in self.list.iter().rev() {
            // SAFETY: handles propios de WinHTTP.
            let _ = unsafe { (self.api.close)(*h) };
        }
    }
}

fn last_error() -> String {
    windows::core::Error::from_thread().message().to_string()
}

/// Baja `url` (solo `https://`) a `out`, como mucho `max` bytes y con plazo
/// `timeout` por operación. Devuelve cuántos bytes escribió.
pub fn get(url: &str, max: u64, timeout: Duration, out: &mut dyn Write) -> Result<u64, NetError> {
    let rest = url.strip_prefix("https://").ok_or(NetError::BadUrl)?;
    let (host, path) =
        rest.split_once('/').map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
    if host.is_empty() || host.contains(['@', ':', ' ']) {
        return Err(NetError::BadUrl);
    }
    let api = api().ok_or_else(|| NetError::Unreachable("WinHTTP no disponible".into()))?;
    let mut hs = Handles { api: &api, list: Vec::new() };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    const ACCESS_AUTOMATIC_PROXY: u32 = 4;
    const ACCESS_DEFAULT_PROXY: u32 = 0;
    const FLAG_SECURE: u32 = 0x0080_0000;
    const QUERY_STATUS_NUMBER: u32 = 19 | 0x2000_0000;
    // SAFETY: cadenas válidas; los handles se cierran en `Handles::drop`.
    unsafe {
        let mut session =
            (api.open)(w!("wm"), ACCESS_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() {
            // Windows anterior a 8.1: sin proxy automático.
            session = (api.open)(w!("wm"), ACCESS_DEFAULT_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        }
        if session.is_null() {
            return Err(NetError::Unreachable(last_error()));
        }
        hs.list.push(session);
        let _ = (api.timeouts)(session, ms, ms, ms, ms);
        let host_w = HSTRING::from(host);
        let connect = (api.connect)(session, PCWSTR(host_w.as_ptr()), 443, 0);
        if connect.is_null() {
            return Err(NetError::Unreachable(last_error()));
        }
        hs.list.push(connect);
        let path_w = HSTRING::from(path.as_str());
        let req = (api.request)(
            connect,
            w!("GET"),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            FLAG_SECURE,
        );
        if req.is_null() {
            return Err(NetError::Unreachable(last_error()));
        }
        hs.list.push(req);
        if !(api.send)(req, PCWSTR::null(), 0, std::ptr::null(), 0, 0, 0).as_bool()
            || !(api.receive)(req, std::ptr::null_mut()).as_bool()
        {
            return Err(NetError::Unreachable(last_error()));
        }
        let mut status = 0u32;
        let mut len = 4u32;
        if !(api.headers)(
            req,
            QUERY_STATUS_NUMBER,
            PCWSTR::null(),
            &mut status as *mut u32 as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
        )
        .as_bool()
        {
            return Err(NetError::Unreachable(last_error()));
        }
        if status != 200 {
            return Err(NetError::Status(status));
        }
        let mut total = 0u64;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            if !(api.read)(req, buf.as_mut_ptr().cast(), buf.len() as u32, &mut read).as_bool() {
                return Err(NetError::Unreachable(last_error()));
            }
            if read == 0 {
                break;
            }
            total += u64::from(read);
            if total > max {
                return Err(NetError::TooBig);
            }
            out.write_all(&buf[..read as usize]).map_err(|e| NetError::Write(e.to_string()))?;
        }
        Ok(total)
    }
}
