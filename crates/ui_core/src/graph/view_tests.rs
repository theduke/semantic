//! Exercise the actual view boundary and its pending initial load through Dioxus.
use super::{EntityGraphView, GraphMode};
use crate::{
    UiCatalog, UiCatalogContext, UiScopeContext,
    context::{provide_rpc_client, provide_toast_dispatcher},
};
use dioxus::prelude::*;
use dxgraph::LayoutConfig;
use semantic_data::value::Value;
use semantic_rpc::{RpcClient, RpcClientDyn};
use semantic_rpc_core::RpcClientError;
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Default)]
struct PendingClient {
    requests: Arc<Mutex<Vec<Value>>>,
    cancelled: Arc<AtomicUsize>,
    complete: bool,
}

struct PendingRequest(Arc<AtomicUsize>);
impl Drop for PendingRequest {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl RpcClientDyn for PendingClient {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> futures::future::BoxFuture<'static, Result<Value, RpcClientError>> {
        assert_eq!(command, "semantic.db.query");
        self.requests.lock().unwrap().push(payload.clone());
        if self.complete {
            let Value::Object(payload) = payload else {
                panic!("query payload")
            };
            let Some(Value::Object(params)) = payload.get("params") else {
                panic!("query params")
            };
            let rows = if params.get("limit").is_some() {
                Vec::new()
            } else {
                let Some(Value::List(ids)) = params.get("ids") else {
                    panic!("query ids")
                };
                ids.iter()
                    .map(|id| {
                        let mut object = semantic_data::value::Object::new();
                        object.insert("id", id.clone());
                        object.insert("semantic:title", Value::String("Loaded root".into()));
                        Value::Object(object)
                    })
                    .collect()
            };
            let mut response = semantic_data::value::Object::new();
            response.insert("rows", Value::List(rows));
            return Box::pin(async move { Ok(Value::Object(response)) });
        }
        let request = PendingRequest(self.cancelled.clone());
        Box::pin(async move {
            let _request = request;
            std::future::pending().await
        })
    }
}

#[derive(Clone)]
struct SessionInputs {
    root: String,
    mode: GraphMode,
    layout: LayoutConfig,
}

#[derive(Clone, Default)]
struct HarnessProps {
    client: PendingClient,
    inputs: Rc<Cell<Option<Signal<SessionInputs>>>>,
    scope: Rc<Cell<Option<Signal<Option<String>>>>>,
    controller: Rc<Cell<Option<dxgraph::GraphController>>>,
    supply_controller: bool,
}

fn view_harness(props: HarnessProps) -> Element {
    let inputs = use_signal(|| SessionInputs {
        root: "root-a".into(),
        mode: GraphMode::Hierarchy,
        layout: LayoutConfig::Manual,
    });
    let scope = use_signal(|| Some("scope-a".to_string()));
    let controller = dxgraph::use_graph_controller();
    let catalog = use_signal(|| Some(UiCatalog::empty()));
    use_context_provider(|| UiScopeContext::new(scope));
    use_context_provider(|| UiCatalogContext::new(catalog));
    provide_toast_dispatcher(4);
    let client = use_hook(|| RpcClient::new(props.client));
    provide_rpc_client(client);
    use_hook(|| {
        props.inputs.set(Some(inputs));
        props.scope.set(Some(scope));
        props.controller.set(Some(controller));
    });
    let inputs = inputs.read();
    rsx! {
        EntityGraphView {
            root: inputs.root.clone(),
            mode: inputs.mode,
            layout: inputs.layout.clone(),
            controller: props.supply_controller.then_some(controller),
        }
    }
}

