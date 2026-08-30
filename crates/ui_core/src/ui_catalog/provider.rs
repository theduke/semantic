use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::CatalogStorageSnapshot;
use semantic_rpc::RpcClient;

use crate::context::{use_active_scope_id, use_rpc_client};
use crate::ui_catalog::{RenderSettings, UiCatalog, UiCatalogError};

#[derive(Clone, PartialEq)]
struct CatalogSourceKey {
    client: RpcClient,
    scope_id: Option<String>,
}

#[derive(Clone, PartialEq)]
struct CatalogRequestKey {
    source: CatalogSourceKey,
    reload_revision: u64,
    render_settings: Option<RenderSettings>,
    configure_catalog: Option<Callback<UiCatalog, UiCatalog>>,
}

struct CatalogLoadResult {
    request: CatalogRequestKey,
    result: std::result::Result<UiCatalog, UiCatalogError>,
}

#[derive(Clone, Copy)]
pub struct UiCatalogContext {
    catalog: Signal<Option<UiCatalog>>,
}

impl UiCatalogContext {
    pub fn new(catalog: Signal<Option<UiCatalog>>) -> Self {
        Self { catalog }
    }

    pub fn catalog_signal(&self) -> Signal<Option<UiCatalog>> {
        self.catalog
    }
}

#[derive(Clone)]
pub struct UiCatalogReload {
    pub reload: Callback<()>,
    pub status: Signal<CatalogLoadStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogLoadStatus {
    Loading,
    Ready,
    Failed(String),
}

pub async fn load_catalog(
    client: RpcClient,
    scope_id: Option<String>,
    render_settings: Option<RenderSettings>,
    configure_catalog: Option<Callback<UiCatalog, UiCatalog>>,
) -> std::result::Result<UiCatalog, UiCatalogError> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    let response = client
        .invoke_value("semantic.db.catalog", Value::Object(payload))
        .await?;
    let Value::Object(object) = response else {
        return Err(UiCatalogError::InvalidResponse(
            "catalog response must be an object".to_string(),
        ));
    };
    match object.get("format") {
        Some(Value::String(format)) if format == "facet-json" => {}
        Some(_) => {
            return Err(UiCatalogError::InvalidResponse(
                "catalog response has unsupported format".to_string(),
            ));
        }
        None => {
            return Err(UiCatalogError::InvalidResponse(
                "catalog response missing format".to_string(),
            ));
        }
    }
    let Some(Value::String(catalog)) = object.get("catalog") else {
        return Err(UiCatalogError::InvalidResponse(
            "catalog response missing catalog string".to_string(),
        ));
    };
    let snapshot = facet_json::from_str::<CatalogStorageSnapshot>(catalog)
        .map_err(|err| UiCatalogError::Decode(err.to_string()))?;
    let mut catalog = UiCatalog::from_snapshot(snapshot);
    if let Some(render_settings) = render_settings {
        *catalog.render_settings_mut() = render_settings;
    }
    if let Some(configure_catalog) = configure_catalog {
        catalog = configure_catalog.call(catalog);
    }
    Ok(catalog)
}

pub fn use_ui_catalog_context() -> UiCatalogContext {
    use_context::<UiCatalogContext>()
}

pub fn use_ui_catalog() -> UiCatalog {
    use_ui_catalog_context()
        .catalog
        .read()
        .clone()
        .expect("UI catalog context is not loaded")
}

pub fn use_ui_catalog_reload() -> UiCatalogReload {
    use_context::<UiCatalogReload>()
}

#[component]
pub fn UiCatalogProvider(
    render_settings: Option<RenderSettings>,
    configure_catalog: Option<Callback<UiCatalog, UiCatalog>>,
    children: Element,
) -> Element {
    let mut catalog_signal = use_signal(|| None::<UiCatalog>);
    let mut loaded_source = use_signal(|| None::<CatalogSourceKey>);
    let mut observed_source = use_signal(|| None::<CatalogSourceKey>);
    let mut applied_request = use_signal(|| None::<CatalogRequestKey>);
    let mut status = use_signal(|| CatalogLoadStatus::Loading);
    let mut reload_revision = use_signal(|| 0_u64);
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let source = CatalogSourceKey {
        client: client.clone(),
        scope_id: scope_id.clone(),
    };
    let request = CatalogRequestKey {
        source: source.clone(),
        reload_revision: *reload_revision.read(),
        render_settings,
        configure_catalog,
    };
    let resource = use_resource(use_reactive((&request,), move |(request,)| {
        let request_for_result = request.clone();
        async move {
            let result = load_catalog(
                request.source.client,
                request.source.scope_id,
                request.render_settings,
                request.configure_catalog,
            )
            .await;
            CatalogLoadResult {
                request: request_for_result,
                result,
            }
        }
    }));
    let reload = Callback::new(move |_| {
        let revision = *reload_revision.peek();
        reload_revision.set(revision.wrapping_add(1));
    });
    use_context_provider(|| UiCatalogContext::new(catalog_signal));
    use_context_provider(|| UiCatalogReload { reload, status });

    use_effect(use_reactive((&source,), move |(source,)| {
        if observed_source.peek().as_ref() != Some(&source) {
            observed_source.set(Some(source.clone()));
            if loaded_source.peek().as_ref() != Some(&source) {
                catalog_signal.set(None);
                loaded_source.set(None);
            }
            status.set(CatalogLoadStatus::Loading);
        }
    }));

    let resource_state = resource.state();
    let request_for_completion = request.clone();
    use_effect(move || match *resource_state.read() {
        UseResourceState::Pending | UseResourceState::Paused | UseResourceState::Stopped => {
            if applied_request.peek().as_ref() != Some(&request_for_completion)
                && !matches!(*status.peek(), CatalogLoadStatus::Loading)
            {
                status.set(CatalogLoadStatus::Loading);
            }
        }
        UseResourceState::Ready => {
            let value = resource.read();
            let Some(completion) = value.as_ref() else {
                status.set(CatalogLoadStatus::Loading);
                return;
            };
            if completion.request != request_for_completion
                || applied_request.peek().as_ref() == Some(&completion.request)
            {
                return;
            }
            applied_request.set(Some(completion.request.clone()));
            match &completion.result {
                Ok(catalog) => {
                    catalog_signal.set(Some(catalog.clone()));
                    loaded_source.set(Some(completion.request.source.clone()));
                    status.set(CatalogLoadStatus::Ready);
                }
                Err(err) => {
                    status.set(CatalogLoadStatus::Failed(err.to_string()));
                }
            }
        }
    });

    let catalog_is_current =
        catalog_signal.read().is_some() && loaded_source.read().as_ref() == Some(&source);
    if catalog_is_current {
        rsx! { {children} }
    } else {
        match &*status.read() {
            CatalogLoadStatus::Loading => rsx! {
                div { class: "semantic-loading", "Loading catalog..." }
            },
            CatalogLoadStatus::Ready => rsx! {
                div { class: "semantic-loading", "Preparing catalog..." }
            },
            CatalogLoadStatus::Failed(message) => rsx! {
                div { class: "semantic-error",
                    p { "Catalog load failed: {message}" }
                    button { onclick: move |_| reload.call(()), "Retry" }
                }
            },
        }
    }
}
