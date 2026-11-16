//! `wmw.exe`: el mismo programa sin ventana de consola (plan-core.md D5). Es
//! el que se lanza al iniciar sesión y desde el acceso directo: no abre una
//! terminal. Los mensajes van a `%LOCALAPPDATA%\wm\wm.log`.

#![windows_subsystem = "windows"]

fn main() -> std::process::ExitCode {
    wm_app::run()
}
