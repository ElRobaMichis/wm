//! `wm.exe`: el ejecutable de consola (CLI, CI y pruebas).

fn main() -> std::process::ExitCode {
    wm_app::run()
}
