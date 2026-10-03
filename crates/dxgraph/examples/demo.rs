//! Native 300-node canvas smoke test. Run with `cargo run -p dxgraph --example demo --features demo`.
use dioxus::prelude::*;
use dxgraph::{
    GraphCanvas, GraphControls, GraphEdge, GraphModel, GraphNode, NodeId, NodeRenderContext,
    Stylesheet, use_graph_controller,
};

fn main() {
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::WindowBuilder::new()
                    .with_title("dxgraph 300-node demo")
                    .with_inner_size(dioxus::desktop::LogicalSize::new(1280.0, 900.0)),
            ),
        )
        .launch(App);
}

#[allow(non_snake_case)]
fn App() -> Element {
    let model = use_signal(|| {
        let mut model = GraphModel::<usize, ()>::default();
        for index in 0..300 {
            let mut node = GraphNode::new(format!("node-{index:03}"), index);
            node.layout_parent =
                (index > 0).then(|| NodeId::from(format!("node-{:03}", (index - 1) / 2)));
            let parent = node.layout_parent.clone();
            let id = node.id.clone();
            model.upsert_node(node);
            if let Some(parent) = parent {
                model
                    .insert_edge(GraphEdge::new(format!("edge-{index:03}"), parent, id, ()))
                    .expect("the parent and child have both been inserted");
            }
        }
        model
    });
    let controller = use_graph_controller();
    rsx! {
        Stylesheet {}
        style {
            {
                "html,body,#main{margin:0;width:100%;height:100%;font-family:system-ui;}button{padding:8px;}"
            }
        }
        div { style: "width:100vw;height:100vh;",
            GraphCanvas::<usize,()> {
                model,
                controller,
                render_node: |context: NodeRenderContext<usize>| rsx! {
                    div { style: "box-sizing:border-box;width:160px;min-height:64px;padding:16px;background:#fff;border:1px solid #aaa;border-radius:6px;",
                        "Node {context.data}"
                    }
                },
                node_label: |node: NodeRenderContext<usize>| format!("Node {}", node.data),
                div { style: "position:absolute;top:16px;left:16px;pointer-events:auto;display:flex;gap:8px;",
                    button {
                        onclick: move |_| {
                            let current = controller.viewport();
                            controller.zoom_by(1.0 / current.zoom);
                            controller.center_on("node-000".into());
                        },
                        "Focus root at 1x"
                    }
                    button { onclick: move |_| controller.fit_view(), "Fit 300 nodes" }
                }
                GraphControls { controller }
            }
        }
    }
}
