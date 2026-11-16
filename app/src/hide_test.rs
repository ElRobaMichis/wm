//! Comandos del spike 2 (architecture.md §13.2): comparar las estrategias
//! para esconder y mostrar ventanas, y recuperar ventanas escondidas.
//!
//! - `wm puppet [n]`: abre `n` ventanas títere (por defecto 10).
//! - `wm hide-test <cloak|minimize|offscreen|hide> [rondas] [apps...]`:
//!   reparte las ventanas en dos "escritorios" y los alterna, midiendo.
//! - `wm restore`: muestra todas las ventanas que tengan nuestra marca.
//! - `wm hidden`: lista las ventanas que tienen nuestra marca.
//! - `wm hide-hold <método> [cycle]`: esconde los títeres impares y se queda
//!   esperando (o alternando sin parar) hasta que lo maten. Es el "core de
//!   mentira" para probar el guardián (spike 3).
//! - `wm crash-test [método] [n]`: lanza `hide-hold … cycle`, lo mata en un
//!   momento al azar, restaura y mide cuánto tarda en verse todo. `n` veces.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, ExitCode, Stdio};
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
    // Con un wm abierto, que lo haga él (así las ventanas pasan al escritorio
    // activo y no vuelven a esconderse en el próximo cambio).
    if wm_platform::tray::send_order(wm_platform::tray::Order::ShowAll) {
        println!("Pedido al wm abierto: mostrar todas las ventanas en el escritorio activo.");
        return ExitCode::SUCCESS;
    }
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

pub fn hidden() -> ExitCode {
    let marked = hide::marked_windows();
    if marked.is_empty() {
        println!("No hay ventanas escondidas por wm.");
        return ExitCode::SUCCESS;
    }
    println!("{} ventanas con la marca de wm:", marked.len());
    for (id, method) in &marked {
        let v = hide::visibility(*id);
        let state = if hide::fully_visible(v) { "se ve" } else { "escondida" };
        let title = windows_list::window_title(*id);
        println!("  {:#x}  «{}»  {state}  «{title}»", id.0, method.name());
    }
    // Código 1 si hay alguna: sirve para scripts de prueba.
    ExitCode::FAILURE
}

