use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::{Object, Value, value::FromValue, vdb::DatabaseSchema};
use semantic_ui_core::{
    UiCatalogContext,
    components::{InlineNotice, NoticeVariant},
    use_active_scope_id, use_rpc_client, use_ui_catalog_context,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VirtualCollection {
    pub name: String,
    pub generation: u64,
    pub available: bool,
    pub reason: Option<String>,
    pub revision: Option<String>,
}

#[derive(Clone, PartialEq)]
struct SourceKey {
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
}

#[derive(Clone, Copy)]
struct VirtualCollectionsContext(Resource<(SourceKey, Result<Vec<VirtualCollection>, String>)>);

#[component]
pub(crate) fn VirtualCollectionsProvider(children: Element) -> Element {
    let source = SourceKey {
        client: use_rpc_client(),
        scope_id: use_active_scope_id(),
    };
    let resource = use_resource(use_reactive((&source,), |(source,)| async move {
        let result = load_virtual_collections(source.client.clone(), source.scope_id.clone()).await;
        (source, result)
    }));
    use_context_provider(|| VirtualCollectionsContext(resource));
    children
}

pub(crate) fn use_virtual_collections() -> Vec<VirtualCollection> {
    use_virtual_collection_state()
        .and_then(Result::ok)
        .unwrap_or_default()
}

pub(crate) fn use_virtual_collection_reload() -> Callback<()> {
    let VirtualCollectionsContext(mut resource) = use_context();
    use_callback(move |_| resource.restart())
}

fn use_virtual_collection_state() -> Option<Result<Vec<VirtualCollection>, String>> {
    let VirtualCollectionsContext(resource) = use_context();
    let source = SourceKey {
        client: use_rpc_client(),
        scope_id: use_active_scope_id(),
    };
    resource
        .read()
        .as_ref()
        .filter(|(loaded, _)| loaded == &source)
        .map(|(_, result)| result.clone())
}

pub(crate) fn use_collection_names() -> Rc<[String]> {
    let catalog = use_ui_catalog_context().catalog_signal();
    let mut names = catalog
        .read()
        .as_ref()
        .map(|catalog| {
            catalog
                .collections()
                .map(|collection| collection.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    names.extend(
        use_virtual_collections()
            .into_iter()
            .map(|entry| entry.name),
    );
    names.sort();
    names.dedup();
    names.into()
}

async fn load_virtual_collections(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
) -> Result<Vec<VirtualCollection>, String> {
    let response = client
        .invoke_value("semantic.vdb.list", Value::Object(scoped_payload(scope_id)))
        .await
        .map_err(|error| error.to_string())?;
    let Value::List(entries) = response else {
        return Err("virtual collection list must be an array".into());
    };
    entries
        .into_iter()
        .map(|entry| {
            let Value::Object(entry) = entry else {
                return Err("virtual collection must be an object".into());
            };
            Ok(VirtualCollection {
                name: entry
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("virtual collection name missing")?
                    .to_owned(),
                generation: u64::from_value(
                    entry
                        .get("generation")
                        .cloned()
                        .ok_or("virtual collection generation missing")?,
                )
                .map_err(|error| error.to_string())?,
                available: matches!(entry.get("available"), Some(Value::Bool(true))),
                reason: entry
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                revision: entry
                    .get("schema_revision")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn scoped_payload(scope_id: Option<String>) -> Object {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", scope_id);
    }
    payload
}

#[component]
pub(crate) fn VirtualEntityCatalog(collection: Option<String>, children: Element) -> Element {
    let source = SourceKey {
        client: use_rpc_client(),
        scope_id: use_active_scope_id(),
    };
    let parent = use_ui_catalog_context().catalog_signal();
    let list = use_virtual_collection_state();
    let entries = list
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let entry = collection
        .as_ref()
        .and_then(|name| entries.iter().find(|entry| &entry.name == name))
        .cloned();
    let key = (source, entry);
    let mut schema = use_resource(use_reactive((&key,), |(key,)| async move {
        let result = match &key.1 {
            Some(entry) if entry.available => {
                let mut payload = scoped_payload(key.0.scope_id.clone());
                payload.insert("name", entry.name.clone());
                key.0
                    .client
                    .invoke_value("semantic.vdb.schema", Value::Object(payload))
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|value| {
                        DatabaseSchema::from_value(value)
                            .map(Some)
                            .map_err(|error| error.to_string())
                    })
            }
            Some(entry) => Err(entry
                .reason
                .clone()
                .unwrap_or_else(|| "Virtual collection unavailable".into())),
            None => Ok(None),
        };
        (key, result)
    }));
    let mut rendering = use_signal(|| parent.peek().clone());
    use_context_provider(|| UiCatalogContext::new(rendering));
    let local = parent.read().clone();
    let completion = schema.read();
    let result = completion
        .as_ref()
        .filter(|(loaded, _)| loaded == &key)
        .map(|(_, result)| result.clone());
    let is_local = collection.as_ref().is_none_or(|name| {
        local
            .as_ref()
            .is_some_and(|catalog| catalog.collection_by_name(name).is_some())
    });
    let combined = match (&local, result) {
        (Some(local), _) if is_local => Ok(Some(local.clone())),
        (Some(local), Some(Ok(Some(schema)))) => local
            .with_virtual_schema(collection.as_deref().unwrap_or_default(), &schema)
            .map(Some),
        (Some(local), Some(Ok(None))) => Ok(Some(local.clone())),
        (_, Some(Err(error))) => Err(error),
        _ => match &list {
            Some(Err(error)) => Err(error.clone()),
            _ => Ok(None),
        },
    };
    let effect_catalog = combined.as_ref().ok().cloned().flatten();
    use_effect(use_reactive(
        (&effect_catalog
            .as_ref()
            .map(|catalog| catalog.snapshot().clone()),),
        move |_| {
            rendering.set(effect_catalog.clone());
        },
    ));
    match combined {
        Err(error) => rsx! { InlineNotice { title: "Could not load virtual schema", message: error,
        variant: NoticeVariant::Error, action_label: "Retry", on_action: move |_| schema.restart() } },
        Ok(Some(catalog))
            if rendering
                .read()
                .as_ref()
                .is_some_and(|rendering| rendering.snapshot() == catalog.snapshot()) =>
        {
            children
        }
        _ => rsx! { div { class: "semantic-loading", "Loading collection schema…" } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::oneshot;
    use semantic_rpc::{
        RpcClient,
        client::{RpcClientDyn, RpcClientFuture},
    };
    use semantic_rpc_core::RpcClientError;
    use semantic_ui_core::{UiScopeContext, provide_rpc_client};
    use std::{
        cell::{Cell, RefCell},
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
        ) -> RpcClientFuture<Result<Value, RpcClientError>> {
            assert_eq!(command, "semantic.vdb.list");
            let (response, receiver) = oneshot::channel();
            self.0.lock().unwrap().push(Request { payload, response });
            Box::pin(async move { Ok(receiver.await.unwrap()) })
        }
    }
    #[derive(Clone)]
    struct Props {
        client: Client,
        scope: Rc<Cell<Option<Signal<Option<String>>>>>,
        names: Rc<RefCell<Vec<String>>>,
    }
    fn app(props: Props) -> Element {
        let scope = use_signal(|| Some("first".into()));
        use_context_provider(|| UiScopeContext::new(scope));
        use_hook(|| props.scope.set(Some(scope)));
        let rpc = use_hook(|| RpcClient::new(props.client));
        provide_rpc_client(rpc);
        rsx! { VirtualCollectionsProvider { Probe { names: props.names } } }
    }
    #[component]
    fn Probe(names: Rc<RefCell<Vec<String>>>) -> Element {
        *names.borrow_mut() = use_virtual_collections()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        rsx! { div {} }
    }
    fn flush(dom: &mut VirtualDom) {
        for _ in 0..10 {
            dom.process_events();
            dom.render_immediate_to_vec();
        }
    }
    fn listing(name: &str) -> Value {
        Value::List(vec![Value::Object(Object::from_iter([
            ("name".into(), Value::String(name.into())),
            ("generation".into(), Value::U64(1)),
            ("available".into(), Value::Bool(true)),
            ("schema_revision".into(), Value::String("1".into())),
        ]))])
    }
    #[test]
    fn scope_switch_discards_old_virtual_navigation() {
        let client = Client::default();
        let scope = Rc::new(Cell::new(None));
        let names = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(
            app,
            Props {
                client: client.clone(),
                scope: scope.clone(),
                names: names.clone(),
            },
        );
        dom.rebuild_to_vec();
        flush(&mut dom);
        let first = client.0.lock().unwrap().remove(0);
        first.response.send(listing("first.fx")).unwrap();
        flush(&mut dom);
        assert_eq!(&*names.borrow(), &["first.fx"]);
        scope.get().unwrap().set(Some("second".into()));
        flush(&mut dom);
        assert!(names.borrow().is_empty());
        let second = client.0.lock().unwrap().remove(0);
        let Value::Object(payload) = second.payload else {
            panic!("scope payload")
        };
        assert_eq!(
            payload.get("scope_id").and_then(Value::as_str),
            Some("second")
        );
        second.response.send(listing("second.fx")).unwrap();
        flush(&mut dom);
        assert_eq!(&*names.borrow(), &["second.fx"]);
    }
}
