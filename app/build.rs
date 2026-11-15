//! Embebe el manifiesto de la aplicación (architecture.md §12.1):
//! DPI PerMonitorV2, `asInvoker` (nunca pide admin), compatibilidad con
//! Windows 10/11 y Common Controls v6.

use embed_manifest::manifest::{DpiAwareness, ExecutionLevel};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(
            new_manifest("WindowManagment.Wm")
                .dpi_awareness(DpiAwareness::PerMonitorV2)
                .requested_execution_level(ExecutionLevel::AsInvoker),
        )
        .expect("no se pudo embeber el manifiesto");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