/// Esconde los títeres impares y espera a que lo maten.
pub fn hold(args: &[String]) -> ExitCode {
    let Some(method) = args.first().and_then(|m| HideMethod::from_name(m)) else {
        eprintln!("Uso: wm hide-hold <cloak|minimize|offscreen|hide> [cycle]");
        return ExitCode::from(2);
    };
    let cycle = args.get(1).is_some_and(|a| a == "cycle");
    let puppets = puppet::find_puppets();
    if puppets.len() < 2 {
        eprintln!("Hacen falta títeres: `wm puppet 10` en otra terminal.");
        return ExitCode::FAILURE;
    }
    let hider = match Hider::new(method) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("El método «{}» no está disponible: {e}", method.name());
            return ExitCode::FAILURE;
        }
    };
    let mut visible: Vec<WindowId> = puppets.iter().copied().step_by(2).collect();
    let mut hidden: Vec<WindowId> = puppets.iter().copied().skip(1).step_by(2).collect();
    for &id in &hidden {
        let _ = hider.hide(id);
    }
    println!("listo");
    let _ = std::io::stdout().flush();
    loop {
        if cycle {
            std::thread::sleep(Duration::from_millis(15));
            for &id in &hidden {
                let _ = hider.show(id);
            }
            for &id in &visible {
                let _ = hider.hide(id);
            }
            std::mem::swap(&mut visible, &mut hidden);
        } else {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
}

/// Mata `n` veces un `hide-hold … cycle` en un momento al azar y mide la
/// recuperación (criterio del spike 3: 100 % visibles en < 500 ms).
pub fn crash_test(args: &[String]) -> ExitCode {
    let method =
        args.first().and_then(|m| HideMethod::from_name(m)).unwrap_or(HideMethod::ShellCloak);
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100).max(1);
    let puppets = puppet::find_puppets();
    if puppets.len() < 2 {
        eprintln!("Hacen falta títeres: `wm puppet 10` en otra terminal.");
        return ExitCode::FAILURE;
    }
    let Ok(exe) = std::env::current_exe() else { return ExitCode::FAILURE };
    let _ = hide::restore_all();
    std::thread::sleep(Duration::from_millis(200));
    let position = |id: &WindowId| {
        let v = hide::visibility(*id);
        (v.left, v.top)
    };
    let before: Vec<(i32, i32)> = puppets.iter().map(position).collect();

    let mut rng = seed();
    let mut times: Vec<Duration> = Vec::new();
    let mut failures = 0usize;
    let mut worst_left = 0usize;
    println!("{n} muertes de `hide-hold {} cycle` con {} títeres...", method.name(), puppets.len());
    for i in 1..=n {
        let mut child = match Command::new(&exe)
            .args(["hide-hold", method.name(), "cycle"])
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("No se pudo lanzar el hijo: {e}");
                return ExitCode::FAILURE;
            }
        };
        let mut line = String::new();
        if let Some(out) = child.stdout.take() {
            let _ = BufReader::new(out).read_line(&mut line);
        }
        // Matar en cualquier punto del ciclo (cada cambio dura ~15 ms).
        let wait = next(&mut rng) % 200;
        std::thread::sleep(Duration::from_millis(wait));
        let _ = child.kill();
        let _ = child.wait();

        let started = Instant::now();
        let report = hide::restore_all();
        let deadline = started + Duration::from_secs(2);
        let mut left;
        loop {
            let not_seen =
                puppets.iter().filter(|id| !hide::fully_visible(hide::visibility(**id))).count();
            left = not_seen + hide::marked_windows().len();
            if left == 0 || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let elapsed = started.elapsed();
        if left > 0 || !report.failed.is_empty() {
            failures += 1;
            worst_left = worst_left.max(left);
            println!(
                "  muerte {i}: {left} ventanas sin recuperar tras 2 s; {} errores",
                report.failed.len()
            );
        }
        times.push(elapsed);
        if i % 100 == 0 {
            println!("  {i}/{n}");
        }
    }
    let alive = puppet::find_puppets().len();
    let moved = puppets.iter().zip(&before).filter(|(id, pos)| position(id) != **pos).count();
    times.sort();
    let pct = |p: f64| times[((times.len() as f64 - 1.0) * p).round() as usize];
    println!();
    println!(
        "Hasta verse todo: p50 {:.1} ms | p95 {:.1} ms | p99 {:.1} ms | máx {:.1} ms",
        ms(pct(0.5)),
        ms(pct(0.95)),
        ms(pct(0.99)),
        ms(*times.last().unwrap_or(&Duration::ZERO))
    );
    let over = times.iter().filter(|t| **t > Duration::from_millis(500)).count();
    println!("Muertes con recuperación > 500 ms: {over}");
    println!("Muertes con ventanas sin recuperar: {failures} (peor caso: {worst_left})");
    println!("Títeres vivos al final: {alive} de {}", puppets.len());
    println!("Títeres fuera de su posición original: {moved}");
    if failures == 0 && over == 0 && alive == puppets.len() && moved == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `wm max-test`: ¿una ventana maximizada sigue maximizada después de
/// esconderla y mostrarla? Con cada método, por el camino normal (`show`) y
/// por el de recuperación (`restore_all`).
pub fn max_test() -> ExitCode {
    let puppets = puppet::find_puppets();
    let Some(&id) = puppets.get(1) else {
        eprintln!("Hacen falta títeres: `wm puppet 10` en otra terminal.");
        return ExitCode::FAILURE;
    };
    let _ = hide::restore_all();
    let pause = || std::thread::sleep(Duration::from_millis(400));
    let mut failures = 0;
    for method in HideMethod::ALL {
        let Ok(hider) = Hider::new(method) else { continue };
        for path in ["show", "restore_all"] {
            hide::set_maximized_for_test(id, true);
            pause();
            let before = hide::is_maximized(id);
            let _ = hider.hide(id);
            pause();
            let hidden_ok = hide::looks_hidden(method, hide::visibility(id));
            if path == "show" {
                let _ = hider.show(id);
            } else {
                let _ = hide::restore_all();
            }
            pause();
            let after = hide::is_maximized(id);
            let seen = hide::fully_visible(hide::visibility(id));
            let ok = before && hidden_ok && after && seen;
            failures += usize::from(!ok);
            println!(
                "  {:<9} {:<11}: maximizada antes {before}, escondida {hidden_ok}, \
                 maximizada después {after}, se ve {seen}  {}",
                method.name(),
                path,
                if ok { "ok" } else { "FALLA" }
            );
        }
    }
    hide::set_maximized_for_test(id, false);
    pause();
    if failures == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn seed() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    t | 1
}

/// xorshift64: alcanza para elegir momentos al azar.
fn next(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
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
    if hider.reconnects() > 0 {
        println!("Reconexiones con el shell (¿se reinició el Explorer?): {}", hider.reconnects());
    }
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
