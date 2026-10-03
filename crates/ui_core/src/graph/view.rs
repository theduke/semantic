use super::loading::{load_expansion, load_nodes};
use super::{
    EntityEdgeData, EntityEdgeKind, EntityGraphExplorer, EntityNodeData, ExplorerLimits, GraphMode,
    RpcGraphSource, node_id,
};
use crate::{
    EntityCard, EntityDisplayRenderer, EntityRenderOptions, EntityTarget, UiCatalog,
    context::{Toast, ToastDispatcher, use_toast_dispatcher},
    use_active_scope_id, use_rpc_client, use_ui_catalog,
};
use dioxus::prelude::*;
use dxgraph::layout::TreeDirection;
use dxgraph::{
    EdgeAnchor, EdgeSide, GraphCanvas, GraphController, GraphModel, LayoutConfig, NoDrag, NoWheel,
    NodeDetail, NodeEvent, NodeId, NodeRenderContext,
};

/// Root, mode and active scope own the lifetime of the explorer and its requests.
#[component]
pub fn EntityGraphView(
    root: EntityTarget,
    mode: GraphMode,
    layout: LayoutConfig,
    #[props(default)] controller: Option<GraphController>,
    #[props(default)] on_focus: Option<EventHandler<EntityTarget>>,
) -> Element {
    let scope = use_active_scope_id();
    let key = session_key(&root, mode, scope.as_deref());
    // Dioxus uses keys when reconciling dynamic sibling lists. A statically
    // positioned child keeps its scope even if its VNode key changes.
    rsx! {
        for key in [key] {
            EntityGraphSession {
                key: "{key}",
                root: root.clone(),
                mode,
                layout: layout.clone(),
                controller,
                on_focus,
            }
        }
    }
}

pub(super) fn session_key(root: &EntityTarget, mode: GraphMode, scope: Option<&str>) -> String {
    format!("{:?}/{scope:?}/{mode:?}", node_id(root))
}

#[derive(Clone)]
struct GraphLoadContext {
    explorer: Signal<EntityGraphExplorer>,
    source: RpcGraphSource,
    catalog: UiCatalog,
    toast: ToastDispatcher,
}
impl GraphLoadContext {
    async fn expand(&self, id: NodeId) {
        let mut explorer = self.explorer;
        let Some(request) = explorer.write().begin_expand(&id) else {
            return;
        };
        let known = explorer
            .peek()
            .model()
            .nodes()
            .filter_map(|node| node.data.target().cloned())
            .collect::<Vec<_>>();
        let result = load_expansion(&self.source, &request, &self.catalog, &known).await;
        // Collapse can cancel an expansion while its request is still pending.
        if !explorer.peek().is_loading(&id) {
            return;
        }
        match result {
            Ok(result) => explorer.write().apply_expansion(&id, result),
            Err(error) => {
                explorer.write().fail_expansion(&id);
                self.toast.show(Toast::error(error));
            }
        }
    }
    async fn start(&self, root: EntityTarget) {
        match load_nodes(&self.source, std::slice::from_ref(&root)).await {
            Ok(nodes) => {
                if let Some(object) = nodes
                    .into_iter()
                    .next()
                    .and_then(|node| node.object().cloned())
                {
                    let mut explorer = self.explorer;
                    explorer.write().set_object(&root, object);
                }
                self.expand(node_id(&root)).await;
            }
            Err(error) => {
                self.toast.show(Toast::error(error));
            }
        }
    }
}

fn presentation_model(
    explorer: &EntityGraphExplorer,
    layout: &LayoutConfig,
) -> GraphModel<EntityNodeData, EntityEdgeData> {
    let mut model = explorer.model().clone();
    let anchors = match layout {
        LayoutConfig::Tree(options) if options.direction == TreeDirection::TopDown => (
            EdgeAnchor::Side(EdgeSide::Bottom),
            EdgeAnchor::Side(EdgeSide::Top),
        ),
        LayoutConfig::Tree(_) => (
            EdgeAnchor::Side(EdgeSide::Right),
            EdgeAnchor::Side(EdgeSide::Left),
        ),
        _ => (EdgeAnchor::Floating, EdgeAnchor::Floating),
    };
    let edges = model
        .edges()
        .filter(|edge| edge.data.kind == EntityEdgeKind::Parent)
        .map(|edge| edge.id.clone())
        .collect::<Vec<_>>();
    for id in edges {
        if let Some(edge) = model.edge_mut(&id) {
            edge.style.source_anchor = anchors.0;
            edge.style.target_anchor = anchors.1;
        }
    }
    model
}

