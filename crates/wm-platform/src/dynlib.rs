//! Funciones de DLL del sistema resueltas **en tiempo de ejecución**.
//!
//! Los wrappers del crate `windows` importan su DLL en forma estática: con
//! solo usar `D3D11CreateDevice`, `wm.exe` cargaría `d3d11.dll` en todos sus
//! roles (guardián, core, CLI), y si la función no existe en esa versión de
//! Windows, el programa ni arranca (architecture.md §13.7). Las funciones de
//! overlays, D3D, D2D y DirectWrite se piden acá, solo cuando hacen falta.

use windows::Win32::Foundation::FARPROC;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::core::{Error, HRESULT, PCSTR, PCWSTR};

/// Carga `dll` desde System32 (si ya estaba cargada, la reutiliza) y busca
/// `name`. La DLL queda cargada para siempre.
pub(crate) fn proc(dll: PCWSTR, name: PCSTR) -> windows::core::Result<FARPROC> {
    // SAFETY: solo se buscan DLL del sistema en System32.
    let module = unsafe { LoadLibraryExW(dll, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }?;
    // SAFETY: módulo válido recién cargado.
    let f = unsafe { GetProcAddress(module, name) };
    if f.is_none() {
        // ERROR_PROC_NOT_FOUND: la función no existe en esta versión de Windows.
        return Err(Error::from_hresult(HRESULT::from_win32(127)));
    }
    Ok(f)
}
