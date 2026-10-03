use dioxus::prelude::*;
use dxgraph::*;

fn graph() -> GraphModel<String, ()> {
    let mut graph = GraphModel::default();
    for id in ["root", "a", "b"] {
        graph
            .insert_node(GraphNode::new(id, id.to_string()))
            .unwrap();
    }
    graph
        .insert_edge(GraphEdge::new("ra", "root", "a", ()))
        .unwrap();
    graph
        .insert_edge(GraphEdge::new("rb", "root", "b", ()))
        .unwrap();
    graph
}
fn canvas() -> Element {
    let model = use_signal(graph);
    let controller = use_graph_controller();
    use_hook(move || {
        controller.set_viewport(Viewport {
            x: 11.0,
            y: 22.0,
            zoom: 0.8,
        })
    });
    rsx! {GraphCanvas{model,controller,render_node:move |node:NodeRenderContext<String>|rsx!{div{{node.data}}},node_label:move |id:NodeId|format!("Node {id}")}}
}
fn controls() -> Element {
    let controller = use_graph_controller();
    rsx! {GraphControls{controller}}
}
#[test]
fn canvas_has_nodes_paths_markers_transform_and_accessibility() {
    let mut dom = VirtualDom::new(canvas);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    assert_eq!(html.matches("data-dxgraph-node=").count(), 3, "{html}");
    assert!(html.contains("dxgraph-arrow-"));
    assert!(html.contains("data-dxgraph-edge=\"ra\""));
    assert!(html.contains("translate(11px,22px) scale(0.8)"));
    assert!(html.contains("role=\"application\""));
    assert!(html.contains("aria-label=\"Node root\""));
    assert!(html.contains("tabindex=0"), "{html}");
}
#[test]
fn controls_are_accessible() {
    let mut dom = VirtualDom::new(controls);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    for label in ["Zoom in", "Zoom out", "Fit graph", "Re-layout graph"] {
        assert!(html.contains(label));
    }
}