struct Harness {
    dom: VirtualDom,
    props: HarnessProps,
}
impl Harness {
    fn new() -> Self {
        Self::with_controller(false)
    }
    fn with_controller(supply_controller: bool) -> Self {
        Self::with_props(HarnessProps {
            supply_controller,
            ..Default::default()
        })
    }
    fn with_props(props: HarnessProps) -> Self {
        let mut dom = VirtualDom::new_with_props(view_harness, props.clone());
        dom.rebuild_in_place();
        let mut harness = Self { dom, props };
        harness.flush();
        assert_eq!(
            harness.request_count(),
            if harness.props.client.complete { 2 } else { 1 }
        );
        harness
    }
    fn flush(&mut self) {
        for _ in 0..8 {
            self.dom.process_events();
            self.dom.render_immediate_to_vec();
        }
    }
    fn inputs(&self) -> Signal<SessionInputs> {
        self.props.inputs.get().expect("mounted harness inputs")
    }
    fn request_count(&self) -> usize {
        self.props.client.requests.lock().unwrap().len()
    }
    fn cancelled(&self) -> usize {
        self.props.client.cancelled.load(Ordering::SeqCst)
    }
    fn assert_remounted(&self) {
        assert_eq!(
            self.request_count(),
            2,
            "a new session must start its own initial load"
        );
        assert_eq!(
            self.cancelled(),
            1,
            "unmounting must cancel the old pending load"
        );
    }
}

#[test]
fn mode_change_remounts_the_entity_graph_session() {
    let mut harness = Harness::new();
    harness.inputs().write().mode = GraphMode::Relations;
    harness.flush();
    harness.assert_remounted();
}

#[test]
fn root_change_remounts_the_entity_graph_session() {
    let mut harness = Harness::new();
    harness.inputs().write().root = "root-b".into();
    harness.flush();
    harness.assert_remounted();
    let html = dioxus_ssr::render(&harness.dom);
    assert!(html.contains("root-b"));
    assert!(
        !html.contains("root-a"),
        "old explorer state must be removed"
    );
}

#[test]
fn scope_change_remounts_and_loads_the_current_scope() {
    let mut harness = Harness::new();
    harness
        .props
        .scope
        .get()
        .expect("mounted scope")
        .set(Some("scope-b".to_string()));
    harness.flush();
    harness.assert_remounted();
    let requests = harness.props.client.requests.lock().unwrap();
    let Value::Object(latest) = requests.last().expect("initial query request") else {
        panic!("query payload must be an object");
    };
    assert_eq!(
        latest.get("scope_id"),
        Some(&Value::String("scope-b".to_string()))
    );
}

#[test]
fn layout_change_preserves_the_entity_graph_session_and_pending_load() {
    let mut harness = Harness::new();
    harness.inputs().write().layout = LayoutConfig::default();
    harness.flush();
    assert_eq!(harness.request_count(), 1);
    assert_eq!(harness.cancelled(), 0);
}

#[test]
fn supplied_controller_reset_preserves_the_entity_graph_session_and_pending_load() {
    let mut harness = Harness::with_controller(true);
    harness.props.controller.get().unwrap().reset_positions();
    harness.flush();
    assert_eq!(harness.request_count(), 1);
    assert_eq!(harness.cancelled(), 0);
}

#[tokio::test]
async fn loaded_fallback_controller_preserves_layout_updates_and_remounts_with_root() {
    let mut harness = Harness::with_props(HarnessProps {
        client: PendingClient {
            complete: true,
            ..Default::default()
        },
        ..Default::default()
    });
    let html = dioxus_ssr::render(&harness.dom);
    assert!(html.contains("dxgraph-world"));
    assert!(html.contains("Loaded root"));
    harness.inputs().write().layout = LayoutConfig::default();
    harness.flush();
    assert_eq!(harness.request_count(), 2);
    assert!(dioxus_ssr::render(&harness.dom).contains("root-a"));
    harness.inputs().write().root = "root-b".into();
    harness.flush();
    assert_eq!(harness.request_count(), 4);
    let html = dioxus_ssr::render(&harness.dom);
    assert!(html.contains("root-b"));
    assert!(!html.contains("root-a"));
}

#[tokio::test]
async fn loaded_supplied_controller_reset_preserves_the_session_and_loaded_entities() {
    let mut harness = Harness::with_props(HarnessProps {
        client: PendingClient {
            complete: true,
            ..Default::default()
        },
        supply_controller: true,
        ..Default::default()
    });
    harness.props.controller.get().unwrap().reset_positions();
    harness.flush();
    assert_eq!(harness.request_count(), 2);
    assert!(dioxus_ssr::render(&harness.dom).contains("Loaded root"));
}
