use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{
    ClassView, ObjectView, RenderMode, use_active_scope_id, use_rpc_client, use_ui_catalog,
};

#[component]
pub fn EntityScreen(collection: String, id: String) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let collection_for_load = collection.clone();
    let id_for_load = id.clone();
    let resource = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let collection = collection_for_load.clone();
        let id = id_for_load.clone();
        async move { load_entity(client, scope_id, collection, id).await }
    });

    rsx! {
        section { class: "semantic-entity",
            h2 { "{collection}/{id}" }
            match &*resource.read_unchecked() {
                Some(Ok(Some(object))) => {
                    let class = catalog.object_class(object).cloned();
                    rsx! {
                        if let Some(class) = class {
                            ClassView {
                                class,
                                object: object.clone(),
                                collection: Some(collection.clone()),
                                id: Some(id.clone()),
                                mode: RenderMode::Detail
                            }
                        } else {
                            ObjectView {
                                object: object.clone(),
                                mode: RenderMode::Detail
                            }
                        }
                    }
                },
                Some(Ok(None)) => rsx! {
                    div { class: "semantic-empty", "Entity not found" }
                },
                Some(Err(err)) => rsx! {
                    div { class: "semantic-error", "{err}" }
                },
                None => rsx! {
                    div { class: "semantic-loading", "Loading entity..." }
                },
            }
        }
    }
}

async fn load_entity(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: String,
    id: String,
) -> std::result::Result<Option<Object>, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert("collection", Value::String(collection));
    payload.insert("id", Value::String(id));
    let response = client
        .invoke_value("semantic.db.get", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())?;
    match response {
        Value::Null | Value::Void => Ok(None),
        Value::Object(mut response) => match response.remove("object") {
            Some(Value::Object(object)) => Ok(Some(object)),
            _ => Err("get response missing object".to_string()),
        },
        _ => Err("get response must be an object or null".to_string()),
    }
}
