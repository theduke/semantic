use wasm_bindgen::{prelude::*, JsCast};

use semantic_ui_core::api::api;

use crate::components::root::Root;

mod base_plugin;
mod components;

#[wasm_bindgen(start)]
pub fn boot() {
    console_error_panic_hook::set_once();
    let conf = tracing_wasm::WASMLayerConfigBuilder::new()
        .set_console_config(tracing_wasm::ConsoleConfig::ReportWithoutConsoleColor)
        .build();
    tracing_wasm::set_as_global_default_with_config(conf);

    tracing::info!("tracing initialized");
    let elem = brass::util::query_selector("#app")
        .expect("Could not get app")
        .dyn_into()
        .unwrap();
    brass::boot::<Root>((), elem);
}
