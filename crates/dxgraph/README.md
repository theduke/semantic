# dxgraph

A content-agnostic Dioxus graph canvas. Nodes contain arbitrary HTML components;
edges are SVG paths. Layouts and the gesture reducer are pure Rust and work on
native and web targets.

```rust,no_run
use dioxus::prelude::*;
use dxgraph::*;

fn App() -> Element {
    let model = use_signal(|| {
        let mut graph = GraphModel::<String, ()>::default();
        let _ = graph.insert_node(GraphNode::new("root", "Hello".into()));
        graph
    });
    let controller = use_graph_controller();
    rsx! {
        Stylesheet {}
        div { style: "height:600px",
            GraphCanvas {
                model,
                controller,
                render_node: |node: NodeRenderContext<String>| rsx! { div { {node.data} } },
                node_label: |node: NodeRenderContext<String>| node.data,
                GraphControls { controller }
            }
        }
    }
}
```

Dioxus/Rust owns the model, world positions, selection, and viewport. The
viewport signal is read by the world wrapper and dotted background, while node
and edge components receive viewport-independent props. Culling uses a margin
and 15% zoom hysteresis. Full, compact and minimal detail thresholds default to
0.6 and 0.3; minimal nodes use a placeholder instead of invoking the full renderer.

Measured unscaled Full-detail border-box sizes replace each node's `size_hint`.
Compact and Minimal measurements never change layout sizes. All valid Full sizes
update anchors and culling; changes within eight pixels do not trigger layout.
Resize bursts request at most one layout per queued tick. A 100 ms fallback reveals
nodes whose renderer does not provide resize notifications.

Wrap buttons, links, inputs and editors in `NoDrag` so they do not start gestures.
Wrap scrollable content in `NoWheel` to retain its wheel behavior. These wrappers
stop Dioxus events; no custom JavaScript is used. The `web` feature adds pointer
capture through web-sys; native drags end when the pointer leaves the canvas.

Layouts include a variable-size tidy tree (top-down or left-right), a balanced
left/right mind map, deterministic warm-started force layout with rectangle
collision removal, concentric radial rings, and manual positions. Implement
`LayoutAlgorithm` and pass `LayoutConfig::Custom(Rc::new(engine))` to supply
another engine. Custom engine identity is compared by `Rc` pointer. Fixed nodes
are preserved, disconnected components are packed, and
cycles become a spanning forest for tree layouts.

The canvas keeps dragged positions for its lifetime and emits `on_node_moved`.
The caller decides whether to persist the position and set the model node's
`pinned` flag. `controller.reset_positions()` clears canvas drag positions and
returns them to automatic layout. Explicit model pins remain caller-owned and fixed.
Incremental expansion retains existing positions. Calling
`controller.relayout()` or changing `layout` requests
a fresh layout, while user-moved and pinned nodes remain fixed.

Keyboard: Enter activates a focused node, Space selects, Escape clears selection,
`+`/`-` zoom, `0` fits, and arrow keys pan. Shift-click toggles selection.
`GraphController` also provides `fit_view`, `zoom_by`, `center_on` and
`set_viewport`. Each node needs an accessible label through `node_label`.

GraphModel, node/edge IDs, geometry, and styles support Serde serialization. Future persisted canvas schemas can map their positions,
sizes, anchors and markers onto JSON Canvas without coupling dxgraph to entities.
