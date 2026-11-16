//! `wm ops-test [segundos] [focus] [burn]` — prueba de carga del spike 4
//! (architecture.md §13.2): un brain real con los carriles de [`Ops`], un
//! hilo que le hace ping cada 1 ms y teclas sintéticas cada 40 ms.
//!
//! Fases (cada una dura `segundos`, por defecto 12):
//! 1. **normal**: todos los títeres responden;
//! 2. **colgado**: el títere colgable deja de responder (a los ~5 s aparece
//!    la ventana fantasma) y se le siguen mandando pedidos;
//! 3. **carriles trabados**: al empezar, una llamada de 400 ms en `win` y otra
//!    de 600 ms en `shell` (más que su plazo): se abandonan esos hilos.
//!
//! Teclas: `H` cambia de "escritorio" (muestra un grupo, esconde el otro, con
//! cloak), `M` mueve todos los títeres 1 px, `S` enumera ventanas y, con
//! `focus`, `F` trae al frente el siguiente títere (roba el foco: no usar la
//! PC mientras tanto). `burn` ocupa todos los núcleos durante la prueba.

use std::collections::HashMap;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use wm_model::Rect;
use wm_model::keyboard::HyperAction;
use wm_platform::clock;
use wm_platform::hide::{self, HideMethod};
use wm_platform::input::{self, InputEvent, Wake};
use wm_platform::ops::{Lane, Op, Ops, OpsEvent, Ticket};
use wm_platform::puppet;
use wm_platform::windows_list::WindowId;

const PHASES: [&str; 3] = ["normal", "colgado", "carriles trabados"];

/// Muestras en microsegundos.
#[derive(Default)]
struct Hist(Vec<u64>);

impl Hist {
    fn push(&mut self, us: u64) {
        self.0.push(us);
    }

    fn summary(&self) -> String {
        if self.0.is_empty() {
            return "sin muestras".to_owned();
        }
        let mut v = self.0.clone();
        v.sort_unstable();
        let pct = |p: f64| v[((v.len() as f64 - 1.0) * p).round() as usize] as f64 / 1000.0;
        format!(
            "n={:<5} p50 {:>6.2} ms | p99 {:>7.2} ms | máx {:>7.2} ms",
            v.len(),
            pct(0.5),
            pct(0.99),
            *v.last().unwrap_or(&0) as f64 / 1000.0
        )
    }

    fn over(&self, us: u64) -> usize {
        self.0.iter().filter(|x| **x > us).count()
    }
}

#[derive(Default)]
struct PhaseStats {
    ping: Hist,
    iteration: Hist,
    key_to_submit: Hist,
    /// Pedido → resultado, por carril, sobre las ventanas sanas.
    others: HashMap<&'static str, Hist>,
    /// Lo mismo, sobre la ventana colgada.
    hung: HashMap<&'static str, Hist>,
    errors: Vec<String>,
}

struct Pending {
    submitted: i64,
    phase: usize,
    on_hung: bool,
    test_stall: bool,
}

struct Brain {
    ops: Ops,
    commons: Vec<WindowId>,
    hung: WindowId,
    rects: HashMap<WindowId, Rect>,
    /// Grupo A visible (true) o B visible (false).
    a_visible: bool,
    nudge: i32,
    focus_next: usize,
    pending: HashMap<Ticket, Pending>,
    phase: usize,
    stats: Vec<PhaseStats>,
}

impl Brain {
    fn submit(&mut self, op: Op) {
        let on_hung = match &op {
            Op::Focus(id) | Op::Unhide(id) => *id == self.hung,
            Op::Place { id, .. } | Op::Hide { id, .. } => *id == self.hung,
            _ => false,
        };
        let test_stall = matches!(op, Op::Stall { .. });
        let submitted = clock::now();
        let ticket = self.ops.submit(op);
        self.pending.insert(ticket, Pending { submitted, phase: self.phase, on_hung, test_stall });
    }

    /// Grupo A: títeres comunes en posición par. Grupo B: impares + el colgable.
    fn groups(&self) -> (Vec<WindowId>, Vec<WindowId>) {
        let a = self.commons.iter().copied().step_by(2).collect();
        let mut b: Vec<WindowId> = self.commons.iter().copied().skip(1).step_by(2).collect();
        b.push(self.hung);
        (a, b)
    }

