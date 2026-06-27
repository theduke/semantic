use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::CatalogStorageSnapshot;
use semantic_rpc::RpcClient;

use crate::context::{use_active_scope_id, use_rpc_client};
use crate::ui_catalog::{RenderSettings, UiCatalog, UiCatalogError};

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
    let mut status = use_signal(|| CatalogLoadStatus::Loading);
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut resource = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let render_settings = render_settings.clone();
        async move { load_catalog(client, scope_id, render_settings, configure_catalog).await }
    });
    use_context_provider(|| UiCatalogContext::new(catalog_signal));
    use_context_provider(|| UiCatalogReload {
        reload: Callback::new(move |_| resource.restart()),
        status,
    });

    match &*resource.read_unchecked() {
        Some(Ok(catalog)) => {
            catalog_signal.set(Some(catalog.clone()));
            status.set(CatalogLoadStatus::Ready);
        }
        Some(Err(err)) => {
            status.set(CatalogLoadStatus::Failed(err.to_string()));
        }
        None => {
            status.set(CatalogLoadStatus::Loading);
        }
    }

    match &*catalog_signal.read() {
        Some(_) => rsx! { {children} },
        None => match &*status.read() {
            CatalogLoadStatus::Loading => rsx! {
                div { class: "semantic-loading", "Loading catalog..." }
            },
            CatalogLoadStatus::Ready => rsx! {
                div { class: "semantic-loading", "Preparing catalog..." }
            },
            CatalogLoadStatus::Failed(message) => rsx! {
                div { class: "semantic-error",
                    p { "Catalog load failed: {message}" }
                    button { onclick: move |_| resource.restart(), "Retry" }
                }
            },
        },
    }
}
