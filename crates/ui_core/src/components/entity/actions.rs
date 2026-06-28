use dioxus::prelude::*;
use dioxus_icons::lucide::{Eye, Trash2};
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
                size: dxcomp::ButtonSize::IconSm,
                title: "Open",
                aria_label: "Open entity",
                onclick: move |_| open(target.clone()),
                Eye { size: "1rem" }
            }
        }
    }
}

#[component]
pub fn EntityDeleteButton(
    target: EntityTarget,
    #[props(default)] on_deleted: Option<EventHandler<EntityTarget>>,
) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut open = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    rsx! {
        dxcomp::Button {
            variant: dxcomp::ButtonVariant::Outline,
            size: dxcomp::ButtonSize::IconSm,
            title: "Delete",
            aria_label: "Delete entity",
            onclick: move |_| open.set(true),
            Trash2 { size: "1rem" }
        }
        dxcomp::AlertDialog {
            open: open(),
            on_open_change: move |next_open: bool| open.set(next_open),
            dxcomp::AlertDialogTitle { "Delete Entity" }
            dxcomp::AlertDialogDescription {
                "This permanently deletes the entity from the current scope."
            }
            dxcomp::AlertDialogActions {
                dxcomp::AlertDialogCancel {
                    on_click: move |_| open.set(false),
                    "Cancel"
                }
                dxcomp::AlertDialogAction {
                    on_click: move |_| {
                        let client = client.clone();
                        let scope_id = scope_id.clone();
                        let target = target.clone();
                        let on_deleted = on_deleted.clone();
                        spawn(async move {
                            match delete_entity(client, scope_id, target.clone()).await {
                                Ok(()) => {
                                    if let Some(on_deleted) = on_deleted {
                                        on_deleted.call(target);
                                    } else {
                                        reload_window();
                                    }
                                }
                                Err(err) => error.set(Some(err)),
                            }
                        });
                    },
                    "Delete"
                }
            }
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
fn reload_window() {
    if let Some(window) = web_sys::window() {
        let _ = window.location().reload();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn reload_window() {}
