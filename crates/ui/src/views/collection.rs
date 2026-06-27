use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{use_active_scope_id, use_rpc_client};

use crate::{components::value_string, views::Route};

#[component]
pub fn CollectionPage(collection: String) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let collection_for_query = collection.clone();
    let resource = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let collection = collection_for_query.clone();
        async move { query_collection(client, scope_id, collection).await }
    });

    rsx! {
        section { class: "semantic-collection",
            h2 { "{collection}" }
            div { class: "semantic-collection__actions",
                dxcomp::Button {
                    onclick: move |_| {
                        navigator().push(Route::CreateEntityPage);
                    },
                    "Create"
                }
            }
            match &*resource.read_unchecked() {
                Some(Ok(rows)) => rsx! {
                    table {
                        thead {
                            tr {
                                th { "id" }
                                th { "type" }
                                th { "fields" }
                                th { "actions" }
                            }
                        }
                        tbody {
                            for row in rows.iter().cloned() {
                                tr {
                                    td {
                                        if let Some(id) = row.get("id").and_then(Value::as_str).map(str::to_string) {
                                            Link {
                                                to: Route::EntityPage {
                                                    collection: collection.clone(),
                                                    id: id.clone()
                                                },
                                                class: "dx-button",
                                                "data-style": "link",
                                                "data-size": "default",
                                                "{id}"
                                            }
                                        }
                                    }
                                    td { "{row.get(\"type\").map(value_string).unwrap_or_default()}" }
                                    td { "{row.len()}" }
                                    td {
                                        if let Some(id) = row.get("id").and_then(Value::as_str).map(str::to_string) {
                                            dxcomp::Button {
                                                variant: dxcomp::ButtonVariant::Outline,
                                                size: dxcomp::ButtonSize::Sm,
                                                onclick: {
                                                    let collection = collection.clone();
                                                    let id = id.clone();
                                                    move |_| {
                                                        navigator().push(Route::EditEntityPage {
                                                            collection: collection.clone(),
                                                            id: id.clone(),
                                                        });
                                                    }
                                                },
                                                "Edit"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                Some(Err(err)) => rsx! {
                    div { class: "semantic-error", "{err}" }
                },
                None => rsx! {
                    div { class: "semantic-loading", "Loading collection..." }
                },
            }
        }
    }
}

async fn query_collection(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: String,
) -> std::result::Result<Vec<Object>, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert(
        "query",
        Value::String(format!("select * from {collection} limit 100")),
    );
    payload.insert("format", Value::String("sql".to_string()));
    let response = client
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())?;
    let Value::Object(object) = response else {
        return Err("query response must be an object".to_string());
    };
    let Some(Value::List(rows)) = object.get("rows") else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .filter_map(|row| match row {
            Value::Object(row) => Some(row.clone()),
            _ => None,
        })
        .collect())
}
