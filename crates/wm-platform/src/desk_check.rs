//! Verificaciones del hito 4.0 (plan-core.md §7): cómo se porta el cloak del
//! shell en los casos que el diseño de los escritorios daba por supuestos.
//!
//! Solo toca ventanas que lanza él mismo (títeres y una Calculadora nueva),
//! salvo en el modo manual, donde el usuario abre el Administrador de tareas.
//! Todo lo que esconde lo vuelve a mostrar; si se corta a mitad, `wm restore`
//! lo recupera (las marcas viven en las ventanas).

use std::ffi::c_void;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND,
    EnumWindows, GW_OWNER, GetClassNameW, GetPropW, GetShellWindow, GetWindow, IsWindow, MSG,
    PM_REMOVE, PeekMessageW, RemovePropW, SW_MINIMIZE, SW_RESTORE, SetPropW, ShowWindowAsync,
    TranslateMessage, WINEVENT_OUTOFCONTEXT,
};
use windows::core::{BOOL, w};

use crate::hide::{self, DWM_CLOAKED_SHELL, HideMethod, Hider};
use crate::windows_list::{self, WindowId};

/// Resultado de una verificación. `ok = None`: informativa (decide el diseño
/// pero no hay respuesta "correcta").
pub struct Check {
    pub name: &'static str,
    pub ok: Option<bool>,
    pub detail: String,
}

fn info(name: &'static str, detail: String) -> Check {
    Check { name, ok: None, detail }
}

fn verdict(name: &'static str, ok: bool, detail: String) -> Check {
    Check { name, ok: Some(ok), detail }
}

// ---------------------------------------------------------------------------
// WinEvents: se anotan todos para contarlos después.
// ---------------------------------------------------------------------------

static EVENTS: Mutex<Vec<(u32, isize)>> = Mutex::new(Vec::new());

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if object == 0
        && let Ok(mut e) = EVENTS.lock()
    {
        e.push((event, hwnd.0 as isize));
    }
}

struct Hooks(Vec<HWINEVENTHOOK>);

impl Hooks {
    fn install() -> Self {
        let mut hooks = Vec::new();
        for (a, b) in [
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
        ] {
            // SAFETY: callback con la firma de WINEVENTPROC; fuera de contexto,
            // se despacha en el bucle de este hilo (`pump`).
            let h =
                unsafe { SetWinEventHook(a, b, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT) };
            if !h.is_invalid() {
                hooks.push(h);
            }
        }
        Self(hooks)
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        for h in &self.0 {
            // SAFETY: hooks propios.
            let _ = unsafe { UnhookWinEvent(*h) };
        }
    }
}

fn clear_events() {
    pump(0);
    if let Ok(mut e) = EVENTS.lock() {
        e.clear();
    }
}

/// Cuántos eventos `event` hubo (de `hwnd`, o de cualquiera con `None`).
fn count(event: u32, hwnd: Option<WindowId>) -> usize {
    EVENTS
        .lock()
        .map(|e| e.iter().filter(|(ev, h)| *ev == event && hwnd.is_none_or(|w| w.0 == *h)).count())
        .unwrap_or(0)
}

/// Atiende mensajes (y con ellos los WinEvents) durante `ms`.
fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    loop {
        let mut msg = MSG::default();
        // SAFETY: bucle de mensajes estándar de este hilo.
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            // SAFETY: mensaje recién leído de la cola de este hilo.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Espera (atendiendo mensajes) hasta que `cond` se cumpla o pasen `ms`.
fn wait(ms: u64, mut cond: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + Duration::from_millis(ms);
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        pump(10);
    }
}

// ---------------------------------------------------------------------------
// Ventanas
// ---------------------------------------------------------------------------