    fn handle_key(&mut self, letter: u8, focus: bool) {
        match letter {
            b'H' => {
                let (a, b) = self.groups();
                let (show, hide_now) = if self.a_visible { (b, a) } else { (a, b) };
                // Mostrar primero, esconder después (architecture.md §6.3).
                for id in show {
                    self.submit(Op::Unhide(id));
                }
                for id in hide_now {
                    self.submit(Op::Hide { id, method: HideMethod::ShellCloak });
                }
                self.a_visible = !self.a_visible;
            }
            b'M' => {
                self.nudge = 1 - self.nudge;
                let moves: Vec<(WindowId, Rect)> = self
                    .rects
                    .iter()
                    .map(|(id, r)| {
                        let d = self.nudge;
                        let (left, right) = (r.left + d, r.right + d);
                        (*id, Rect { left, top: r.top, right, bottom: r.bottom })
                    })
                    .collect();
                for (id, rect) in moves {
                    self.submit(Op::Place { id, rect });
                }
            }
            b'S' => self.submit(Op::Scan),
            b'F' if focus => {
                let mut all = self.commons.clone();
                all.push(self.hung);
                let id = all[self.focus_next % all.len()];
                self.focus_next += 1;
                self.submit(Op::Focus(id));
            }
            _ => {}
        }
    }

