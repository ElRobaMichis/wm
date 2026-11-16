//! Reloj de alta resolución (QPC) para medir latencias.

use std::sync::atomic::{AtomicI64, Ordering};

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

static FREQ: AtomicI64 = AtomicI64::new(0);

/// Instante actual en ticks de QPC.
pub fn now() -> i64 {
    let mut t = 0i64;
    // SAFETY: puntero a una variable local válida.
    let _ = unsafe { QueryPerformanceCounter(&mut t) };
    t
}

fn freq() -> i64 {
    let f = FREQ.load(Ordering::Relaxed);
    if f > 0 {
        return f;
    }
    let mut f = 0i64;
    // SAFETY: puntero a una variable local válida.
    let _ = unsafe { QueryPerformanceFrequency(&mut f) };
    let f = f.max(1);
    FREQ.store(f, Ordering::Relaxed);
    f
}

/// Microsegundos entre dos instantes de QPC (0 si `later` es anterior).
pub fn micros(earlier: i64, later: i64) -> u64 {
    let d = i128::from((later - earlier).max(0));
    (d * 1_000_000 / i128::from(freq())) as u64
}

/// Microsegundos desde `earlier` hasta ahora.
pub fn micros_since(earlier: i64) -> u64 {
    micros(earlier, now())
}

/// El instante `us` microsegundos después de `qpc`.
pub fn plus_micros(qpc: i64, us: u64) -> i64 {
    qpc + (i128::from(us) * i128::from(freq()) / 1_000_000) as i64
}

/// Fecha y hora local, para el log: `2026-11-16 04:21:07.123`.
pub fn local_stamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: sin precondiciones.
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}
