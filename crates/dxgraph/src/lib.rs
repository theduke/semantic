//! Content-agnostic graph models, geometry, layouts, and Dioxus rendering.
use dioxus::prelude::*;
pub mod canvas;
pub use canvas::*;
pub use interaction::{NoDrag, NoWheel};
pub mod edge;
pub mod geometry;
pub mod interaction;
pub mod layout;
pub mod model;
pub use edge::*;
pub use geometry::*;
pub use layout::{
    LayoutAlgorithm, LayoutConfig, LayoutEdge, LayoutInput, LayoutNode, LayoutOutput, run_layout,
};
pub use model::*;

pub const CSS: &str = include_str!("dxgraph.css");
#[component]
pub fn Stylesheet() -> Element {
    rsx! { style { {CSS} } }
}
