use super::*;
use crate::{UiScopeContext, provide_rpc_client};
use dioxus::core::{AttributeValue, ElementId, Mutation};
use futures::channel::oneshot;
use semantic_base::labels::encode_labels;
use semantic_data::value::Value;
use semantic_rpc::{RpcClient, RpcClientDyn};
use semantic_rpc_core::RpcClientError;
use std::{
    cell::Cell,
    collections::BTreeMap,
    rc::Rc,
    sync::{Arc, Mutex},
};

struct Request {
    command: String,
    payload: Value,
    respond: oneshot::Sender<Value>,
}

#[derive(Clone, Default)]
struct Client(Arc<Mutex<Vec<Request>>>);

impl RpcClientDyn for Client {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> futures::future::BoxFuture<'static, Result<Value, RpcClientError>> {
        let (respond, response) = oneshot::channel();
        self.0.lock().unwrap().push(Request {
            command,
            payload,
            respond,
        });
        Box::pin(async move { Ok(response.await.expect("test response sender dropped")) })
    }
}

impl Client {
    fn take(&self, suffix: &str) -> Request {
        let mut requests = self.0.lock().unwrap();
        let index = requests
            .iter()
            .position(|request| request.command.ends_with(suffix))
            .expect("expected RPC call");
        requests.remove(index)
    }
}

#[derive(Clone)]
struct HarnessProps {
    client: Client,
    target: Rc<Cell<Option<Signal<EntityTarget>>>>,
    scope: Rc<Cell<Option<Signal<Option<String>>>>>,
    closed: Rc<Cell<usize>>,
    changed: Rc<Cell<usize>>,
}

fn harness(props: HarnessProps) -> Element {
    let target = use_signal(|| EntityTarget::default_collection("first"));
    let scope = use_signal(|| Some("scope-a".to_string()));
    use_context_provider(|| UiScopeContext::new(scope));
    use_hook(|| {
        props.target.set(Some(target));
        props.scope.set(Some(scope));
    });
    let rpc = use_hook(|| RpcClient::new(props.client));
    provide_rpc_client(rpc);
    rsx! {
        LabelEditor {
            target: target(),
            on_close: move |_| props.closed.set(props.closed.get() + 1),
            on_changed: move |_| props.changed.set(props.changed.get() + 1),
        }
    }
}

struct Harness {
    dom: VirtualDom,
    props: HarnessProps,
    accessible: BTreeMap<String, ElementId>,
    text: Vec<String>,
}

impl Harness {
    fn new() -> Self {
        dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
        let props = HarnessProps {
            client: Client::default(),
            target: Rc::default(),
            scope: Rc::default(),
            closed: Rc::default(),
            changed: Rc::default(),
        };
        let mut dom = VirtualDom::new_with_props(harness, props.clone());
        let edits = dom.rebuild_to_vec().edits;
        let mut this = Self {
            dom,
            props,
            accessible: BTreeMap::new(),
            text: vec![],
        };
        this.observe(edits);
        this.flush();
        this
    }

    fn observe(&mut self, edits: Vec<Mutation>) {
        for edit in edits {
            match edit {
                Mutation::SetAttribute {
                    name: "aria-label",
                    value: AttributeValue::Text(value),
                    id,
                    ..
                } => {
                    self.accessible.insert(value, id);
                }
                Mutation::CreateTextNode { value, .. } | Mutation::SetText { value, .. } => {
                    self.text.push(value)
                }
                _ => {}
            }
        }
    }

    fn flush(&mut self) {
        // Run ready tasks and their resulting renders to quiescence. Pending RPC
        // futures remain suspended until the test explicitly supplies a response.
        for _ in 0..10 {
            self.dom.process_events();
            let edits = self.dom.render_immediate_to_vec().edits;
            self.observe(edits);
        }
    }

    fn click(&mut self, label: &str) {
        let id = *self.accessible.get(label).expect("accessible button");
        let data = dioxus_html::PlatformEventData::new(Box::new(
            dioxus_html::SerializedMouseData::default(),
        ));
        self.dom.runtime().handle_event(
            "click",
            Event::new(Rc::new(data) as Rc<dyn std::any::Any>, true),
            id,
        );
        self.flush();
    }

    fn complete_load(&mut self, name: &str) {
        let labels = vec![Label::new(name, name)];
        self.props
            .client
            .take(".list")
            .respond
            .send(encode_labels(labels.clone()))
            .unwrap();
        self.props
            .client
            .take(".load")
            .respond
            .send(encode_labels(labels))
            .unwrap();
        self.flush();
        assert!(self.accessible.contains_key(&format!("Remove {name}")));
    }

