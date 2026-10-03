pub mod gesture;
pub mod input;
pub mod platform;
use dioxus::prelude::*;
pub use gesture::*;

/// Wrap interactive node content so it does not start a node drag.
#[component]
pub fn NoDrag(children: Element) -> Element {
    rsx! { div { class:"dxgraph-nodrag", onpointerdown:move |event|event.stop_propagation(), onclick:move |event|event.stop_propagation(), onkeydown:move |event|event.stop_propagation(), {children} } }
}
/// Wrap scrollable node content so it retains its own wheel behavior.
#[component]
pub fn NoWheel(children: Element) -> Element {
    rsx! { div { class:"dxgraph-nowheel", onwheel:move |event|event.stop_propagation(), {children} } }
}
