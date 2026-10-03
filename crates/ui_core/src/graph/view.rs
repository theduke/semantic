use super::loading::{load_expansion, load_nodes};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InitialGraphPhase {
    #[default]
    Root,
    Neighborhood,
    Ready,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct InitialGraphLoad {
    generation: u64,
    phase: InitialGraphPhase,
}
impl InitialGraphLoad {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            phase: InitialGraphPhase::Root,
        }
    }
    fn root_loaded(&mut self, generation: u64) {
        if self.generation == generation && self.phase == InitialGraphPhase::Root {
            self.phase = InitialGraphPhase::Neighborhood;
        }
    }
    fn finish(&mut self, generation: u64) {
        if self.generation == generation && self.phase == InitialGraphPhase::Neighborhood {
            self.phase = InitialGraphPhase::Ready;
        }
    }
    fn fail(&mut self, generation: u64) {
        if self.generation == generation {
            self.phase = InitialGraphPhase::Ready;
        }
    }
    fn ready(self) -> bool {
        self.phase == InitialGraphPhase::Ready
    }
}
use super::{
    EntityGraphExplorer, EntityNodeData, EntityNodeKind, ExplorerLimits, GraphMode, RpcGraphSource,
    node_id,
};
use crate::{
    EntityCard, EntityDisplayRenderer, EntityRenderOptions, EntityTarget, UiCatalog,
    context::{Toast, use_toast_dispatcher},
    use_active_scope_id, use_rpc_client, use_ui_catalog,
};
use dioxus::prelude::*;
use dxgraph::{
    GraphCanvas, GraphController, LayoutConfig, NoDrag, NodeDetail, NodeEvent, NodeId,
    NodeRenderContext,
};