    fn change_identity(&mut self, change: &str) {
        match change {
            "target" => self
                .props
                .target
                .get()
                .unwrap()
                .set(EntityTarget::default_collection("second")),
            "collection" => self
                .props
                .target
                .get()
                .unwrap()
                .set(EntityTarget::new(Some("other".into()), "first")),
            "scope" => self.props.scope.get().unwrap().set(Some("scope-b".into())),
            _ => unreachable!(),
        }
        self.accessible.clear();
        self.text.clear();
        self.flush();
    }
}

#[test]
fn assigned_labels_missing_from_catalog_remain_visible_and_selected() {
    let label = Label::new("new", "New label");
    let state = EditorState::loaded(vec![], vec![label.clone()]);
    assert_eq!(state.catalog.labels.get("new"), Some(&label));
    assert!(state.selected.contains("new"));
    assert!(!state.dirty());
}

#[test]
fn editor_removes_legacy_group_assignments_incrementally() {
    let mut harness = Harness::new();
    let groups = vec![
        Label::new_group("first-group", "First"),
        Label::new_group("second-group", "Second"),
    ];
    // This also exercises a newer assignment snapshot than the catalog read.
    harness
        .props
        .client
        .take(".list")
        .respond
        .send(encode_labels(vec![]))
        .unwrap();
    harness
        .props
        .client
        .take(".load")
        .respond
        .send(encode_labels(groups.clone()))
        .unwrap();
    harness.flush();
    assert!(harness.accessible.contains_key("Remove First"));
    assert!(harness.accessible.contains_key("Remove Second"));
    harness.click("Remove First");
    harness.click("Save labels");
    let request = harness.props.client.take(".remove");
    let Value::Object(payload) = request.payload else {
        panic!("object payload");
    };
    assert_eq!(
        payload.get("label_ids"),
        Some(&Value::List(vec![Value::String("first-group".into())]))
    );
    request
        .respond
        .send(encode_labels(vec![groups[1].clone()]))
        .unwrap();
    harness.flush();
    assert_eq!(harness.props.changed.get(), 1);
    harness.click("Remove Second");
    harness.click("Save labels");
    let request = harness.props.client.take(".replace");
    let Value::Object(payload) = request.payload else {
        panic!("object payload");
    };
    assert_eq!(payload.get("label_ids"), Some(&Value::List(vec![])));
    request.respond.send(encode_labels(vec![])).unwrap();
    harness.flush();
    assert_eq!(harness.props.changed.get(), 2);
}

#[test]
fn public_editor_resets_target_collection_and_scope_during_pending_loads() {
    for change in ["target", "collection", "scope"] {
        let mut harness = Harness::new();
        let old_list = harness.props.client.take(".list");
        let old_load = harness.props.client.take(".load");
        harness.change_identity(change);
        // Unmounting the keyed session cancels its load, including both RPC waits.
        assert!(
            old_list
                .respond
                .send(encode_labels(vec![Label::new("old", "Old")]))
                .is_err()
        );
        assert!(
            old_load
                .respond
                .send(encode_labels(vec![Label::new("old", "Old")]))
                .is_err()
        );
        harness.complete_load("Current");
        assert!(!harness.text.iter().any(|text| text.contains("Old")));
        assert_eq!(harness.props.closed.get(), 0);
        assert_eq!(harness.props.changed.get(), 0);
    }
}

#[test]
fn public_editor_cancels_pending_saves_when_target_collection_or_scope_changes() {
    for change in ["target", "collection", "scope"] {
        let mut harness = Harness::new();
        harness.complete_load("Original");
        harness.click("Remove Original");
        harness.click("Close and save labels");
        let old_save = harness.props.client.take(".replace");
        let Value::Object(payload) = &old_save.payload else {
            panic!("object payload");
        };
        assert_eq!(payload.get("id").and_then(Value::as_str), Some("first"));
        assert_eq!(
            payload.get("scope_id").and_then(Value::as_str),
            Some("scope-a")
        );
        harness.change_identity(change);
        assert!(old_save.respond.send(encode_labels(vec![])).is_err());
        harness.complete_load("Current");
        assert_eq!(harness.props.closed.get(), 0);
        assert_eq!(harness.props.changed.get(), 0);
        harness.click("Remove Current");
        harness.click("Save labels");
        let new_save = harness.props.client.take(".replace");
        let Value::Object(payload) = &new_save.payload else {
            panic!("object payload");
        };
        assert_eq!(
            payload.get("id").and_then(Value::as_str),
            Some(if change == "target" {
                "second"
            } else {
                "first"
            })
        );
        assert_eq!(
            payload.get("collection").and_then(Value::as_str),
            Some(if change == "collection" {
                "other"
            } else {
                "entities"
            })
        );
        assert_eq!(
            payload.get("scope_id").and_then(Value::as_str),
            Some(if change == "scope" {
                "scope-b"
            } else {
                "scope-a"
            })
        );
        new_save.respond.send(encode_labels(vec![])).unwrap();
        harness.flush();
        assert_eq!(harness.props.closed.get(), 0);
        assert_eq!(harness.props.changed.get(), 1);
    }
}