fn node_title(data: &EntityNodeData, catalog: &UiCatalog) -> String {
    match data {
        EntityNodeData::Entity { target, object, .. } => object
            .as_ref()
            .map(|object| catalog.entity_title(object))
            .unwrap_or_else(|| target.id.clone()),
        EntityNodeData::Ambiguous { id, .. } => id.clone(),
        EntityNodeData::Overflow { .. } => "More…".into(),
    }
}

#[component]
fn EntityGraphSession(
    root: EntityTarget,
    mode: GraphMode,
    layout: LayoutConfig,
    controller: Option<GraphController>,
    on_focus: Option<EventHandler<EntityTarget>>,
) -> Element {
    let catalog = use_ui_catalog();
    let source = RpcGraphSource::new(use_rpc_client(), use_active_scope_id());
    let mut explorer =
        use_signal(|| EntityGraphExplorer::new(root.clone(), mode, ExplorerLimits::default()));
    let mut selected = use_signal(|| None::<NodeId>);
    let mut ready = use_signal(|| false);
    let toast = use_toast_dispatcher();
    let load = GraphLoadContext {
        explorer,
        source,
        catalog: catalog.clone(),
        toast,
    };
    let initial = load.clone();
    use_future(move || {
        let initial = initial.clone();
        let root = root.clone();
        async move {
            initial.start(root).await;
            ready.set(true);
        }
    });
    let model = use_memo(use_reactive((&layout,), move |(layout,)| {
        presentation_model(&explorer.read(), &layout)
    }));
    let toggle_load = load.clone();
    let on_toggle = use_callback(move |id: NodeId| {
        if explorer.peek().is_expanded(&id) {
            explorer.write().collapse(&id);
        } else {
            let load = toggle_load.clone();
            spawn(async move { load.expand(id).await });
        }
    });
    let render_node = use_callback(move |context: NodeRenderContext<EntityNodeData>| {
        let id = context.id;
        rsx! {
            EntityGraphNode {
                data: context.data,
                detail: context.detail,
                on_toggle: move |_| on_toggle.call(id.clone()),
            }
        }
    });
    let node_catalog = catalog.clone();
    let node_label = use_callback(move |context: NodeRenderContext<EntityNodeData>| {
        node_title(&context.data, &node_catalog)
    });
    let detail = selected().and_then(|id| {
        explorer
            .read()
            .model()
            .node(&id)
            .map(|node| (id, node.data.clone()))
    });
    let load_parent = use_callback(move |id: NodeId| {
        let Some(target) = explorer.peek().ancestors_request(&id) else {
            return;
        };
        let load = load.clone();
        spawn(async move {
            match load_nodes(&load.source, &[target]).await {
                Ok(nodes) => {
                    if let Some(data) = nodes.into_iter().next() {
                        explorer.write().apply_ancestor(&id, data);
                    }
                }
                Err(error) => {
                    load.toast.show(Toast::error(error));
                }
            }
        });
    });
    let select_node = move |event: NodeEvent| selected.set(Some(event.id));
    let activate_node = move |event: NodeEvent| on_toggle.call(event.id);
    let pin_node = move |(id, position)| explorer.write().pin(&id, position);
    let select_nodes = move |ids: Vec<NodeId>| selected.set(ids.first().cloned());
    rsx! {
        style { {include_str!("graph.css")} }
        div { class: "semantic-entity-graph",
            details { class: "semantic-graph-list",
                summary { "List of loaded entities" }
                ul { aria_label: "Loaded graph entities",
                    for node in explorer.read().model().nodes() {
                        li { key: "{node.id}",
                            button {
                                r#type: "button",
                                onclick: {
                                    let id = node.id.clone();
                                    move |_| selected.set(Some(id.clone()))
                                },
                                {node_title(&node.data, &catalog)}
                            }
                            if let Some(target) = node.data.target() {
                                button {
                                    r#type: "button",
                                    onclick: {
                                        let id = node.id.clone();
                                        move |_| on_toggle.call(id.clone())
                                    },
                                    if node.data.expanded() {
                                        "Collapse"
                                    } else {
                                        "Expand"
                                    }
                                }
                                if let Some(href) = catalog
                                    .entity_navigation()
                                    .href
                                    .as_ref()
                                    .and_then(|build| build(target))
                                {
                                    a { href, "Open entity" }
                                }
                            }
                        }
                    }
                }
            }
            if ready() {
                GraphCanvas::<EntityNodeData,EntityEdgeData> {
                    model,
                    render_node,
                    node_label,
                    layout,
                    controller,
                    on_node_click: select_node,
                    on_node_activate: activate_node,
                    on_node_moved: pin_node,
                    on_selection_change: select_nodes,
                    if let Some((id, data)) = detail {
                        NoDrag {
                            NoWheel {
                                aside {
                                    class: "semantic-graph-detail",
                                    aria_label: "Selected entity",
                                    button {
                                        r#type: "button",
                                        onclick: move |_| selected.set(None),
                                        "Close"
                                    }
                                    if let Some(target) = data.target() {
                                        if let Some(object) = data.object().cloned() {
                                            EntityCard {
                                                object,
                                                options: EntityRenderOptions {
                                                    collection: target.collection.clone(),
                                                    id: Some(target.id.clone()),
                                                    renderer: EntityDisplayRenderer::Custom,
                                                    preview: true,
                                                    actions: true,
                                                },
                                            }
                                        } else {
                                            p { "Entity unavailable: {target.id}" }
                                        }
                                        button {
                                            r#type: "button",
                                            onclick: {
                                                let id = id.clone();
                                                move |_| on_toggle.call(id.clone())
                                            },
                                            if data.expanded() {
                                                "Collapse"
                                            } else {
                                                "Expand"
                                            }
                                        }
                                        if mode != GraphMode::Relations && explorer.read().ancestors_request(&id).is_some() {
                                            button {
                                                r#type: "button",
                                                onclick: {
                                                    let id = id.clone();
                                                    move |_| load_parent.call(id.clone())
                                                },
                                                "Load parent"
                                            }
                                        }
                                        if let Some(focus) = on_focus {
                                            button {
                                                r#type: "button",
                                                onclick: {
                                                    let target = target.clone();
                                                    move |_| focus.call(target.clone())
                                                },
                                                "Focus here"
                                            }
                                        }
                                        if let Some(open) = catalog.entity_navigation().open.clone() {
                                            button {
                                                r#type: "button",
                                                onclick: {
                                                    let target = target.clone();
                                                    move |_| open(target.clone())
                                                },
                                                "Open"
                                            }
                                        }
                                    } else if matches!(data, EntityNodeData::Ambiguous { .. }) {
                                        p {
                                            "The relationship index does not identify a unique collection for this entity."
                                        }
                                    } else {
                                        p { "More entities exist beyond the graph's display limit." }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                p { role: "status", "Loading graph…" }
            }
        }
    }
}

#[component]
pub fn EntityGraphNode(
    data: EntityNodeData,
    #[props(default)] detail: NodeDetail,
    #[props(default)] on_toggle: Option<EventHandler<()>>,
) -> Element {
    let catalog = use_ui_catalog();
    let title = node_title(&data, &catalog);
    let class = data
        .object()
        .and_then(|object| catalog.object_class(object));
    let class_name = class
        .map(|class| {
            class
                .meta
                .title
                .clone()
                .unwrap_or_else(|| class.name.clone())
        })
        .unwrap_or_else(|| {
            if data.object().is_some() {
                "Unknown class"
            } else {
                "Unresolved entity"
            }
            .into()
        });
    let color =
        super::explorer::stable_color(class.map(|class| class.id.as_str()).unwrap_or("unknown"));
    let hues = [240, 285, 330, 35, 90, 175, 205, 220];
    let hue = hues[color as usize];
    let kind = match &data {
        EntityNodeData::Overflow { .. } => "overflow",
        EntityNodeData::Ambiguous { .. } => "ambiguous",
        EntityNodeData::Entity { object: None, .. } => "unresolved",
        _ => "entity",
    };
    let loading = data.loading();
    let expanded = data.expanded();
    let toggle = move |event: MouseEvent| {
        event.stop_propagation();
        if let Some(callback) = on_toggle {
            callback.call(());
        }
    };
    rsx! {
        article {
            class: "semantic-graph-node",
            style: "--semantic-graph-accent:var(--dx-palette-{color},hsl({hue} 45% 55%));",
            "data-entity-kind": kind,
            strong { class: "semantic-graph-node__title", "{title}" }
            if detail == NodeDetail::Full {
                if let Some(target) = data.target() {
                    span { class: "semantic-graph-node__class", "{class_name}" }
                    span { class: "semantic-graph-node__id", "{target.id}" }
                    NoDrag {
                        button {
                            r#type: "button",
                            aria_label: if expanded { "Collapse node" } else { "Expand node" },
                            disabled: loading,
                            onclick: toggle,
                            if loading {
                                "…"
                            } else if expanded {
                                "−"
                            } else {
                                "+"
                            }
                        }
                    }
                } else if matches!(data, EntityNodeData::Ambiguous { .. }) {
                    span { class: "semantic-graph-node__class", "Unresolved collection" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_ssr_covers_entity_unresolved_and_overflow() {
        fn app() -> Element {
            let catalog = use_signal(|| Some(UiCatalog::empty()));
            use_context_provider(|| crate::UiCatalogContext::new(catalog));
            let mut object = semantic_data::value::Object::new();
            object.insert(
                "semantic:title",
                semantic_data::value::Value::String("Example entity".into()),
            );
            rsx! {
                EntityGraphNode { data: super::super::entity_data(EntityTarget::default_collection("a"), Some(object)) }
                EntityGraphNode { data: super::super::entity_data(EntityTarget::default_collection("missing"), None) }
                EntityGraphNode {
                    data: EntityNodeData::Overflow {
                        parent: NodeId::from("a"),
                    },
                }
            }
        }
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let rendered = dioxus_ssr::render(&dom);
        for text in [
            "Example entity",
            "missing",
            "More…",
            "Unknown class",
            "data-entity-kind=\"entity\"",
            "data-entity-kind=\"unresolved\"",
            "data-entity-kind=\"overflow\"",
        ] {
            assert!(rendered.contains(text), "missing {text}: {rendered}");
        }
    }
    #[test]
    fn hierarchy_anchors_follow_layout_presentation() {
        let root = EntityTarget::default_collection("root");
        let child = EntityTarget::default_collection("child");
        let mut graph =
            EntityGraphExplorer::new(root.clone(), GraphMode::Both, ExplorerLimits::default());
        graph.apply_expansion(
            &node_id(&root),
            super::super::ExpansionResult {
                nodes: vec![super::super::entity_data(child.clone(), None)],
                edges: vec![super::super::ExpansionEdge {
                    source: node_id(&root),
                    target: node_id(&child),
                    kind: EntityEdgeKind::Parent,
                }],
            },
        );
        assert_eq!(
            graph.model().edges().next().unwrap().style.source_anchor,
            EdgeAnchor::Floating
        );
        let tree = presentation_model(&graph, &LayoutConfig::Tree(Default::default()));
        assert_eq!(
            tree.edges().next().unwrap().style.source_anchor,
            EdgeAnchor::Side(EdgeSide::Bottom)
        );
        for layout in [
            LayoutConfig::MindMap(Default::default()),
            LayoutConfig::Radial(Default::default()),
            LayoutConfig::Force(Default::default()),
        ] {
            let model = presentation_model(&graph, &layout);
            assert_eq!(
                model.edges().next().unwrap().style.source_anchor,
                EdgeAnchor::Floating
            );
            assert_eq!(
                model.edges().next().unwrap().style.target_anchor,
                EdgeAnchor::Floating
            );
        }
    }
}
