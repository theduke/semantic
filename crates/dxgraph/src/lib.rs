//! Content-agnostic graph models, geometry, layouts, and Dioxus rendering.
use dioxus::prelude::*;
pub mod canvas;
pub use canvas::{
    CanvasConfig, GraphCanvas, GraphCanvasProps, GraphController, GraphControls, NodeDetail,
    NodeEvent, NodeRenderContext, use_graph_controller,
};
pub use interaction::{NoDrag, NoWheel};
pub mod edge;
pub mod geometry;
pub mod interaction;
pub mod layout;
pub mod model;
pub use edge::{
    EdgeAnchor, EdgeGeometry, EdgeMarker, EdgePathStyle, EdgeSide, EdgeStyle, edge_geometry,
    edge_geometry_with_offset, floating_anchor,
};
pub use geometry::{Point, Rect, Size, Viewport, ViewportLimits};
pub use layout::{
    LayoutAlgorithm, LayoutConfig, LayoutEdge, LayoutInput, LayoutNode, LayoutOutput, run_layout,
};
pub use model::{EdgeId, GraphEdge, GraphError, GraphModel, GraphNode, NodeId};

pub const CSS: &str = include_str!("dxgraph.css");
#[component]
pub fn Stylesheet() -> Element {
    rsx! {
        style { {CSS} }
    }
}