    fn record(&mut self, event: OpsEvent) {
        match event {
            OpsEvent::Done { ticket, lane, result, .. } => {
                let Some(p) = self.pending.remove(&ticket) else { return };
                let us = clock::micros_since(p.submitted);
                let stats = &mut self.stats[p.phase];
                if p.test_stall {
                    return; // los atascos provocados no cuentan como operaciones
                }
                if let Err(e) = result {
                    let which = if p.on_hung { "colgada" } else { "sana" };
                    stats.errors.push(format!("{} ({which}): {e}", lane.name()));
                }
                let map = if p.on_hung { &mut stats.hung } else { &mut stats.others };
                map.entry(lane.name()).or_default().push(us);
            }
            OpsEvent::Late { .. } => {}
        }
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let secs: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(12).clamp(6, 120);
    let focus = args.iter().any(|a| a == "focus");
    let burn = args.iter().any(|a| a == "burn");

    let Some(hung) = puppet::find_hang_puppet() else {
        eprintln!("Falta el títere colgable: `wm hang-puppet` en otra terminal.");
        return ExitCode::FAILURE;
    };
    let commons = puppet::find_puppets();
    if commons.len() < 2 {
        eprintln!("Faltan títeres comunes: `wm puppet 9` en otra terminal.");
        return ExitCode::FAILURE;
    }
    let _ = hide::restore_all();
    std::thread::sleep(Duration::from_millis(200));
    let mut rects = HashMap::new();
    for &id in commons.iter().chain(std::iter::once(&hung)) {
        let v = hide::visibility(id);
        rects.insert(id, Rect { left: v.left, top: v.top, right: v.right, bottom: v.bottom });
    }

    input::ensure_message_queue();
    wm_platform::ops::boost_process();
    wm_platform::ops::boost_brain_thread();
    let brain_tid = input::current_thread_id();
    let ops = match Ops::start(brain_tid) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("No se pudieron arrancar los carriles: {e}");
            return ExitCode::FAILURE;
        }
    };

    let total = Duration::from_secs(secs * PHASES.len() as u64);
    let mut burner = None;
    if burn {
        if let Ok(exe) = std::env::current_exe() {
            burner = Command::new(exe)
                .args(["burn", &(total.as_secs() + 2).to_string()])
                .stdout(std::process::Stdio::null())
                .spawn()
                .ok();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    println!(
        "ops-test: {} títeres + 1 colgable, {} s por fase{}{}.",
        commons.len(),
        secs,
        if focus { ", con foco" } else { "" },
        if burn { ", CPU al 100 %" } else { "" }
    );

    let stop = Arc::new(AtomicBool::new(false));
    let pinger = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
                input::post_ping(brain_tid);
            }
        })
    };
    let keys = {
        let stop = Arc::clone(&stop);
        let letters: &'static [u8] = if focus { b"HMSF" } else { b"HMS" };
        std::thread::spawn(move || {
            let mut i = 0usize;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(40));
                let letter = letters[i % letters.len()];
                input::simulate_action(brain_tid, HyperAction::Letter { letter, thumb: false });
                i += 1;
            }
        })
    };

    let mut brain = Brain {
        ops,
        commons,
        hung,
        rects,
        a_visible: true,
        nudge: 0,
        focus_next: 0,
        pending: HashMap::new(),
        phase: 0,
        stats: (0..PHASES.len()).map(|_| PhaseStats::default()).collect(),
    };
    // Estado inicial: grupo B escondido.
    let (_, b) = brain.groups();
    for id in b {
        brain.submit(Op::Hide { id, method: HideMethod::ShellCloak });
    }

    let start = Instant::now();
    let phase_len = Duration::from_secs(secs);
    loop {
        let phase_end = start + phase_len * (brain.phase as u32 + 1);
        let deadline = brain.ops.next_deadline().map_or(phase_end, |d| d.min(phase_end));
        let wake = input::wait(Some(deadline));
        let t0 = clock::now();
        match wake {
            Wake::Ping(sent) => brain.stats[brain.phase].ping.push(clock::micros(sent, t0)),
            Wake::Input(InputEvent::Action {
                action: HyperAction::Letter { letter, .. },
                qpc,
                ..
            }) => {
                brain.handle_key(letter, focus);
                let us = clock::micros_since(qpc);
                brain.stats[brain.phase].key_to_submit.push(us);
            }
            Wake::Quit => break,
            _ => {}
        }
        let now = Instant::now();
        for event in brain.ops.expire(now) {
            brain.record(event);
        }
        for event in brain.ops.drain() {
            brain.record(event);
        }
        if !matches!(wake, Wake::Timeout) {
            let us = clock::micros_since(t0);
            brain.stats[brain.phase].iteration.push(us);
        }

        if now >= phase_end {
            brain.phase += 1;
            if brain.phase == PHASES.len() {
                break;
            }
            match brain.phase {
                1 => {
                    let ms = (secs * 1000 + 3000) as u32;
                    let _ = puppet::hang(hung, ms, false);
                }
                2 => {
                    brain.submit(Op::Stall { lane: Lane::Win, ms: 400 });
                    brain.submit(Op::Stall { lane: Lane::Shell, ms: 600 });
                }
                _ => {}
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    let _ = pinger.join();
    let _ = keys.join();
    if let Some(mut b) = burner {
        let _ = b.kill();
        let _ = b.wait();
    }
    let ops_stats = brain.ops.stats();

    for (i, name) in PHASES.iter().enumerate() {
        let s = &brain.stats[i];
        println!();
        println!("== Fase {}: {name}", i + 1);
        println!("  ping al brain       {}", s.ping.summary());
        println!(
            "  vuelta del brain    {}  (> 5 ms: {})",
            s.iteration.summary(),
            s.iteration.over(5000)
        );
        println!("  tecla → pedido      {}", s.key_to_submit.summary());
        for lane in Lane::ALL {
            if let Some(h) = s.others.get(lane.name()) {
                println!("  {:<6} sanas       {}", lane.name(), h.summary());
            }
        }
        for lane in Lane::ALL {
            if let Some(h) = s.hung.get(lane.name()) {
                println!("  {:<6} colgada     {}", lane.name(), h.summary());
            }
        }
        if !s.errors.is_empty() {
            println!("  errores: {}", s.errors.len());
            let mut seen: Vec<&String> = Vec::new();
            for e in &s.errors {
                if !seen.contains(&e) {
                    seen.push(e);
                }
            }
            for e in seen.iter().take(5) {
                println!("    {e}");
            }
        }
    }
    println!();
    println!(
        "Hilos abandonados: {} | volvieron tarde: {} | cloak hecho con minimize: {}",
        ops_stats.abandoned, ops_stats.late, ops_stats.degraded_hides
    );

    // Dejar todo como estaba (esperando a que el colgable se descuelgue).
    std::thread::sleep(Duration::from_secs(4));
    let report = hide::restore_all();
    std::thread::sleep(Duration::from_millis(300));
    for (id, r) in &brain.rects {
        let _ = wm_platform::windows_list::place_async(*id, *r);
    }
    let left = hide::marked_windows().len();
    println!("Restauradas al final: {} | marcadas que quedaron: {left}", report.restored.len());
    ExitCode::SUCCESS
}
