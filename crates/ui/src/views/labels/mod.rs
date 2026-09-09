mod form;
mod tree;

use crate::components::PageHeader;
use dioxus::prelude::*;
use form::LabelDetails;
use semantic_base::labels::{Label, LabelCatalog, LabelKind};
use semantic_ui_core::components::labels::client;
use semantic_ui_core::{use_active_scope_id, use_rpc_client};
use tree::LabelTree;

#[component]
pub fn LabelsPage() -> Element {
    let scope = use_active_scope_id();
    rsx! {
        for scope in [scope] {
            LabelManager { key: "{scope:?}", scope }
        }
    }
}

#[component]
fn LabelManager(scope: Option<String>) -> Element {
    let rpc = use_rpc_client();
    let mut catalog = use_signal(LabelCatalog::default);
    let mut loaded = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut selected = use_signal(|| None::<Label>);
    let mut dirty = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut pending_selection = use_signal(|| None::<Label>);
    let mut query = use_signal(String::new);
    let mut revision = use_signal(|| 0u64);
    let mut reload = use_signal(|| 0u64);
    use_resource(move || {
        let rpc = rpc.clone();
        let scope = scope.clone();
        let _ = reload();
        async move {
            match client::list(&rpc, scope).await {
                Ok(labels) => {
                    catalog.set(LabelCatalog::new(labels));
                    loaded.set(true);
                    error.set(None);
                }
                Err(message) => error.set(Some(message)),
            }
        }
    });
    let choose = use_callback(move |label: Label| {
        if busy() {
            return;
        }
        if dirty() {
            pending_selection.set(Some(label));
        } else {
            selected.set(Some(label));
            revision += 1;
        }
    });
    let normalized_query = query().trim().to_lowercase();
    let group_count = catalog
        .read()
        .labels
        .values()
        .filter(|label| label.is_group())
        .count();
    let label_count = catalog.read().labels.len() - group_count;
    let matches = catalog
        .read()
        .labels
        .keys()
        .filter(|id| {
            catalog
                .read()
                .path(id)
                .to_lowercase()
                .contains(&normalized_query)
        })
        .count();
    rsx! {
        section { class: "semantic-label-manager",
            PageHeader { title: "Labels", description: "Give your entities a shared vocabulary. Nest labels into groups and choose how their children can be selected.",
                actions: rsx! {
                    dxcomp::Button { disabled: !loaded() || busy(), onclick: move |_| choose.call(new_label(None)), "+ New label" }
                    dxcomp::Button { variant: dxcomp::ButtonVariant::Outline, disabled: !loaded() || busy(),
                        onclick: move |_| { let mut group = new_label(None); group.kind = LabelKind::Group; choose.call(group); }, "+ New group" }
                }
            }
            if let Some(message) = error() {
                p { class: "semantic-error", role: "alert", "{message}" }
                dxcomp::Button { variant: dxcomp::ButtonVariant::Outline, onclick: move |_| reload += 1, "Try again" }
            }
            if !loaded() && error().is_none() { p { role: "status", "Loading labels…" } }
            if loaded() {
                div { class: "semantic-label-manager__layout",
                    aside { class: "semantic-label-manager__hierarchy",
                        div { class: "semantic-label-manager__toolbar",
                            h2 { "Hierarchy" }
                            span { class: "semantic-label-muted", "{label_count} labels · {group_count} groups" }
                        }
                        label { class: "semantic-label-field",
                            span { class: "semantic-label-muted", "Search labels" }
                            input { r#type: "search", placeholder: "Filter labels and groups…", value: query(), oninput: move |event| query.set(event.value()) }
                        }
                        if catalog.read().labels.is_empty() {
                            div { class: "semantic-label-empty",
                                h3 { "Start with a label" }
                                p { "Try a topic, a project, or a status. Add child labels to organize related choices." }
                                dxcomp::Button { variant: dxcomp::ButtonVariant::Outline,
                                    onclick: move |_| choose.call(new_label(None)), "Create your first label" }
                            }
                        } else if matches == 0 {
                            p { class: "semantic-label-empty", "No labels match your search." }
                        } else {
                            LabelTree { catalog: std::rc::Rc::new(catalog()), selected: selected().map(|label| label.id), query: normalized_query,
                                on_select: move |id: String| {
                                    let label = catalog.peek().labels.get(&id).cloned();
                                    if let Some(label) = label { choose.call(label); }
                                }
                            }
                        }
                    }
                    div { class: "semantic-label-manager__details",
                        if let Some(label) = selected() {
                            LabelDetails { key: "{label.id}:{revision}", label, catalog: catalog(),
                                on_dirty: move |value| dirty.set(value), on_busy: move |value| busy.set(value),
                                on_saved: move |label: Label| {
                                    catalog.write().labels.insert(label.id.clone(), label.clone());
                                    selected.set(Some(label)); dirty.set(false);
                                },
                                on_deleted: move |id: String| { catalog.write().labels.remove(&id); selected.set(None); dirty.set(false); },
                                on_child: move |id| choose.call(new_label(Some(id))),
                            }
                        } else {
                            div { class: "semantic-label-empty semantic-label-empty--details",
                                h2 { "A place for every label" }
                                p { "Select a label to edit its name, color, description, and place in the hierarchy." }
                                p { "Exclusive groups allow one child label per entity—useful for statuses such as Planned, In progress, and Done." }
                            }
                        }
                    }
                }
            }
        }
        dxcomp::AlertDialog { open: pending_selection().is_some(),
            on_open_change: move |open: bool| if !open { pending_selection.set(None); },
            dxcomp::AlertDialogTitle { "Discard unsaved label changes?" }
            dxcomp::AlertDialogDescription { "Your current label draft has not been saved." }
            dxcomp::AlertDialogActions {
                dxcomp::Button { variant: dxcomp::ButtonVariant::Outline, onclick: move |_| pending_selection.set(None), "Keep editing" }
                dxcomp::Button { variant: dxcomp::ButtonVariant::Destructive,
                    onclick: move |_| { selected.set(pending_selection()); pending_selection.set(None); dirty.set(false); revision += 1; }, "Discard changes" }
            }
        }
    }
}

fn new_label(parent_id: Option<String>) -> Label {
    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut label = Label::new(
        format!(
            "label-{}-{sequence}",
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ),
        "",
    );
    label.parent_id = parent_id;
    label
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use futures::channel::oneshot;
    use semantic_data::value::Value;
    use semantic_rpc::{RpcClient, RpcClientDyn};
    use semantic_rpc_core::RpcClientError;
    use semantic_ui_core::{UiScopeContext, provide_rpc_client};
    use std::{
        cell::Cell,
        rc::Rc,
        sync::{Arc, Mutex},
    };

    struct Request {
        payload: Value,
        response: oneshot::Sender<Value>,
    }

    #[derive(Clone, Default)]
    struct Client(Arc<Mutex<Vec<Request>>>);

    impl RpcClientDyn for Client {
        fn invoke_value(
            &self,
            command: String,
            payload: Value,
        ) -> futures::future::BoxFuture<'static, Result<Value, RpcClientError>> {
            assert_eq!(command, "semantic.base.labels.list");
            let (response, receiver) = oneshot::channel();
            self.0.lock().unwrap().push(Request { payload, response });
            Box::pin(async move { Ok(receiver.await.expect("test response")) })
        }
    }

    #[derive(Clone)]
    struct Props {
        client: Client,
        scope: Rc<Cell<Option<Signal<Option<String>>>>>,
    }

    fn app(props: Props) -> Element {
        let scope = use_signal(|| Some("first".to_string()));
        use_context_provider(|| UiScopeContext::new(scope));
        use_hook(|| props.scope.set(Some(scope)));
        let rpc = use_hook(|| RpcClient::new(props.client));
        provide_rpc_client(rpc);
        rsx! { LabelsPage {} }
    }

    fn flush(dom: &mut VirtualDom) {
        for _ in 0..10 {
            dom.process_events();
            dom.render_immediate_to_vec();
        }
    }

    #[test]
    fn labels_page_remounts_manager_when_scope_changes() {
        let client = Client::default();
        let scope = Rc::new(Cell::new(None));
        let mut dom = VirtualDom::new_with_props(
            app,
            Props {
                client: client.clone(),
                scope: scope.clone(),
            },
        );
        dom.rebuild_to_vec();
        flush(&mut dom);
        let first = client.0.lock().unwrap().remove(0);
        scope.get().unwrap().set(Some("second".into()));
        flush(&mut dom);
        // The old manager's pending load is gone, and the new one requests the
        // selected scope instead of keeping the first catalog/draft alive.
        assert!(first.response.send(Value::List(vec![])).is_err());
        let second = client.0.lock().unwrap().remove(0);
        let Value::Object(payload) = second.payload else {
            panic!("scope payload");
        };
        assert_eq!(
            payload.get("scope_id").and_then(Value::as_str),
            Some("second")
        );
        second.response.send(Value::List(vec![])).unwrap();
        flush(&mut dom);
    }
}
