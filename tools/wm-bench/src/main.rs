//! `wm-bench` — mediciones de los spikes, en un binario aparte para que
//! `wm.exe` no cargue D3D ni DXGI (architecture.md §13.7).
//!
//! `wm-bench overlay [arranques]` mide la demo de overlays (`wm overlay-demo`)
//! desde afuera, con Desktop Duplication en cada monitor:
//! 1. **Frío desde cero** (`arranques` veces, 20 por defecto): desde antes de
//!    lanzar el proceso hasta el primer cuadro con el cuadrado.
//! 2. **Precreado**: overlays y Compositor ya creados y ocultos (como los
//!    tendría el core); desde la orden `show` hasta el primer cuadro. Después,
//!    5 s de animación: % de refrescos con posición nueva, regularidad del
//!    paso, CPU de la demo. Esconder y volver a mostrar.
//! 3. **Suspendida**: animación de 8 s; a los 2 s se suspende la demo 10 s
//!    (más que los 5 s en que Windows da una app por colgada). La animación
//!    tiene que seguir y llegar al final.
//! 4. **Mapa con texto** por tres caminos (`shape`, `d2d`, `warp`): primer
//!    cuadro (incluye armar el mapa) y memoria.
//!
//! Memoria: working set privado y commit de la demo y de `dwm.exe`, leídos
//! con `NtQuerySystemInformation`.

