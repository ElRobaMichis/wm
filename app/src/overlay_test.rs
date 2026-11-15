//! `wm overlay-demo [square|shape|d2d|warp|bare] [loop|once] [now]` — la demo del
//! spike 5 (architecture.md §13.7). La mide `wm-bench overlay` desde otro
//! proceso, sin que `wm.exe` tenga que cargar D3D ni DXGI.
//!
//! Crea los overlays ocultos (con el mapa de teclado si se pide un camino de
//! texto) y escribe `READY` en stdout. Después obedece órdenes por stdin:
//! `show`, `hide`, `quit`. Con `now`, muestra apenas termina de crear.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

use wm_platform::input;
use wm_platform::overlay_demo::{Demo, Motion};
use wm_platform::overlay_text::TextPath;

/// Mensaje al hilo de la demo con una orden leída de stdin.
const WM_DEMO_CMD: u32 = 0x8000 + 0x40; // WM_APP + 0x40
const CMD_SHOW: usize = 1;
const CMD_HIDE: usize = 2;

pub fn demo(args: &[String]) -> ExitCode {
    let text = args.iter().find_map(|a| TextPath::from_name(a));
    let motion = if args.iter().any(|a| a == "once") {
        Motion::Once(Duration::from_secs(8))
    } else {
        Motion::Loop
    };
    let now = args.iter().any(|a| a == "now");

    input::ensure_message_queue();
    wm_platform::overlay::disable_ghosting();
    if args.iter().any(|a| a == "bare") {
        // Línea de base de memoria: el mismo ejecutable sin overlays.
        println!("READY");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        return ExitCode::SUCCESS;
    }
    let mut demo = match Demo::new(motion, text) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("No se pudieron crear los overlays: {e}");
            return ExitCode::FAILURE;
        }
    };
    if now && let Err(e) = demo.show() {
        eprintln!("No se pudo mostrar: {e}");
        return ExitCode::FAILURE;
    }
    println!("READY");
    let _ = std::io::stdout().flush();

    let ui = input::current_thread_id();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let cmd = match line.trim() {
                "show" => CMD_SHOW,
                "hide" => CMD_HIDE,
                "quit" => 0,
                _ => continue,
            };
            wm_platform::overlay::post_to_thread(ui, WM_DEMO_CMD, cmd);
            if cmd == 0 {
                break;
            }
        }
        wm_platform::overlay::post_to_thread(ui, WM_DEMO_CMD, 0);
    });

    while let Some(cmd) = wm_platform::overlay::next_thread_message(WM_DEMO_CMD) {
        let result = match cmd {
            CMD_SHOW => demo.show(),
            CMD_HIDE => demo.hide(),
            _ => break,
        };
        if let Err(e) = result {
            eprintln!("Error: {e}");
        }
        println!("DONE {cmd}");
        let _ = std::io::stdout().flush();
    }
    ExitCode::SUCCESS
}
