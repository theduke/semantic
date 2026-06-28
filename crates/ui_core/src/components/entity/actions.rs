use dioxus::prelude::*;
use semantic_data::builtin::DEFAULT_COLLECTION;
use semantic_data::value::{Object, Value};

use crate::ui_catalog::EntityTarget;
use crate::ui_catalog::use_ui_catalog;
use crate::{use_active_scope_id, use_rpc_client};

#[component]
pub fn EntityOpenButton(target: EntityTarget) -> Element {
    let catalog = use_ui_catalog();
    let open = catalog.entity_navigation().open.clone();
    rsx! {
        if let Some(open) = open {
            dxcomp::Button {
                variant: dxcomp::ButtonVariant::Outline,
                size: dxcomp::ButtonSize::Sm,
                onclick: move |_| open(target.clone()),
                "Open"
            }
        }
    }
}

#[component]
pub fn EntityDeleteButton(target: EntityTarget) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut error = use_signal(|| None::<String>);

    rsx! {
        dxcomp::Button {
            variant: dxcomp::ButtonVariant::Outline,
            size: dxcomp::ButtonSize::Sm,
            onclick: move |_| {
                if !confirm_delete() {
                    return;
                }
                let client = client.clone();
                let scope_id = scope_id.clone();
                let target = target.clone();
                spawn(async move {
                    match delete_entity(client, scope_id, target).await {
                        Ok(()) => reload_window(),
                        Err(err) => error.set(Some(err)),
                    }
                });
            },
            "Delete"
        }
        if let Some(error) = error.read().as_ref() {
            span { class: "semantic-error", "{error}" }
        }
    }
}

async fn delete_entity(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    target: EntityTarget,
) -> std::result::Result<(), String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    if let Some(collection) = target.collection {
        if collection != DEFAULT_COLLECTION {
            payload.insert("collection", Value::String(collection));
        }
    }
    payload.insert("id", Value::String(target.id));
    client
        .invoke_value("semantic.db.delete", Value::Object(payload))
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[cfg(target_arch = "wasm32")]
fn confirm_delete() -> bool {
    web_sys::window()
        .and_then(|window| window.confirm_with_message("Delete this entity?").ok())
        .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn confirm_delete() -> bool {
    true
}

#[cfg(target_arch = "wasm32")]
fn reload_window() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn reload_window() {}
