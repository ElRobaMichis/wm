//! Compila la interfaz Slint (con las traducciones incluidas en el binario)
//! y embebe el manifiesto: DPI PerMonitorV2, `asInvoker`.

use embed_manifest::manifest::{DpiAwareness, ExecutionLevel};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/settings.slint", config)
        .expect("no se pudo compilar la interfaz Slint");

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(
            new_manifest("WindowManagment.WmSettings")
                .dpi_awareness(DpiAwareness::PerMonitorV2)
                .requested_execution_level(ExecutionLevel::AsInvoker),
        )
        .expect("no se pudo embeber el manifiesto");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=lang");
}
