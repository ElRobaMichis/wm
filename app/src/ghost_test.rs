//! `wm ghost-demo <puppet N | app NOMBRE> <dx> <ms> [vueltas] [curtain]` —
//! spike 6 (architecture.md §7.4): anima una ventana ajena con un fantasma,
//! ida y vuelta `dx` píxeles en horizontal, `ms` por tramo.
//!
//! Secuencia por tramo (revisada con Fable):
//! 1. fantasma (miniatura en vivo, marco visible 1:1) encima de la real, en
//!    el rectángulo viejo; esperar 2 cuadros;
//! 2. cloak de la real, confirmado con `DWMWA_CLOAKED`; 1 cuadro más;
//! 3. un solo `SetWindowPos` (asíncrono) al destino, que además la sube al
//!    frente (el fantasma se ve encima de todo: la real tiene que terminar
//!    arriba);
//! 4. vuelo: un paso por cuadro del compositor, con la posición calculada
//!    para el cuadro siguiente (por tiempo);
//! 5. esperar a que la real esté en el destino (hasta 150 ms);
//! 6. uncloak, confirmado; 2 cuadros;
//! 7. quitar el fantasma.
//!
//! Con `curtain`, pone un telón magenta del tamaño del monitor justo debajo
//! de la ventana: así `wm-bench ghost` puede medir apps cuyo color no
//! controlamos (cuenta lo que no es magenta).
//!
//! Escribe `READY l t r b` (marco visible) y `ANIM t0 t1 desde hasta llegó`.

use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wm_model::Rect;
use wm_platform::clock;
use wm_platform::ghost::{self, GhostHost};
use wm_platform::hide::{HideMethod, Hider};
use wm_platform::windows_list::{self, WindowId};

fn target(args: &[String]) -> Option<WindowId> {
    match args.first().map(String::as_str) {
        Some("puppet") => {
            let n: usize = args.get(1)?.parse().ok()?;
            wm_platform::puppet::find_puppets().get(n.checked_sub(1)?).copied()
        }
        Some("app") => {
            let name = args.get(1)?.to_lowercase();
            windows_list::user_windows().into_iter().find(|w| w.app == name).map(|w| w.id)
        }
        _ => None,
    }
}

fn shift(r: Rect, dx: i32) -> Rect {
    Rect { left: r.left + dx, right: r.right + dx, ..r }
}

fn lerp(a: Rect, b: Rect, t: f64) -> Rect {
    let l = |x: i32, y: i32| x + ((y - x) as f64 * t).round() as i32;
    Rect {
        left: l(a.left, b.left),
        top: l(a.top, b.top),
        right: l(a.right, b.right),
        bottom: l(a.bottom, b.bottom),
    }
}

/// Espera hasta `frames` cuadros a que se cumpla `done`.
fn wait_until(frames: u32, mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..frames {
        if done() {
            return true;
        }
        ghost::wait_frame();
    }
    done()
}

