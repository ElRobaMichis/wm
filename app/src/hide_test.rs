//! Comandos del spike 2 (architecture.md §13.2): comparar las estrategias
//! para esconder y mostrar ventanas, y recuperar ventanas escondidas.
//!
//! - `wm puppet [n]`: abre `n` ventanas títere (por defecto 10).
//! - `wm hide-test <cloak|minimize|offscreen|hide> [rondas] [apps...]`:
//!   reparte las ventanas en dos "escritorios" y los alterna, midiendo.
//! - `wm restore`: muestra todas las ventanas que tengan nuestra marca.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use wm_platform::hide::{self, HideMethod, Hider};
use wm_platform::puppet;
use wm_platform::windows_list::{self, WindowId};

pub fn run_puppets(args: &[String]) -> ExitCode {
    let count = args.first().and_then(|s| s.parse().ok()).unwrap_or(10usize).clamp(2, 20);
    println!("Abriendo {count} ventanas títere. Cerrá cualquiera para cerrarlas todas.");
    match puppet::run_puppets(count) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("No se pudieron abrir los títeres: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn restore() -> ExitCode {
    let report = hide::restore_all();
    if report.restored.is_empty() && report.failed.is_empty() {
        println!("No había ventanas escondidas por wm.");
    }
    for (id, method) in &report.restored {
        println!("Restaurada {:#x} (estaba escondida con «{}»).", id.0, method.name());
    }
    for (id, error) in &report.failed {
        println!("NO se pudo restaurar {:#x}: {error}", id.0);
    }
    if report.failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

pub fn run(args: &[String]) -> ExitCode {
    let Some(method) = args.first().and_then(|m| HideMethod::from_name(m)) else {
        eprintln!("Uso: wm hide-test <cloak|minimize|offscreen|hide> [rondas] [apps...]");
        return ExitCode::from(2);
    };
    let rounds: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10).max(1);
    let apps: Vec<String> = args.iter().skip(2).map(|a| a.to_lowercase()).collect();

    // Restos de una prueba anterior que se cortó.
    let leftovers = hide::restore_all();
    if !leftovers.restored.is_empty() {
        println!("Restauradas {} ventanas de una prueba anterior.", leftovers.restored.len());
        std::thread::sleep(Duration::from_millis(300));
    }

    let targets: Vec<WindowId> = if apps.is_empty() {
        puppet::find_puppets()
    } else {
        windows_list::user_windows()
            .into_iter()
            .filter(|w| apps.contains(&w.app))
            .map(|w| w.id)
            .collect()
    };
    if targets.len() < 2 {
        eprintln!("Hacen falta al menos 2 ventanas.");
        eprintln!("Abrí títeres en otra terminal con `wm puppet 10`, o nombrá apps abiertas");
        eprintln!("(por ejemplo `wm hide-test cloak 10 notepad brave`).");
        return ExitCode::FAILURE;
    }

    let hider = match Hider::new(method) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("El método «{}» no está disponible: {e}", method.name());
            return ExitCode::FAILURE;
        }
    };

    // Escritorio A = posiciones pares, B = impares.
    let a: Vec<WindowId> = targets.iter().copied().step_by(2).collect();
    let b: Vec<WindowId> = targets.iter().copied().skip(1).step_by(2).collect();
    println!(
        "Método «{}»: {} ventanas, escritorio A = {} y B = {}. {rounds} cambios.",
        method.name(),
        targets.len(),
        a.len(),
        b.len()
    );
    println!("Mirá la pantalla: no debería verse el fondo de pantalla ni parpadeos.");
    println!("(Ctrl+C a mitad de la prueba deja ventanas escondidas: `wm restore` las recupera.)");
    println!();

    let mut errors: Vec<String> = Vec::new();
    // Estado inicial: solo A visible.
    for &id in &b {
        if let Err(e) = hider.hide(id) {
            errors.push(format!("esconder {:#x}: {e}", id.0));
        }
    }
    std::thread::sleep(Duration::from_millis(800));

    let mut timings: Vec<Duration> = Vec::new();
    let mut wrong_states = 0usize;
    let (mut visible, mut hidden) = (a.clone(), b.clone());
    for round in 1..=rounds {
        let started = Instant::now();
        // Orden de architecture.md §6.3: primero mostrar el destino, después
        // esconder el origen (así no se ve el fondo entre medio).
        for &id in &hidden {
            if let Err(e) = hider.show(id) {
                errors.push(format!("mostrar {:#x}: {e}", id.0));
            }
        }
        for &id in &visible {
            if let Err(e) = hider.hide(id) {
                errors.push(format!("esconder {:#x}: {e}", id.0));
            }
        }
        let elapsed = started.elapsed();
        timings.push(elapsed);
        std::mem::swap(&mut visible, &mut hidden);

        // Verificar el estado un momento después (las llamadas son asíncronas).
        std::thread::sleep(Duration::from_millis(150));
        let is_hidden = |id: &WindowId| hide::looks_hidden(method, hide::visibility(*id));
        let bad_hidden = hidden.iter().filter(|id| !is_hidden(id)).count();
        let bad_visible = visible.iter().filter(|id| is_hidden(id)).count();
        let bad = bad_hidden + bad_visible;
        wrong_states += bad;
        let status =
            if bad == 0 { "ok".to_owned() } else { format!("{bad} ventanas en mal estado") };
        println!("  cambio {round:>2}: {:>6.2} ms  {status}", ms(elapsed));
        std::thread::sleep(Duration::from_millis(600));
    }

    // Dejar todo visible.
    for &id in &hidden {
        let _ = hider.show(id);
    }

    timings.sort();
    let pct = |p: f64| timings[((timings.len() as f64 - 1.0) * p).round() as usize];
    println!();
    println!(
        "Tiempo de cada cambio (emitir las llamadas): p50 {:.2} ms | p95 {:.2} ms | máx {:.2} ms",
        ms(pct(0.5)),
        ms(pct(0.95)),
        ms(*timings.last().unwrap_or(&Duration::ZERO))
    );
    println!("  (meta del spike: p95 < 30 ms)");
    println!("Ventanas en mal estado tras los cambios: {wrong_states}");
    if !errors.is_empty() {
        println!("Errores ({}):", errors.len());
        for e in errors.iter().take(10) {
            println!("  {e}");
        }
    }
    ExitCode::SUCCESS
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
