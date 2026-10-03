//! Exercise the actual view boundary and its pending initial load through Dioxus.
use super::{EntityGraphView, GraphMode};
use crate::{
    EntityTarget, UiCatalog, UiCatalogContext, UiScopeContext,
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
        self.requests.lock().unwrap().push(payload);
        let request = PendingRequest(self.cancelled.clone());
        Box::pin(async move {
            let _request = request;
            std::future::pending().await
        })
    }
}

#[derive(Clone)]
struct SessionInputs {
    root: EntityTarget,
    mode: GraphMode,
    layout: LayoutConfig,
}

#[derive(Clone, Default)]
struct HarnessProps {
    client: PendingClient,
    inputs: Rc<Cell<Option<Signal<SessionInputs>>>>,
    scope: Rc<Cell<Option<Signal<Option<String>>>>>,
}

fn view_harness(props: HarnessProps) -> Element {
    let inputs = use_signal(|| SessionInputs {
        root: EntityTarget::default_collection("root-a"),
        mode: GraphMode::Hierarchy,
        layout: LayoutConfig::Manual,
    });
    let scope = use_signal(|| Some("scope-a".to_string()));
    let catalog = use_signal(|| Some(UiCatalog::empty()));
    use_context_provider(|| UiScopeContext::new(scope));
    use_context_provider(|| UiCatalogContext::new(catalog));
    provide_toast_dispatcher(4);
    let client = use_hook(|| RpcClient::new(props.client));
    provide_rpc_client(client);
    use_hook(|| {
        props.inputs.set(Some(inputs));
        props.scope.set(Some(scope));
    });
    let inputs = inputs.read();
    rsx! {
        EntityGraphView {
            root: inputs.root.clone(),
            mode: inputs.mode,
            layout: inputs.layout.clone(),
        }
    }
}

struct Harness {
    dom: VirtualDom,
    props: HarnessProps,
}
impl Harness {
    fn new() -> Self {
        let props = HarnessProps::default();
        let mut dom = VirtualDom::new_with_props(view_harness, props.clone());
        dom.rebuild_in_place();
        let mut harness = Self { dom, props };
        harness.flush();
        assert_eq!(harness.request_count(), 1);
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
    harness.inputs().write().root = EntityTarget::default_collection("root-b");
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