#[component]
pub fn EntityGraphView(
    root: EntityTarget,
    mode: GraphMode,
    layout: LayoutConfig,
    #[props(default)] layout_revision: u64,
    #[props(default)] controller: Option<GraphController>,
    #[props(default)] on_focus: Option<EventHandler<EntityTarget>>,
) -> Element {
    let catalog = use_ui_catalog();
    let source = RpcGraphSource::new(use_rpc_client(), use_active_scope_id());
    let mut explorer =
        use_signal(|| EntityGraphExplorer::new(root.clone(), mode, ExplorerLimits::default()));
    let mut selected = use_signal(|| None::<NodeId>);
    let mut generation = use_signal(|| 0u64);
    let mut initial_load = use_signal(InitialGraphLoad::default);
    let toast = use_toast_dispatcher();
    let initialize_source = source.clone();
    let initialize_catalog = catalog.clone();
    use_effect(use_reactive((&root, &mode), move |(root, mode)| {
        explorer.set(EntityGraphExplorer::new(
            root.clone(),
            mode,
            ExplorerLimits::default(),
        ));
        selected.set(None);
        let next = *generation.peek() + 1;
        generation.set(next);
        initial_load.set(InitialGraphLoad::new(next));
        let source = initialize_source.clone();
        let catalog = initialize_catalog.clone();
        spawn(async move {
            match load_nodes(&source, std::slice::from_ref(&root)).await {
                Ok(nodes) if *generation.peek() == next => {
                    if let Some(object) = nodes.into_iter().next().and_then(|node| node.object) {
                        explorer.write().set_object(&root, object);
                    }
                    initial_load.write().root_loaded(next);
                    expand(
                        explorer,
                        generation,
                        next,
                        node_id(&root),
                        source,
                        catalog,
                        toast,
                    )
                    .await;
                    initial_load.write().finish(next);
                }
                Err(error) if *generation.peek() == next => {
                    toast.show(Toast::error(error));
                    initial_load.write().fail(next);
                }
                _ => {}
            }
        });
    }));
    let model = use_memo(move || explorer.read().model().clone());
    let toggle_source = source.clone();
    let toggle_catalog = catalog.clone();
    let on_toggle = use_callback(move |id: NodeId| {
        if explorer.peek().is_expanded(&id) {
            explorer.write().collapse(&id);
            return;
        }
        let source = toggle_source.clone();
        let catalog = toggle_catalog.clone();
        let token = *generation.peek();
        spawn(expand(
            explorer, generation, token, id, source, catalog, toast,
        ));
    });
    let node_catalog = catalog.clone();
    let render_node = use_callback(move |context: NodeRenderContext<EntityNodeData>| {
        let loading = explorer.read().is_loading(&context.id);
        let expanded = explorer.read().is_expanded(&context.id);
        rsx! { EntityGraphNode { data: context.data, detail: context.detail, loading, expanded, on_toggle: move |_| on_toggle.call(context.id.clone()) } }
    });
    let node_label = use_callback(move |id: NodeId| {
        explorer
            .read()
            .model()
            .node(&id)
            .map(|node| {
                node.data
                    .object
                    .as_ref()
                    .map(|object| node_catalog.entity_title(object))
                    .unwrap_or_else(|| node.data.target.id.clone())
            })
            .unwrap_or_else(|| id.0)
    });
    let detail = selected().and_then(|id| {
        explorer
            .read()
            .model()
            .node(&id)
            .map(|node| (id, node.data.clone()))
    });
    let ancestor_source = source.clone();
    let load_parent = use_callback(move |id: NodeId| {
        let Some(target) = explorer.peek().ancestors_request(&id) else {
            return;
        };
        let source = ancestor_source.clone();
        let token = *generation.peek();
        spawn(async move {
            match load_nodes(&source, &[target]).await {
                Ok(nodes) if *generation.peek() == token => {
                    if let Some(data) = nodes.into_iter().next() {
                        explorer.write().apply_ancestor(&id, data);
                    }
                }
                Err(error) if *generation.peek() == token => {
                    toast.show(Toast::error(error));
                }
                _ => {}
            }
        });
    });
    let focus_source = source.clone();
    let focus_catalog = catalog.clone();
    let focus_here = use_callback(move |target: EntityTarget| {
        if let Some(callback) = on_focus {
            callback.call(target);
            return;
        }
        let object = explorer
            .peek()
            .model()
            .node(&node_id(&target))
            .and_then(|node| node.data.object.clone());
        explorer.set(EntityGraphExplorer::new(
            target.clone(),
            mode,
            ExplorerLimits::default(),
        ));
        if let Some(object) = object {
            explorer.write().set_object(&target, object);
        }
        selected.set(None);
        let token = *generation.peek() + 1;
        generation.set(token);
        initial_load.set(InitialGraphLoad::new(token));
        initial_load.write().root_loaded(token);
        let source = focus_source.clone();
        let catalog = focus_catalog.clone();
        spawn(async move {
            expand(
                explorer,
                generation,
                token,
                node_id(&target),
                source,
                catalog,
                toast,
            )
            .await;
            initial_load.write().finish(token);
        });
    });
    rsx! {
        style { {include_str!("graph.css")} }
        div { class: "semantic-entity-graph",
            details { class: "semantic-graph-list",
                summary { "List of loaded entities" }
                ul { aria_label: "Loaded graph entities",
                    for node in explorer.read().model().nodes() {
                        li { key: "{node.id}",
                            button { r#type: "button", onclick: { let id = node.id.clone(); move |_| selected.set(Some(id.clone())) },
                                {node.data.object.as_ref().map(|object| catalog.entity_title(object)).unwrap_or_else(|| node.data.target.id.clone())}
                            }
                            if !matches!(node.data.kind, EntityNodeKind::Overflow { .. }) {
                                button { r#type: "button", onclick: { let id = node.id.clone(); move |_| on_toggle.call(id.clone()) },
                                    if explorer.read().is_expanded(&node.id) { "Collapse" } else { "Expand" }
                                }
                                if let Some(href) = catalog.entity_navigation().href.as_ref().and_then(|build| build(&node.data.target)) {
                                    a { href, "Open entity" }
                                }
                            }
                        }
                    }
                }
            }
            if initial_load().ready() {
            GraphCanvas::<EntityNodeData, super::EntityEdgeData> {
                key: "{node_id(&explorer.read().root)}",
                model: model, render_node, node_label, layout, layout_revision, controller,
                on_node_click: move |event: NodeEvent| selected.set(Some(event.id)),
                on_node_activate: move |event: NodeEvent| on_toggle.call(event.id),
                on_node_double_click: move |event: NodeEvent| on_toggle.call(event.id),
                on_node_moved: move |(id, position)| { let mut state = explorer.write(); let _ = state.model_mut().set_position(&id, position); let _ = state.model_mut().set_pinned(&id, true); },
                on_selection_change: move |ids: Vec<NodeId>| selected.set(ids.first().cloned()),
                if let Some((id, data)) = detail {
                    aside { class: "semantic-graph-detail", aria_label: "Selected entity", onpointerdown: move |event| event.stop_propagation(), onwheel: move |event| event.stop_propagation(),
                        button { r#type: "button", onclick: move |_| selected.set(None), "Close" }
                        if let Some(object) = data.object.clone() {
                            EntityCard { object, options: EntityRenderOptions { collection: data.target.collection.clone(), id: Some(data.target.id.clone()), renderer: EntityDisplayRenderer::Custom, preview: true, actions: true } }
                        } else { p { "Entity unavailable: {data.target.id}" } }
                        if !matches!(data.kind, EntityNodeKind::Overflow { .. }) {
                            button { r#type: "button", onclick: { let id = id.clone(); move |_| on_toggle.call(id.clone()) }, if explorer.read().is_expanded(&id) { "Collapse" } else { "Expand" } }
                            if mode != GraphMode::Relations && explorer.read().ancestors_request(&id).is_some() {
                                button { r#type: "button", onclick: { let parent_id = id.clone(); move |_| load_parent.call(parent_id.clone()) }, "Load parent" }
                            }
                            button { r#type: "button", onclick: { let target = data.target.clone(); move |_| focus_here.call(target.clone()) }, "Focus here" }
                            if let Some(open) = catalog.entity_navigation().open.clone() {
                                button { r#type: "button", onclick: { let target = data.target.clone(); move |_| open(target.clone()) }, "Open" }
                            }
                        }
                    }
                }
            }
            } else { p { role: "status", "Loading graph…" } }
        }
    }
}

async fn expand(
    mut explorer: Signal<EntityGraphExplorer>,
    generation: Signal<u64>,
    token: u64,
    id: NodeId,
    source: RpcGraphSource,
    catalog: UiCatalog,
    toast: crate::context::ToastDispatcher,
) {
    let Some(request) = explorer.write().begin_expand(&id) else {
        return;
    };
    let known = explorer
        .peek()
        .model()
        .nodes()
        .map(|node| node.data.target.clone())
        .collect::<Vec<_>>();
    let result = load_expansion(&source, &request, &catalog, &known).await;
    if *generation.peek() != token {
        return;
    }
    // A collapsed or removed expansion must not reappear when its old request completes.
    if !explorer.peek().is_loading(&id) {
        return;
    }
    match result {
        Ok(result) => explorer.write().apply_expansion(&id, result),
        Err(error) => {
            explorer.write().fail_expansion(&id);
            toast.show(Toast::error(error));
        }
    }
}

#[component]
pub fn EntityGraphNode(
    data: EntityNodeData,
    #[props(default)] detail: NodeDetail,
    #[props(default)] loading: bool,
    #[props(default)] expanded: bool,
    #[props(default)] on_toggle: Option<EventHandler<()>>,
) -> Element {
    let catalog = use_ui_catalog();
    let title = match &data.kind {
        EntityNodeKind::Overflow { remaining_hint, .. } => format!("+{remaining_hint} more"),
        _ => data
            .object
            .as_ref()
            .map(|object| catalog.entity_title(object))
            .unwrap_or_else(|| data.target.id.clone()),
    };
    let class = data
        .object
        .as_ref()
        .and_then(|object| catalog.object_class(object));
    let class_name = class
        .map(|class| {
            class
                .meta
                .title
                .clone()
                .unwrap_or_else(|| class.name.clone())
        })
        .unwrap_or_else(|| "Unresolved entity".into());
    let color =
        super::explorer::stable_color(class.map(|class| class.id.as_str()).unwrap_or("unknown"));
    let overflow = matches!(data.kind, EntityNodeKind::Overflow { .. });
    rsx! {
        article { class: "semantic-graph-node", style: "--semantic-graph-accent:var(--dx-palette-{color},hsl({color}00 45% 55%));", "data-entity-kind": if overflow { "overflow" } else if data.object.is_none() { "unresolved" } else { "entity" },
            strong { class: "semantic-graph-node__title", "{title}" }
            if detail == NodeDetail::Full && !overflow {
                span { class: "semantic-graph-node__class", "{class_name}" }
                span { class: "semantic-graph-node__id", "{data.target.id}" }
                NoDrag {
                    button { r#type: "button", aria_label: if expanded { "Collapse node" } else { "Expand node" }, disabled: loading,
                        onclick: move |event| { event.stop_propagation(); if let Some(callback) = on_toggle { callback.call(()); } },
                        if loading { "…" } else if expanded { "−" } else { "+" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canvas_waits_for_neighborhood_and_old_load_cannot_reveal_new_root() {
        let mut load = InitialGraphLoad::new(1);
        load.finish(1);
        assert!(!load.ready());
        load.root_loaded(1);
        assert!(!load.ready());
        load.finish(1);
        assert!(load.ready());
        load = InitialGraphLoad::new(2);
        load.root_loaded(1);
        load.finish(1);
        load.fail(1);
        assert!(!load.ready());
        load.root_loaded(2);
        load.finish(2);
        assert!(load.ready());
    }
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
                EntityGraphNode { data: EntityNodeData { target: EntityTarget::default_collection("overflow"), object: None, kind: EntityNodeKind::Overflow { parent: NodeId::from("a"), remaining_hint: 12 } } }
            }
        }
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let rendered = dioxus_ssr::render(&dom);
        for text in [
            "Example entity",
            "missing",
            "+12 more",
            "data-entity-kind=\"entity\"",
            "data-entity-kind=\"unresolved\"",
            "data-entity-kind=\"overflow\"",
        ] {
            assert!(rendered.contains(text), "missing {text}: {rendered}");
        }
    }
}
