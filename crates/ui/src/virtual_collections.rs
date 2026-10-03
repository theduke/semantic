use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::{
    Object, Value,
    value::{FromValue, IntoValue},
    vdb::{DatabaseSchema, VdbInfo, VdbSchemaRequest},
};
use semantic_ui_core::{
    UiCatalog, UiCatalogContext,
    components::{InlineNotice, NoticeVariant},
    use_active_scope_id, use_rpc_client, use_ui_catalog_context,
};

pub(crate) type VirtualCollection = VdbInfo;

/// Memo completions compare by identity, without comparing full catalogs.
#[derive(Clone)]
struct CatalogResult(Rc<Result<Option<UiCatalog>, String>>);

impl PartialEq for CatalogResult {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, PartialEq)]
struct SourceKey {
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
}

#[derive(Clone, Copy)]
struct VirtualCollectionsContext {
    resource: Resource<(SourceKey, Result<Vec<VirtualCollection>, String>)>,
    original_catalog: Signal<Option<UiCatalog>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CollectionIdentity {
    Local,
    Virtual,
    Pending,
    Unknown,
}

fn collection_identity(
    original: Option<&UiCatalog>,
    list: Option<&Result<Vec<VirtualCollection>, String>>,
    name: &str,
) -> CollectionIdentity {
    if original.is_some_and(|catalog| catalog.collection_by_name(name).is_some()) {
        return CollectionIdentity::Local;
    }
    match list {
        Some(Ok(entries)) if entries.iter().any(|entry| entry.name == name) => {
            CollectionIdentity::Virtual
        }
        None => CollectionIdentity::Pending,
        _ => CollectionIdentity::Unknown,
    }
}

pub(crate) fn use_collection_read_only(name: &str) -> bool {
    let context = use_context::<VirtualCollectionsContext>();
    let list = use_virtual_collection_state();
    collection_identity(
        context.original_catalog.read().as_ref(),
        list.as_ref(),
        name,
    ) != CollectionIdentity::Local
}

#[component]
pub(crate) fn VirtualCollectionsProvider(children: Element) -> Element {
    let original_catalog = use_ui_catalog_context().catalog_signal();
    let source = SourceKey {
        client: use_rpc_client(),
        scope_id: use_active_scope_id(),
    };
    let resource = use_resource(use_reactive((&source,), |(source,)| async move {
        let result = load_virtual_collections(source.client.clone(), source.scope_id.clone()).await;
        (source, result)
    }));
    use_context_provider(|| VirtualCollectionsContext {
        resource,
        original_catalog,
    });
    children
}

pub(crate) fn use_virtual_collections() -> Vec<VirtualCollection> {
    use_virtual_collection_state()
        .and_then(Result::ok)
        .unwrap_or_default()
}

pub(crate) fn use_virtual_collection_reload() -> Callback<()> {
    let VirtualCollectionsContext { mut resource, .. } = use_context();
    use_callback(move |_| resource.restart())
}

fn use_virtual_collection_state() -> Option<Result<Vec<VirtualCollection>, String>> {
    let VirtualCollectionsContext {
        resource,
        original_catalog,
    } = use_context();
    let source = SourceKey {
        client: use_rpc_client(),
        scope_id: use_active_scope_id(),
    };
    resource
        .read()
        .as_ref()
        .filter(|(loaded, _)| loaded == &source)
        .map(|(_, result)| {
            result.clone().map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| {
                        original_catalog
                            .read()
                            .as_ref()
                            .is_none_or(|local| local.collection_by_name(&entry.name).is_none())
                    })
                    .collect()
            })
        })
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
    Vec::<VdbInfo>::from_value(response).map_err(|error| error.to_string())
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
    let original = use_context::<VirtualCollectionsContext>().original_catalog;
    let identity = collection_identity(
        original.read().as_ref(),
        list.as_ref(),
        collection
            .as_deref()
            .unwrap_or(semantic_data::builtin::DEFAULT_COLLECTION),
    );
    let entry = collection
        .as_ref()
        .and_then(|name| {
            list.as_ref()?
                .as_ref()
                .ok()?
                .iter()
                .find(|entry| &entry.name == name)
        })
        .cloned();
    let listing_error = list
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .cloned();
    let key = (source, entry);
    let mut schema = use_resource(use_reactive((&key,), |(key,)| async move {
        let result = match &key.1 {
            Some(entry) if entry.available => {
                let payload = VdbSchemaRequest {
                    scope_id: key.0.scope_id.clone(),
                    name: entry.name.clone(),
                }
                .into_value();
                key.0
                    .client
                    .invoke_value("semantic.vdb.schema", payload)
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
    let combined = use_memo(use_reactive(
        (&key, &identity, &listing_error, &collection),
        move |(key, identity, listing_error, collection)| {
            let local = parent.read().clone();
            let completion = schema.read();
            let result = completion
                .as_ref()
                .filter(|(loaded, _)| loaded == &key)
                .map(|(_, result)| result.clone());
            CatalogResult(Rc::new(match (&local, result) {
                (Some(local), _) if identity == CollectionIdentity::Local => {
                    Ok(Some(local.clone()))
                }
                _ if identity == CollectionIdentity::Pending => Ok(None),
                (Some(local), Some(Ok(Some(schema)))) => local
                    .with_virtual_schema(collection.as_deref().unwrap_or_default(), &schema)
                    .map(Some),
                (_, Some(Ok(None))) => Err("Unknown collection".into()),
                (_, Some(Err(error))) => Err(error),
                _ => match listing_error {
                    Some(error) => Err(error),
                    _ => Ok(None),
                },
            }))
        },
    ));
    let mut rendered = use_signal(|| None::<CatalogResult>);
    use_effect(move || {
        let completed = combined.read().clone();
        rendering.set(completed.0.as_ref().as_ref().ok().cloned().flatten());
        rendered.set(Some(completed));
    });
    let completed = combined.read();
    match completed.0.as_ref() {
        Err(error) => {
            rsx! { InlineNotice { title: "Could not load virtual schema", message: error.clone(),
            variant: NoticeVariant::Error, action_label: "Retry", on_action: move |_| schema.restart() } }
        }
        Ok(Some(_)) if rendered.read().as_ref() == Some(&completed) => children,
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
        command: String,
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
            assert!(matches!(
                command.as_str(),
                "semantic.vdb.list" | "semantic.vdb.schema"
            ));
            let (response, receiver) = oneshot::channel();
            self.0.lock().unwrap().push(Request {
                command,
                payload,
                response,
            });
            Box::pin(async move { Ok(receiver.await.unwrap()) })
        }
    }
    #[derive(Clone)]
    struct Props {
        client: Client,
        scope: Rc<Cell<Option<Signal<Option<String>>>>>,
        names: Rc<RefCell<Vec<String>>>,
        original: UiCatalog,
        read_only: Rc<Cell<(bool, bool)>>,
    }
    fn app(props: Props) -> Element {
        let catalog = use_signal(|| Some(props.original));
        use_context_provider(|| UiCatalogContext::new(catalog));
        let scope = use_signal(|| Some("first".into()));
        use_context_provider(|| UiScopeContext::new(scope));
        use_hook(|| props.scope.set(Some(scope)));
        let rpc = use_hook(|| RpcClient::new(props.client));
        provide_rpc_client(rpc);
        rsx! { VirtualCollectionsProvider { Probe { names: props.names, read_only: props.read_only } } }
    }
    #[component]
    fn Probe(names: Rc<RefCell<Vec<String>>>, read_only: Rc<Cell<(bool, bool)>>) -> Element {
        read_only.set((
            use_collection_read_only("fx"),
            use_collection_read_only("remote"),
        ));
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
        vec![VdbInfo {
            name: name.into(),
            generation: 1,
            available: true,
            schema_revision: Some("1".into()),
            ..Default::default()
        }]
        .into_value()
    }

    #[derive(Clone)]
    struct CatalogProps {
        client: Client,
        repaint: Rc<Cell<Option<Signal<u64>>>>,
        observed: Rc<RefCell<Vec<usize>>>,
    }

    fn catalog_app(props: CatalogProps) -> Element {
        let catalog = use_signal(|| {
            Some(UiCatalog::from_snapshot(
                semantic_db_core::catalog::Catalog::new().to_storage_snapshot(),
            ))
        });
        use_context_provider(|| UiCatalogContext::new(catalog));
        let scope = use_signal(|| Some("first".into()));
        use_context_provider(|| UiScopeContext::new(scope));
        let repaint = use_signal(|| 0_u64);
        use_hook(|| props.repaint.set(Some(repaint)));
        let rpc = use_hook(|| RpcClient::new(props.client));
        provide_rpc_client(rpc);
        rsx! {
            VirtualCollectionsProvider {
                VirtualEntityCatalog { collection: Some("remote".into()),
                    CatalogProbe { observed: props.observed, repaint: *repaint.read() }
                }
            }
        }
    }

    #[component]
    fn CatalogProbe(observed: Rc<RefCell<Vec<usize>>>, repaint: u64) -> Element {
        let signal = use_ui_catalog_context().catalog_signal();
        let catalog = signal.read();
        let catalog = catalog.as_ref().unwrap();
        assert!(
            catalog
                .collections()
                .any(|collection| collection.name == "remote")
        );
        observed
            .borrow_mut()
            .push(catalog.render_registry() as *const _ as usize);
        rsx! { div { "{repaint}" } }
    }

    #[test]
    fn unrelated_renders_reuse_the_virtual_rendering_catalog() {
        let client = Client::default();
        let repaint = Rc::new(Cell::new(None));
        let observed = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(
            catalog_app,
            CatalogProps {
                client: client.clone(),
                repaint: repaint.clone(),
                observed: observed.clone(),
            },
        );
        dom.rebuild_to_vec();
        flush(&mut dom);
        client
            .0
            .lock()
            .unwrap()
            .remove(0)
            .response
            .send(listing("remote"))
            .unwrap();
        flush(&mut dom);
        let schema = client.0.lock().unwrap().remove(0);
        assert_eq!(schema.command, "semantic.vdb.schema");
        assert_eq!(
            schema.payload.get_field("name").and_then(Value::as_str),
            Some("remote")
        );
        schema
            .response
            .send(DatabaseSchema::default().into_value())
            .unwrap();
        flush(&mut dom);
        assert!(!observed.borrow().is_empty());
        repaint.get().unwrap().set(1);
        flush(&mut dom);
        let observed = observed.borrow();
        assert!(observed.len() >= 2);
        assert!(observed.iter().all(|identity| identity == &observed[0]));
        assert!(client.0.lock().unwrap().is_empty());
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
                original: UiCatalog::from_snapshot(
                    semantic_db_core::catalog::Catalog::new().to_storage_snapshot(),
                ),
                read_only: Rc::new(Cell::new((true, true))),
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

    #[test]
    fn original_local_collection_wins_conflicts_and_pending_identity_is_read_only() {
        use semantic_db_core::catalog::{Catalog, CollectionKind, IntegrityMode};
        let mut local = Catalog::new();
        local
            .upsert_collection("fx", CollectionKind::Polymorphic, IntegrityMode::Permissive)
            .unwrap();
        let original = UiCatalog::from_snapshot(local.to_storage_snapshot());
        let unavailable = |name: &str| VirtualCollection {
            name: name.into(),
            generation: 1,
            available: false,
            reason: Some("unavailable".into()),
            ..Default::default()
        };
        let list = Ok(vec![unavailable("fx"), unavailable("remote")]);
        assert_eq!(
            collection_identity(Some(&original), Some(&list), "fx"),
            CollectionIdentity::Local
        );
        assert_eq!(
            collection_identity(Some(&original), None, "fx"),
            CollectionIdentity::Local
        );
        assert_eq!(
            collection_identity(Some(&original), Some(&list), "remote"),
            CollectionIdentity::Virtual
        );
        assert_eq!(
            collection_identity(Some(&original), None, "remote"),
            CollectionIdentity::Pending
        );
        // The rendering overlay has a virtual shell; it must never replace the
        // original catalog used by identity and mutation-control checks.
        let overlay = original
            .with_virtual_schema("remote", &DatabaseSchema::default())
            .unwrap();
        assert!(overlay.collection_by_name("remote").is_some());
        assert_eq!(
            collection_identity(Some(&original), Some(&list), "remote"),
            CollectionIdentity::Virtual
        );
    }

    #[test]
    fn local_conflict_keeps_controls_and_navigation_while_pending_remote_is_read_only() {
        use semantic_db_core::catalog::{Catalog, CollectionKind, IntegrityMode};
        let mut local = Catalog::new();
        local
            .upsert_collection("fx", CollectionKind::Polymorphic, IntegrityMode::Permissive)
            .unwrap();
        let client = Client::default();
        let names = Rc::new(RefCell::new(Vec::new()));
        let read_only = Rc::new(Cell::new((true, false)));
        let mut dom = VirtualDom::new_with_props(
            app,
            Props {
                client: client.clone(),
                scope: Rc::new(Cell::new(None)),
                names: names.clone(),
                original: UiCatalog::from_snapshot(local.to_storage_snapshot()),
                read_only: read_only.clone(),
            },
        );
        dom.rebuild_to_vec();
        flush(&mut dom);
        assert_eq!(read_only.get(), (false, true));
        let request = client.0.lock().unwrap().remove(0);
        let unavailable = |name: &str| {
            VdbInfo {
                name: name.into(),
                generation: 1,
                available: false,
                reason: Some("unavailable".into()),
                ..Default::default()
            }
            .into_value()
        };
        request
            .response
            .send(Value::List(vec![unavailable("fx"), unavailable("remote")]))
            .unwrap();
        flush(&mut dom);
        assert_eq!(&*names.borrow(), &["remote"]);
        assert_eq!(read_only.get(), (false, true));
    }
}