/// Ventanas top-level (con o sin dueño) de la clase `class`.
fn windows_of_class(class: &str) -> Vec<WindowId> {
    struct Data<'a> {
        class: &'a str,
        found: Vec<WindowId>,
    }
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` es el `&mut Data` que pasamos abajo.
        let data = unsafe { &mut *(data.0 as *mut Data) };
        let mut name = [0u16; 64];
        // SAFETY: buffer local válido.
        let len = unsafe { GetClassNameW(hwnd, &mut name) };
        if String::from_utf16_lossy(&name[..len.max(0) as usize]) == data.class {
            data.found.push(WindowId(hwnd.0 as isize));
        }
        true.into()
    }
    let mut data = Data { class, found: Vec::new() };
    // SAFETY: el callback solo usa `data` mientras dura la llamada.
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut data as *mut Data as isize)) };
    data.found
}

fn owner(id: WindowId) -> Option<WindowId> {
    // SAFETY: lectura sobre un HWND; si no existe, falla sin efectos.
    unsafe { GetWindow(id.hwnd(), GW_OWNER) }
        .ok()
        .filter(|h| !h.is_invalid())
        .map(|h| WindowId(h.0 as isize))
}

fn class_name(id: WindowId) -> String {
    let mut name = [0u16; 64];
    // SAFETY: buffer local válido.
    let len = unsafe { GetClassNameW(id.hwnd(), &mut name) };
    String::from_utf16_lossy(&name[..len.max(0) as usize])
}

fn alive(id: WindowId) -> bool {
    // SAFETY: sin precondiciones.
    unsafe { IsWindow(Some(id.hwnd())) }.as_bool()
}

fn show_cmd(id: WindowId, cmd: windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD) {
    // SAFETY: asíncrona; no bloquea con apps colgadas.
    let _ = unsafe { ShowWindowAsync(id.hwnd(), cmd) };
}

fn cloaked(id: WindowId) -> u32 {
    hide::visibility(id).cloaked
}

fn describe(id: WindowId) -> String {
    let v = hide::visibility(id);
    format!("cloak={} visible={} minimizada={}", v.cloaked, v.visible, v.minimized)
}

/// Los procesos que lanzamos; se cierran (solo ellos, por su handle) al salir.
struct Children(Vec<Child>);

impl Children {
    fn spawn(&mut self, exe: &Path, args: &[&str]) -> Option<u32> {
        let child = Command::new(exe).args(args).spawn().ok()?;
        let pid = child.id();
        self.0.push(child);
        Some(pid)
    }
}

impl Drop for Children {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn puppets_of(pid: u32) -> Vec<WindowId> {
    // Por título ("Títere 1", "Títere 2"): no depende del orden de creación.
    let mut ws: Vec<WindowId> = crate::puppet::find_puppets()
        .into_iter()
        .filter(|&id| windows_list::window_pid(id) == pid)
        .collect();
    ws.sort_by_key(|&id| windows_list::window_title(id));
    ws
}

// ---------------------------------------------------------------------------
// Verificaciones automáticas
// ---------------------------------------------------------------------------

/// Corre las verificaciones que no necesitan al usuario. `exe` es `wm.exe`
/// (para lanzar los títeres).
pub fn run(exe: &Path) -> Vec<Check> {
    let mut out = Vec::new();
    let hider = match Hider::new(HideMethod::ShellCloak) {
        Ok(h) => h,
        Err(e) => {
            out.push(verdict("cloak disponible", false, e));
            return out;
        }
    };
    let _hooks = Hooks::install();
    let mut children = Children(Vec::new());

    // Ruido de CLOAKED/UNCLOAKED en reposo (decide si escucharlos sale caro).
    clear_events();
    pump(2000);
    out.push(info(
        "ruido en reposo",
        format!(
            "en 2 s: {} CLOAKED, {} UNCLOAKED, {} FOREGROUND de todo el sistema",
            count(EVENT_OBJECT_CLOAKED, None),
            count(EVENT_OBJECT_UNCLOAKED, None),
            count(EVENT_SYSTEM_FOREGROUND, None)
        ),
    ));

    let Some(pid) = children.spawn(exe, &["puppet", "2"]) else {
        out.push(verdict("títeres", false, "no se pudieron lanzar".into()));
        return out;
    };
    if !wait(5000, || puppets_of(pid).len() == 2) {
        out.push(verdict("títeres", false, "no aparecieron".into()));
        return out;
    }
    let p = puppets_of(pid);
    let (p1, p2) = (p[0], p[1]);
    pump(300);

    // (a) Nuestro propio cloak, ¿dispara CLOAKED/UNCLOAKED? (para `Reassert`:
    // hay que distinguir los nuestros de los del shell).
    clear_events();
    let t = Instant::now();
    let r = hider.hide(p1);
    let hid = wait(500, || cloaked(p1) == DWM_CLOAKED_SHELL);
    let ms_hide = t.elapsed().as_secs_f64() * 1000.0;
    pump(100);
    let c = count(EVENT_OBJECT_CLOAKED, Some(p1));
    let t = Instant::now();
    let _ = hider.show(p1);
    let shown = wait(500, || cloaked(p1) == 0);
    let ms_show = t.elapsed().as_secs_f64() * 1000.0;
    pump(100);
    let u = count(EVENT_OBJECT_UNCLOAKED, Some(p1));
    out.push(verdict(
        "cloak propio",
        r.is_ok() && hid && shown,
        format!(
            "esconder {ms_hide:.1} ms, mostrar {ms_show:.1} ms; eventos: {c} CLOAKED y {u} UNCLOAKED de la ventana"
        ),
    ));

    // Esconder la ventana que tiene el foco: ¿cuánto tarda DWM en confirmarlo?
    let mut times = Vec::new();
    for _ in 0..5 {
        let _ = windows_list::activate(p2);
        pump(100);
        let _ = windows_list::activate(p1);
        let t = Instant::now();
        let _ = hider.hide(p1);
        let ok = wait(1000, || cloaked(p1) == DWM_CLOAKED_SHELL);
        times.push(if ok {
            format!("{:.0}", t.elapsed().as_secs_f64() * 1000.0)
        } else {
            "nunca".into()
        });
        let _ = hider.show(p1);
        wait(500, || cloaked(p1) == 0);
        pump(50);
    }
    // En lote, con la del frente en el medio (falla vista en el ci-probe,
    // donde antes se la escondió con `minimize`, `offscreen` y `hide`).
    for m in [HideMethod::Minimize, HideMethod::Offscreen, HideMethod::Hide] {
        if let Ok(h) = Hider::new(m) {
            let _ = h.hide(p2);
            pump(150);
            let _ = h.show(p2);
            pump(150);
        }
    }
    let _ = windows_list::activate(p2);
    pump(100);
    let wins = [(p1, windows_list::window_pid(p1)), (p2, windows_list::window_pid(p2))];
    let r = hider.hide_many(&wins);
    let t = Instant::now();
    let late = wait(2000, || cloaked(p2) == DWM_CLOAKED_SHELL);
    out.push(info(
        "lote con la del frente",
        format!(
            "{:?}; la del frente con cloak {} ms después del lote ({late}); al frente queda {}",
            r.iter().map(|(_, r)| r.is_ok()).collect::<Vec<_>>(),
            t.elapsed().as_millis(),
            windows_list::foreground().map(class_name).unwrap_or_default()
        ),
    ));
    let _ = hider.show_many(&wins);
    out.push(info(
        "esconder la recién activada",
        format!("cloak confirmado a los {} ms", times.join(", ")),
    ));

    // (ii, aproximación) Foco a una ventana con cloak, como el clic en la
    // barra de tareas: ¿se destapa sola? ¿llega FOREGROUND?
    // Primero el foco a otra: si la escondida ya estaba al frente, activarla
    // no cambia nada (ni evento ni destape).
    let _ = windows_list::activate(p1);
    pump(100);
    let _ = hider.hide(p2);
    wait(500, || cloaked(p2) == DWM_CLOAKED_SHELL);
    clear_events();
    let a = windows_list::activate(p2);
    pump(200);
    let fg = windows_list::foreground() == Some(p2);
    let ev = count(EVENT_SYSTEM_FOREGROUND, Some(p2));
    let un = count(EVENT_OBJECT_UNCLOAKED, Some(p2));
    out.push(info(
        "foco a una escondida",
        format!(
            "SetForegroundWindow {} ; al frente: {fg}; FOREGROUND: {ev}; UNCLOAKED: {un}; queda {}",
            if a.ok { "aceptado" } else { "rechazado" },
            describe(p2)
        ),
    ));
    let _ = hider.show(p2);
    wait(500, || cloaked(p2) == 0);

    // (v) Minimizada + cloak: ¿vuelve minimizada y sin cloak?
    show_cmd(p1, SW_MINIMIZE);
    wait(1000, || hide::visibility(p1).minimized);
    let _ = hider.hide(p1);
    let hidden_min = wait(500, || cloaked(p1) == DWM_CLOAKED_SHELL);
    let while_hidden = describe(p1);
    let _ = hider.show(p1);
    let back = wait(500, || cloaked(p1) == 0);
    let v = hide::visibility(p1);
    show_cmd(p1, SW_RESTORE);
    let restored = wait(1000, || hide::fully_visible(hide::visibility(p1)));
    out.push(verdict(
        "minimizada + cloak",
        hidden_min && back && v.minimized && restored,
        format!(
            "escondida: {while_hidden}; al mostrar: {}; restaurarla después: {}",
            describe_v(v),
            if restored { "se ve" } else { "NO se ve" }
        ),
    ));

    // (iv) Foco al shell (escritorio vacío).
    // SAFETY: sin precondiciones.
    let shell = unsafe { GetShellWindow() };
    if shell.is_invalid() {
        out.push(verdict("foco al shell", false, "GetShellWindow no devolvió nada".into()));
    } else {
        let shell = WindowId(shell.0 as isize);
        let a = windows_list::activate(shell);
        pump(100);
        let fg = windows_list::foreground();
        out.push(verdict(
            "foco al shell",
            a.ok,
            format!(
                "activar {} ({}): {}; al frente queda {}",
                class_name(shell),
                if a.used_neutral_key { "con tecla neutra" } else { "directo" },
                if a.ok { "sí" } else { "no" },
                fg.map(class_name).unwrap_or_else(|| "nada".into())
            ),
        ));
    }

    // (viii) ¿A cuál activa Windows al minimizar o cerrar la del frente,
    // si hay una escondida más arriba en Z que una visible?
    out.push(z_order_check(&hider, exe, &mut children));

    // ¿La espera del brain (MsgWaitForMultipleObjectsEx) despierta con un
    // WinEvent fuera de contexto?
    out.push(wake_check(p1, p2));

    // (i) Diálogo con dueño.
    out.push(owned_check(&hider, exe, &mut children));

    // (vii) Pantalla completa (sin borde, como un video o un juego en ventana).
    out.push(fullscreen_check(&hider, exe, &mut children));

    // (vi) Una UWP (Calculadora).
    out.push(uwp_check(&hider));

    drop(children);
    out
}

fn describe_v(v: hide::Visibility) -> String {
    format!("cloak={} visible={} minimizada={}", v.cloaked, v.visible, v.minimized)
}

fn z_order_check(hider: &Hider, exe: &Path, children: &mut Children) -> Check {
    const NAME: &str = "foco tras minimizar o cerrar";
    // Tres procesos de un títere cada uno, para poder cerrar uno solo.
    let mut wins = Vec::new();
    for _ in 0..3 {
        let Some(pid) = children.spawn(exe, &["puppet", "1"]) else {
            return verdict(NAME, false, "no se pudieron lanzar los títeres".into());
        };
        if !wait(5000, || puppets_of(pid).len() == 1) {
            return verdict(NAME, false, "no apareció un títere".into());
        }
        wins.push(puppets_of(pid)[0]);
    }
    pump(300);
    let name = |w: WindowId, wins: &[WindowId]| match wins.iter().position(|&x| x == w) {
        Some(0) => "la visible (abajo)".to_string(),
        Some(1) => "LA ESCONDIDA".to_string(),
        Some(2) => "la del frente".to_string(),
        _ => w.map_or_class(),
    };
    let mut result = Vec::new();
    for case in ["minimizar", "cerrar"] {
        // Orden Z: visible (0) < escondida (1) < frente (2).
        for &w in &wins {
            let _ = windows_list::activate(w);
            pump(80);
        }
        let _ = hider.hide(wins[1]);
        wait(500, || cloaked(wins[1]) == DWM_CLOAKED_SHELL);
        let _ = windows_list::activate(wins[2]);
        pump(150);
        if case == "minimizar" {
            show_cmd(wins[2], SW_MINIMIZE);
        } else {
            windows_list::close(wins[2]);
        }
        pump(400);
        let fg = windows_list::foreground();
        result.push(format!(
            "{case}: queda al frente {}; la escondida {}",
            fg.map(|w| name(w, &wins)).unwrap_or_else(|| "nada".into()),
            describe(wins[1])
        ));
        let _ = hider.show(wins[1]);
        wait(500, || cloaked(wins[1]) == 0);
        if case == "minimizar" {
            show_cmd(wins[2], SW_RESTORE);
            wait(1000, || hide::fully_visible(hide::visibility(wins[2])));
        }
    }
    info(NAME, result.join(" · "))
}

fn wake_check(a: WindowId, b: WindowId) -> Check {
    use windows::Win32::UI::WindowsAndMessaging::{
        MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, QS_ALLINPUT,
    };
    const NAME: &str = "WinEvent despierta la espera";
    let _ = windows_list::activate(a);
    pump(300);
    clear_events();
    pump(0);
    // Otro hilo cambia el foco a los 150 ms; este espera como el brain.
    let target = b.0;
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        let _ = windows_list::activate(WindowId(target));
    });
    let start = Instant::now();
    // SAFETY: sin handles; espera mensajes de la cola de este hilo.
    let r = unsafe { MsgWaitForMultipleObjectsEx(None, 2000, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
    let woke_ms = start.elapsed().as_secs_f64() * 1000.0;
    let _ = t.join();
    pump(50);
    let got = count(EVENT_SYSTEM_FOREGROUND, Some(b));
    let woke = r.0 != 0x102;
    verdict(
        NAME,
        woke && got > 0,
        format!(
            "{} a los {woke_ms:.0} ms; FOREGROUND recibido después: {got}",
            if woke { "despertó" } else { "NO despertó (venció el plazo)" }
        ),
    )
}

trait MapOrClass {
    fn map_or_class(self) -> String;
}

impl MapOrClass for WindowId {
    fn map_or_class(self) -> String {
        format!("otra ({})", class_name(self))
    }
}

fn owned_check(hider: &Hider, exe: &Path, children: &mut Children) -> Check {
    const NAME: &str = "diálogo con dueño";
    let Some(pid) = children.spawn(exe, &["puppet-hard", "owned"]) else {
        return verdict(NAME, false, "no se pudo lanzar el títere".into());
    };
    let mine = || -> Vec<WindowId> {
        windows_of_class("wm-puppet-hard")
            .into_iter()
            .filter(|&id| windows_list::window_pid(id) == pid)
            .collect()
    };
    if !wait(5000, || mine().len() == 2) {
        return verdict(NAME, false, "no aparecieron las dos ventanas".into());
    }
    pump(300);
    let ws = mine();
    let (Some(&main), Some(&dialog)) =
        (ws.iter().find(|&&w| owner(w).is_none()), ws.iter().find(|&&w| owner(w).is_some()))
    else {
        return verdict(NAME, false, "no se distinguió el diálogo".into());
    };
    let in_list = windows_list::user_windows().iter().any(|w| w.id == dialog);
    let _ = hider.hide(main);
    wait(500, || cloaked(main) == DWM_CLOAKED_SHELL);
    pump(150);
    let follows = cloaked(dialog);
    let dialog_state = describe(dialog);
    let _ = hider.show(main);
    wait(500, || cloaked(main) == 0);
    pump(150);
    let after = cloaked(dialog);
    // Si no lo arrastra: ¿se puede esconder el diálogo aparte?
    let mut alone = String::new();
    if follows == 0 {
        let r = hider.hide(dialog);
        let ok = wait(500, || cloaked(dialog) == DWM_CLOAKED_SHELL);
        let _ = hider.show(dialog);
        let back = wait(500, || cloaked(dialog) == 0);
        alone = format!(
            "; escondido aparte: {} (vuelve: {back})",
            match (r, ok) {
                (Ok(()), true) => "sí".to_string(),
                (Ok(()), false) => "pedido aceptado pero sin cloak".to_string(),
                (Err(e), _) => format!("error {e}"),
            }
        );
    }
    info(
        NAME,
        format!(
            "con el dueño escondido, el diálogo queda {dialog_state} ({}); al mostrar el dueño: cloak={after}; en la lista de ventanas de usuario: {in_list}{alone}",
            if follows != 0 { "lo arrastra" } else { "NO lo arrastra" }
        ),
    )
}

fn fullscreen_check(hider: &Hider, exe: &Path, children: &mut Children) -> Check {
    const NAME: &str = "pantalla completa";
    let Some(pid) = children.spawn(exe, &["puppet-hard", "borderless"]) else {
        return verdict(NAME, false, "no se pudo lanzar el títere".into());
    };
    let mut id = None;
    if !wait(5000, || {
        id = crate::puppet::find_hard_puppet(pid);
        id.is_some()
    }) {
        return verdict(NAME, false, "no apareció".into());
    }
    let id = id.unwrap_or(WindowId(0));
    let Some(frame) = crate::ghost::frame_bounds(id) else {
        return verdict(NAME, false, "sin marco".into());
    };
    let Some(m) = crate::monitors().into_iter().find(|m| {
        let b = m.bounds;
        let (cx, cy) = ((frame.left + frame.right) / 2, (frame.top + frame.bottom) / 2);
        cx >= b.left && cx < b.right && cy >= b.top && cy < b.bottom
    }) else {
        return verdict(NAME, false, "sin monitor".into());
    };
    let _ = windows_list::place_async(id, m.bounds);
    if !wait(1000, || crate::ghost::frame_bounds(id) == Some(m.bounds)) {
        return verdict(NAME, false, "no llegó a pantalla completa".into());
    }
    let _ = hider.hide(id);
    let hid = wait(500, || cloaked(id) == DWM_CLOAKED_SHELL);
    let _ = hider.show(id);
    let back = wait(500, || cloaked(id) == 0);
    pump(100);
    let same = crate::ghost::frame_bounds(id) == Some(m.bounds);
    verdict(
        NAME,
        hid && back && same,
        format!("se esconde: {hid}; vuelve: {back}; sigue cubriendo el monitor: {same}"),
    )
}

fn uwp_check(hider: &Hider) -> Check {
    const NAME: &str = "app UWP (Configuración)";
    const CLASS: &str = "ApplicationFrameWindow";
    // Las que ya se veían antes no se cierran al final (pueden ser del usuario).
    let before: Vec<WindowId> = windows_of_class(CLASS)
        .into_iter()
        .filter(|w| hide::fully_visible(hide::visibility(*w)))
        .collect();
    // La Configuración de Windows es UWP en todas las ediciones (en LTSC
    // `calc.exe` abre la calculadora clásica, que es Win32).
    if Command::new("explorer.exe").arg("ms-settings:about").spawn().is_err() {
        return info(NAME, "omitida: no se pudo abrir ms-settings:".into());
    }
    let mut id = None;
    let appeared = wait(8000, || {
        id = windows_list::foreground()
            .filter(|w| class_name(*w) == CLASS && hide::fully_visible(hide::visibility(*w)));
        id.is_some()
    });
    let Some(id) = id.filter(|_| appeared) else {
        return info(NAME, "omitida: la Configuración no pasó al frente en 8 s".into());
    };
    let ours = !before.contains(&id);
    pump(500);
    let title = windows_list::window_title(id);
    let r = hider.hide(id);
    let hid = wait(500, || cloaked(id) == DWM_CLOAKED_SHELL);
    let marked = hide::hidden_with(id) == Some(HideMethod::ShellCloak);
    let _ = hider.show(id);
    let back = wait(500, || cloaked(id) == 0);
    pump(100);
    let visible = hide::fully_visible(hide::visibility(id));
    let closed = ours && {
        windows_list::close(id);
        wait(3000, || !alive(id) || !hide::fully_visible(hide::visibility(id)))
    };
    verdict(
        NAME,
        r.is_ok() && hid && marked && back && visible,
        format!(
            "«{title}»: se esconde: {hid}; marca: {marked}; vuelve y se ve: {}; {}",
            back && visible,
            if ours { format!("cerrada: {closed}") } else { "ya estaba abierta: se deja".into() }
        ),
    )
}

// ---------------------------------------------------------------------------
// Verificaciones con el usuario
// ---------------------------------------------------------------------------

/// Modo manual: barra de tareas, Alt+Tab y una app de administrador. Escribe
/// las instrucciones y lo que ve con `say`.
pub fn manual(exe: &Path, say: &mut dyn FnMut(String)) {
    let hider = match Hider::new(HideMethod::ShellCloak) {
        Ok(h) => h,
        Err(e) => return say(format!("Sin cloak: {e}")),
    };
    let _hooks = Hooks::install();
    let mut children = Children(Vec::new());
    let Some(pid) = children.spawn(exe, &["puppet", "2"]) else {
        return say("No se pudieron lanzar los títeres.".into());
    };
    if !wait(5000, || puppets_of(pid).len() == 2) {
        return say("Los títeres no aparecieron.".into());
    }
    let p = puppets_of(pid);
    let p2 = p[1];
    pump(300);
    let _ = hider.hide(p2);
    wait(500, || cloaked(p2) == DWM_CLOAKED_SHELL);

    say("1/3 · El «Títere 2» está escondido (cloak). En la barra de tareas, pasá el mouse por el ícono de los títeres y hacé clic en la miniatura del Títere 2 (si aparece). Tenés 30 s.".into());
    watch_cloaked(p2, 30, say);
    let _ = hider.hide(p2);
    wait(500, || cloaked(p2) == DWM_CLOAKED_SHELL);

    say("2/3 · Lo escondí de nuevo. Ahora probá Alt+Tab: ¿aparece el Títere 2 en la lista? Si aparece, elegilo. Tenés 30 s.".into());
    watch_cloaked(p2, 30, say);
    let _ = hider.show(p2);

    say("3/3 · Abrí el Administrador de tareas (Ctrl+Shift+Esc) y dejalo al frente. Tenés 30 s."
        .into());
    let mut target = None;
    wait(30_000, || {
        target = windows_list::foreground()
            .filter(|&w| crate::session::process_elevated(windows_list::window_pid(w)));
        target.is_some()
    });
    let Some(t) = target else {
        return say("No vi ninguna ventana de administrador al frente; se omite.".into());
    };
    pump(300);
    // SAFETY: propiedad de prueba sobre el HWND; se borra enseguida.
    let set = unsafe {
        SetPropW(
            t.hwnd(),
            w!("wm.test"),
            Some(HANDLE(std::ptr::without_provenance_mut::<c_void>(1))),
        )
    }
    .is_ok();
    // SAFETY: lectura.
    let read = unsafe { GetPropW(t.hwnd(), w!("wm.test")) }.0 as usize == 1;
    // SAFETY: borrar la propiedad de prueba (si existe).
    let _ = unsafe { RemovePropW(t.hwnd(), w!("wm.test")) };
    let r = hider.hide(t);
    let hid = wait(500, || cloaked(t) == DWM_CLOAKED_SHELL);
    pump(300);
    let _ = hider.show(t);
    let back = wait(500, || cloaked(t) == 0);
    // Si la marca no se pudo poner, `hide` no esconde: por las dudas, mostrar.
    if !back {
        let _ = hider.show(t);
    }
    say(format!(
        "Administrador de tareas: SetProp {}; leída: {read}; esconder: {}; volvió: {back}. Ya podés cerrarlo.",
        if set { "aceptado" } else { "rechazado" },
        match (r, hid) {
            (Ok(()), true) => "sí".to_string(),
            (Ok(()), false) => "pedido aceptado pero sin cloak".to_string(),
            (Err(e), _) => format!("no ({e})"),
        }
    ));
}

/// Durante `secs` segundos, cuenta qué le pasa a la ventana escondida `id`.
fn watch_cloaked(id: WindowId, secs: u64, say: &mut dyn FnMut(String)) {
    clear_events();
    let until = Instant::now() + Duration::from_secs(secs);
    let mut seen_front = false;
    let mut seen_uncloak = false;
    while Instant::now() < until {
        pump(50);
        let front = windows_list::foreground() == Some(id);
        let un = cloaked(id) == 0;
        if front && !seen_front {
            seen_front = true;
            say(format!("  → el Títere 2 pasó al frente ({}).", describe(id)));
        }
        if un && !seen_uncloak {
            seen_uncloak = true;
            say("  → el Títere 2 se destapó solo (UNCLOAKED).".into());
        }
        if seen_front && seen_uncloak {
            break;
        }
    }
    say(format!(
        "  Resumen: FOREGROUND del títere: {}; UNCLOAKED: {}; CLOAKED: {}; quedó {}.",
        count(EVENT_SYSTEM_FOREGROUND, Some(id)),
        count(EVENT_OBJECT_UNCLOAKED, Some(id)),
        count(EVENT_OBJECT_CLOAKED, Some(id)),
        describe(id)
    ));
}