pub fn demo(args: &[String]) -> ExitCode {
    let Some(id) = target(args) else {
        eprintln!("Uso: wm ghost-demo <puppet N | app NOMBRE> <dx> <ms> [vueltas] [curtain]");
        return ExitCode::from(2);
    };
    let dx: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(600);
    let ms: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let reps: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(10);
    let curtain_on = args.iter().any(|a| a == "curtain");

    wm_platform::input::ensure_message_queue();
    // Las apps de prueba se llevan a un tamaño y lugar conocidos, al frente.
    if args.first().is_some_and(|a| a == "app") {
        wm_platform::hide::set_maximized_for_test(id, false);
        std::thread::sleep(Duration::from_millis(300));
        let r = Rect { left: 100, top: 150, right: 1000, bottom: 800 };
        let _ = windows_list::place_async_top(id, r);
    } else if let Some(r) = ghost::window_rect(id) {
        let _ = windows_list::place_async_top(id, r);
    }
    std::thread::sleep(Duration::from_millis(600));
    let (Some(start_win), Some(start_efb)) = (ghost::window_rect(id), ghost::frame_bounds(id))
    else {
        return ExitCode::FAILURE;
    };
    let center = ((start_efb.left + start_efb.right) / 2, (start_efb.top + start_efb.bottom) / 2);
    let monitor = wm_platform::monitors().into_iter().find(|m| {
        let b = m.bounds;
        center.0 >= b.left && center.0 < b.right && center.1 >= b.top && center.1 < b.bottom
    });
    let Some(monitor) = monitor else { return ExitCode::FAILURE };
    let period = Duration::from_secs_f64(1.0 / f64::from(monitor.refresh_hz.unwrap_or(60)));

    let curtain = curtain_on.then(|| wm_platform::ghost_curtain::Curtain::new(monitor.bounds, id));
    let hider = match Hider::new(HideMethod::ShellCloak) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Sin cloak no hay fantasma: {e}");
            return ExitCode::FAILURE;
        }
    };
    let host = match GhostHost::new() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("No se pudo crear el anfitrión del fantasma: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut g = match host.ghost(id) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("No se pudo registrar la miniatura: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "marco visible {}×{}; DWM informa la fuente como {:?}; reloj del compositor: {}",
        start_efb.width(),
        start_efb.height(),
        g.source_size(),
        ghost::has_compositor_clock()
    );
    println!(
        "READY {} {} {} {} {}",
        start_efb.left, start_efb.top, start_efb.right, start_efb.bottom, id.0
    );
    let _ = std::io::stdout().flush();
    for _ in 0..40 {
        ghost::wait_frame(); // deja pintar el telón
    }

    let duration = Duration::from_millis(ms);
    let mut dir = 1;
    for _ in 0..reps {
        let (Some(win_from), Some(efb_from)) = (ghost::window_rect(id), ghost::frame_bounds(id))
        else {
            break;
        };
        let (win_to, efb_to) = (shift(win_from, dx * dir), shift(efb_from, dx * dir));

        // 1. Fantasma encima, en el marco viejo.
        let _ = g.place(efb_from);
        host.show();
        ghost::wait_frame();
        ghost::wait_frame();
        // 2. Cloak confirmado.
        let _ = hider.hide(id);
        wait_until(10, || ghost::is_cloaked(id));
        ghost::wait_frame();
        // 3. Un solo SetWindowPos, al frente.
        let _ = windows_list::place_async_top(id, win_to);
        // 4. Vuelo.
        ghost::boost_clock(true);
        let t0 = clock::now();
        let started = Instant::now();
        loop {
            let t = (started.elapsed() + period).as_secs_f64() / duration.as_secs_f64();
            if t >= 1.0 {
                break;
            }
            let _ = g.place(lerp(efb_from, efb_to, t));
            ghost::wait_frame();
        }
        let _ = g.place(efb_to);
        ghost::wait_frame();
        let t1 = clock::now();
        ghost::boost_clock(false);
        // 5. La real tiene que estar en el destino.
        let arrived = wait_until(22, || ghost::frame_bounds(id) == Some(efb_to));
        if !arrived && let Some(real) = ghost::frame_bounds(id) {
            let _ = g.place(real);
            ghost::wait_frame();
        }
        // 6. Uncloak confirmado.
        let _ = hider.show(id);
        wait_until(10, || !ghost::is_cloaked(id));
        ghost::wait_frame();
        ghost::wait_frame();
        // 7. Quitar el fantasma.
        g.conceal();
        host.hide();
        println!("ANIM {t0} {t1} {} {} {}", efb_from.left, efb_to.left, u8::from(arrived));
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_millis(300));
        dir = -dir;
    }
    let _ = windows_list::place_async_top(id, start_win);
    drop(curtain);
    println!("END");
    ExitCode::SUCCESS
}
