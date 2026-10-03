use crate::{
    Rect,
    edge::{EdgeMarker, EdgeStyle, edge_geometry_with_offset},
};
use dioxus::prelude::*;

#[component]
pub(crate) fn EdgeView(
    id: String,
    source: Rect,
    target: Rect,
    style: EdgeStyle,
    marker: String,
    offset: i32,
    self_loop: bool,
    label: Option<String>,
) -> Element {
    let geometry = edge_geometry_with_offset(source, target, &style, offset, self_loop);
    let from = (style.from_end == EdgeMarker::Arrow).then(|| format!("url(#{marker})"));
    let to = (style.to_end == EdgeMarker::Arrow).then(|| format!("url(#{marker})"));
    let class = format!("dxgraph-edge {}", style.class.as_deref().unwrap_or(""));
    rsx! {
        g { class, "data-dxgraph-edge": id,
            path {
                d: geometry.path_d.clone(),
                fill: "none",
                marker_start: from,
                marker_end: to,
                stroke_dasharray: if style.dashed { "5 4" } else { "none" },
            }
            if let Some(label) = label {
                text {
                    class: "dxgraph-edge-label",
                    x: geometry.label_pos.x,
                    y: geometry.label_pos.y,
                    text_anchor: "middle",
                    {label}
                }
            }
        }
    }
}