#[cfg(windows)]
mod ghost;
#[cfg(windows)]
mod meter;
#[cfg(windows)]
mod sysinfo;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        #[cfg(windows)]
        Some("overlay") => overlay::run(&args[1..]),
        #[cfg(windows)]
        Some("snapshot") => overlay::snapshot(),
        #[cfg(windows)]
        Some("capture") => match meter::capture_primary() {
            Ok((img, w, h)) => {
                let file = args.get(1).map_or("captura.bmp", String::as_str);
                match meter::save_bmp(std::path::Path::new(file), &img, w, (0, 0, w, h)) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("{e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        #[cfg(windows)]
        Some("ghost") => match overlay::wm_exe() {
            Some(exe) => ghost::run(&args[1..], &exe),
            None => {
                eprintln!("No encuentro wm.exe junto a wm-bench.exe.");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("Uso: wm-bench overlay [arranques] | snapshot | ghost [dx] [ms] [vueltas]");
            ExitCode::from(2)
        }
    }
}

#[cfg(windows)]
mod overlay {
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::io::AsRawHandle;
    use std::path::PathBuf;
    use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};

    use wm_platform::clock;
    use wm_platform::overlay_demo::MARK_ROW;
    use wm_platform::probe;

    use crate::meter::{self, Measured, Stats};
    use crate::sysinfo::{self, Memory};

    /// Un `wm overlay-demo` lanzado, con su stdin y su stdout.
    struct Demo {
        child: Child,
        stdin: ChildStdin,
        lines: Receiver<String>,
    }

    impl Demo {
        fn spawn(exe: &PathBuf, args: &[&str]) -> Result<Self, String> {
            let mut child = Command::new(exe)
                .arg("overlay-demo")
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            let stdin = child.stdin.take().ok_or("sin stdin")?;
            let out = child.stdout.take().ok_or("sin stdout")?;
            let (tx, lines) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.send(line);
                }
            });
            Ok(Self { child, stdin, lines })
        }

        fn wait(&self, prefix: &str, timeout: Duration) -> bool {
            let until = Instant::now() + timeout;
            loop {
                let left = until.saturating_duration_since(Instant::now());
                match self.lines.recv_timeout(left) {
                    Ok(l) if l.starts_with(prefix) => return true,
                    Ok(_) => {}
                    Err(_) => return false,
                }
            }
        }

        fn send(&mut self, cmd: &str) {
            let _ = writeln!(self.stdin, "{cmd}");
            let _ = self.stdin.flush();
        }

        fn pid(&self) -> u32 {
            self.child.id()
        }

        fn handle(&self) -> isize {
            self.child.as_raw_handle() as isize
        }

        fn quit(mut self) {
            self.send("quit");
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                if let Ok(Some(_)) = self.child.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn mb(b: u64) -> f64 {
        b as f64 / (1024.0 * 1024.0)
    }

    fn mem(m: Option<Memory>) -> String {
        m.map_or("?".to_owned(), |m| {
            format!("{:.1} MB WS privado / {:.1} MB commit", mb(m.private_ws), mb(m.commit))
        })
    }

    fn percentiles(mut v: Vec<f64>) -> String {
        if v.is_empty() {
            return "sin datos".to_owned();
        }
        v.sort_by(f64::total_cmp);
        let p = |q: f64| v[((v.len() as f64 - 1.0) * q).round() as usize];
        let n = v.len();
        format!("p50 {:.1} ms | p95 {:.1} ms | máx {:.1} ms (n={n})", p(0.5), p(0.95), p(1.0))
    }

    fn stats_line(s: &Stats) -> String {
        format!(
            "{:.1} % de refrescos con posición nueva ({} de {:.0}); DWM compuso {:.1} %; \
             paso {:.1} ± {:.2} px [{}..{}]; salteados {}{}",
            s.moved_pct(),
            s.moved,
            s.refreshes,
            s.frames_pct(),
            s.step_mean,
            s.step_sd,
            s.step_min,
            s.step_max,
            s.skipped,
            if s.trustworthy() { "" } else { " (MEDICIÓN NO CONFIABLE: > 1 % salteados)" }
        )
    }

    struct Marks {
        cold: Vec<i64>,
        pre_show: i64,
        steady: (i64, i64),
        reshow: i64,
        sus_show: i64,
        sus: (i64, i64),
        text: Vec<(String, Vec<i64>)>,
        end: i64,
    }

    pub fn wm_exe() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("wm.exe")))
            .filter(|p| p.exists())
    }

    fn notification_state() -> String {
        use windows::Win32::UI::Shell::SHQueryUserNotificationState;
        // SAFETY: consulta sin parámetros.
        match unsafe { SHQueryUserNotificationState() } {
            Ok(s) => match s.0 {
                1 => "no presente".into(),
                2 => "OCUPADO (app a pantalla completa)".into(),
                3 => "D3D A PANTALLA COMPLETA".into(),
                4 => "modo presentación".into(),
                5 => "acepta notificaciones (normal)".into(),
                6 => "hora silenciosa".into(),
                7 => "app de la tienda a pantalla completa".into(),
                n => format!("estado {n}"),
            },
            Err(e) => format!("error {e}"),
        }
    }

    /// Captura el mapa dibujado por cada camino de texto en un BMP, y comprueba
    /// que un overlay del tamaño del monitor no haga creer al shell que hay
    /// una app a pantalla completa.
    pub fn snapshot() -> ExitCode {
        let Some(exe) = wm_exe() else {
            eprintln!("No encuentro wm.exe junto a wm-bench.exe.");
            return ExitCode::FAILURE;
        };
        let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
        println!("Estado del shell sin overlays: {}", notification_state());
        for path in ["shape", "warp", "d2d"] {
            let Ok(mut d) = Demo::spawn(&exe, &[path, "loop"]) else { continue };
            d.wait("READY", Duration::from_secs(5));
            d.send("show");
            d.wait("DONE", Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(800));
            let state = notification_state();
            match meter::capture_primary() {
                Ok((img, w, h)) => {
                    let (cw, ch) = (1160u32.min(w), 400u32.min(h));
                    let rect = ((w - cw) / 2, (h - ch) / 2, cw, ch);
                    let file = dir.join(format!("overlay-{path}.bmp"));
                    match meter::save_bmp(&file, &img, w, rect) {
                        Ok(()) => println!("{path}: {} (shell: {state})", file.display()),
                        Err(e) => println!("{path}: no se pudo guardar: {e}"),
                    }
                }
                Err(e) => println!("{path}: no se pudo capturar: {e}"),
            }
            d.quit();
            std::thread::sleep(Duration::from_millis(300));
        }
        ExitCode::SUCCESS
    }

    pub fn run(args: &[String]) -> ExitCode {
        let runs: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(20).max(1);
        let Some(exe) = wm_exe() else {
            eprintln!("No encuentro wm.exe junto a wm-bench.exe.");
            return ExitCode::FAILURE;
        };

        let stop = Arc::new(AtomicBool::new(false));
        let meters = match meter::start(MARK_ROW, meter::ColorKey::MAGENTA, Arc::clone(&stop)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("No se pudo arrancar Desktop Duplication: {e}");
                return ExitCode::FAILURE;
            }
        };
        for m in &meters {
            println!("Monitor {} {:?} a {:.2} Hz", m.name, m.desktop, m.refresh_hz);
        }
        let dwm_idle = sysinfo::of_name("dwm.exe");
        std::thread::sleep(Duration::from_millis(300));
        let gap = Duration::from_millis(400);

        let mut marks = Marks {
            cold: Vec::new(),
            pre_show: 0,
            steady: (0, 0),
            reshow: 0,
            sus_show: 0,
            sus: (0, 0),
            text: Vec::new(),
            end: 0,
        };
        let mut report: Vec<String> = Vec::new();

        // Línea de base: el mismo ejecutable sin overlays.
        if let Ok(d) = Demo::spawn(&exe, &["bare"]) {
            d.wait("READY", Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(500));
            report.push(format!("wm.exe sin overlays: {}", mem(sysinfo::of_pid(d.pid()))));
            d.quit();
        }

        // 1. Frío desde cero.
        print!("Arranques en frío");
        for _ in 0..runs {
            let t0 = clock::now();
            let Ok(d) = Demo::spawn(&exe, &["square", "loop", "now"]) else { continue };
            marks.cold.push(t0);
            d.wait("READY", Duration::from_secs(5));
            std::thread::sleep(gap);
            d.quit();
            std::thread::sleep(gap);
            print!(".");
            let _ = std::io::stdout().flush();
        }
        println!();

        // 2. Precreado.
        let cpu;
        let (pre_mem, show_mem, dwm_showing);
        {
            let Ok(mut d) = Demo::spawn(&exe, &["square", "loop"]) else {
                return ExitCode::FAILURE;
            };
            d.wait("READY", Duration::from_secs(5));
            std::thread::sleep(Duration::from_secs(1));
            pre_mem = sysinfo::of_pid(d.pid());
            marks.pre_show = clock::now();
            d.send("show");
            std::thread::sleep(Duration::from_millis(500));
            let (c0, q0) = (probe::process_cpu_ms(d.handle()), clock::now());
            std::thread::sleep(Duration::from_secs(5));
            let (c1, q1) = (probe::process_cpu_ms(d.handle()), clock::now());
            marks.steady = (q0, q1);
            show_mem = sysinfo::of_pid(d.pid());
            dwm_showing = sysinfo::of_name("dwm.exe");
            cpu = (c1 - c0, clock::micros(q0, q1) as f64 / 1000.0);
            d.send("hide");
            std::thread::sleep(Duration::from_millis(700));
            marks.reshow = clock::now();
            d.send("show");
            std::thread::sleep(Duration::from_secs(1));
            d.quit();
        }
        std::thread::sleep(gap);

        // 3. Suspendida.
        let (suspended, ghosts);
        {
            let Ok(mut d) = Demo::spawn(&exe, &["square", "once"]) else {
                return ExitCode::FAILURE;
            };
            d.wait("READY", Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(500));
            marks.sus_show = clock::now();
            d.send("show");
            std::thread::sleep(Duration::from_secs(2));
            let s0 = clock::now();
            suspended = probe::suspend_process(d.handle(), true);
            std::thread::sleep(Duration::from_millis(7000)); // 9 s desde el show
            ghosts = probe::ghost_windows().len();
            std::thread::sleep(Duration::from_millis(3000)); // 12 s desde el show
            let s1 = clock::now();
            let _ = probe::suspend_process(d.handle(), false);
            marks.sus = (s0, s1);
            std::thread::sleep(Duration::from_millis(500));
            d.quit();
        }
        std::thread::sleep(gap);

        // 4. Mapa con texto.
        // (camino, demo antes de mostrar, demo mostrando, dwm.exe mostrando)
        type TextMem = (String, Option<Memory>, Option<Memory>, Option<Memory>);
        let mut text_mem: Vec<TextMem> = Vec::new();
        for path in ["shape", "d2d", "warp"] {
            let mut shows = Vec::new();
            let mut last = (None, None, None);
            for _ in 0..5 {
                let Ok(mut d) = Demo::spawn(&exe, &[path, "loop"]) else { continue };
                d.wait("READY", Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(500));
                let before = sysinfo::of_pid(d.pid());
                shows.push(clock::now());
                d.send("show");
                d.wait("DONE", Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(1500));
                last = (before, sysinfo::of_pid(d.pid()), sysinfo::of_name("dwm.exe"));
                d.quit();
                std::thread::sleep(gap);
            }
            text_mem.push((path.to_owned(), last.0, last.1, last.2));
            marks.text.push((path.to_owned(), shows));
        }
        marks.end = clock::now();
        stop.store(true, Ordering::Relaxed);

        // ------------------------------------------------------------------
        println!();
        for line in &report {
            println!("{line}");
        }
        println!("dwm.exe en reposo: {}", mem(dwm_idle));
        println!();
        println!("== Precreado (overlays y Compositor ya creados, ocultos)");
        println!("  demo antes de mostrar: {}", mem(pre_mem));
        println!("  demo mostrando:        {}", mem(show_mem));
        println!("  dwm.exe mostrando:     {}", mem(dwm_showing));
        println!(
            "  CPU de la demo animando: {:.2} ms en {:.0} ms ({:.3} %)",
            cpu.0,
            cpu.1,
            100.0 * cpu.0 / cpu.1
        );
        println!();
        println!("== Suspendida 10 s a mitad de una animación de 8 s");
        println!("  NtSuspendProcess: {suspended}; ventanas fantasma a los 9 s: {ghosts}");
        println!();
        println!("== Mapa con texto: memoria (última de 5 corridas)");
        for (path, before, after, dwm) in &text_mem {
            println!("  {path:<6} demo antes {} → después {}", mem(*before), mem(*after));
            println!("         dwm.exe {}", mem(*dwm));
        }

        for m in meters {
            let (name, hz, desktop) = (m.name.clone(), m.refresh_hz, m.desktop);
            let Measured { samples, lost } = match m.finish() {
                Ok(s) => s,
                Err(e) => {
                    println!("{name}: error del medidor: {e}");
                    continue;
                }
            };
            println!();
            println!(
                "== {name} ({hz:.2} Hz): {} cuadros, duplicación recreada {lost} veces",
                samples.len()
            );
            let ends = |i: usize, v: &Vec<i64>| v.get(i + 1).copied().unwrap_or(marks.pre_show);
            let cold: Vec<f64> = marks
                .cold
                .iter()
                .enumerate()
                .filter_map(|(i, t)| meter::first_visible(&samples, *t, ends(i, &marks.cold)))
                .collect();
            let missing = marks.cold.len() - cold.len();
            println!("  frío desde cero:        {}", percentiles(cold));
            if missing > 0 {
                println!("    ({missing} arranques sin cuadrado visible)");
            }
            let pre = meter::first_visible(&samples, marks.pre_show, marks.steady.0);
            let re = meter::first_visible(&samples, marks.reshow, marks.sus_show);
            println!(
                "  precreado, primer show: {}; volver a mostrar: {}",
                pre.map_or("no se vio".into(), |v| format!("{v:.1} ms")),
                re.map_or("no se vio".into(), |v| format!("{v:.1} ms"))
            );
            let steady = meter::analyze(&samples, marks.steady.0, marks.steady.1, hz);
            println!("  animando: {}", stats_line(&steady));

            let (s0, s1) = marks.sus;
            // Desde 0,1 s después de suspender hasta el final de la animación (8 s).
            let anim_end = clock::plus_micros(marks.sus_show, 8_000_000);
            let during = meter::analyze(&samples, clock::plus_micros(s0, 100_000), anim_end, hz);
            println!("  suspendida (moviéndose): {}", stats_line(&during));
            let final_x = samples
                .iter()
                .filter(|s| s.qpc >= s0 && s.qpc < s1)
                .filter_map(|s| s.x)
                .next_back();
            let end_x = desktop.width() - 40 - 1;
            match final_x {
                Some(x) => println!(
                    "  suspendida: última posición x={x} (final de la animación ≈ {end_x}): {}",
                    if (i64::from(x) - i64::from(end_x)).abs() <= 3 {
                        "LLEGÓ al final"
                    } else {
                        "NO llegó"
                    }
                ),
                None => println!("  suspendida: no se vio el cuadrado"),
            }

            for (path, shows) in &marks.text {
                let firsts: Vec<f64> = shows
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| {
                        let until = shows.get(i + 1).copied().unwrap_or(marks.end);
                        meter::first_visible(&samples, *t, until)
                    })
                    .collect();
                let first = percentiles(firsts);
                println!("  mapa {path:<6}: primer cuadro (arma el mapa al mostrar) {first}");
            }
        }
        ExitCode::SUCCESS
    }
}
