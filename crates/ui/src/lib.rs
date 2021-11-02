use wasm_bindgen::prelude::*;

// use crate::components::root::Root;

mod components;

#[wasm_bindgen]
pub fn boot() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();

    tracing::info!("tracing initialized");

    let doc = web_sys::window().unwrap().document().unwrap();
    let root = doc.create_element("div").unwrap();
    root.set_id("app");

    let body = doc.body().unwrap();
    body.append_child(&root).unwrap();

    brass::launch(root, || components::boot::Boot::render());
}
