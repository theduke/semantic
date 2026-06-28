use dioxus::prelude::*;
use semantic_data::builtin::DEFAULT_COLLECTION;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{use_active_scope_id, use_rpc_client};

use crate::components::value_string;

#[component]
pub fn QueryPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut query = use_signal(|| format!("select * from {DEFAULT_COLLECTION} limit 50"));
    let mut result = use_signal(|| None::<std::result::Result<Value, String>>);

    rsx! {
        section { class: "semantic-query",
            h2 { "Query" }
            dxcomp::Textarea {
                value: "{query}",
                oninput: move |event: FormEvent| query.set(event.value())
            }
            div {
                dxcomp::Button {
                    onclick: move |_| {
                        let client = client.clone();
                        let scope_id = scope_id.clone();
                        let query_text = query.read().clone();
                        spawn(async move {
                            result.set(Some(run_query(client, scope_id, query_text).await));
                        });
                    },
                    "Run"
                }
            }
            if let Some(result) = result.read().as_ref() {
                match result {
                    Ok(value) => rsx! { QueryResultView { value: value.clone() } },
                    Err(err) => rsx! { div { class: "semantic-error", "{err}" } },
                }
            }
        }
    }
}

#[component]
fn QueryResultView(value: Value) -> Element {
    match value {
        Value::Object(object) => {
            let kind = object
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            if let Some(Value::List(rows)) = object.get("rows") {
                let rows: Vec<Object> = rows
                    .iter()
                    .filter_map(|row| match row {
                        Value::Object(row) => Some(row.clone()),
                        _ => None,
                    })
                    .collect();
                rsx! {
                    div { class: "semantic-query-result",
                        h3 { "{kind}" }
                        table {
                            tbody {
                                for row in rows {
                                    {
                                        let summary = row.iter()
                                            .map(|(key, value)| format!("{key}: {}", value_string(value)))
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        rsx! {
                                    tr {
                                                td { "{summary}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                rsx! {
                    pre { "{object:?}" }
                }
            }
        }
        other => rsx! {
            pre { "{other:?}" }
        },
    }
}

async fn run_query(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    query: String,
) -> std::result::Result<Value, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert("query", Value::String(query));
    payload.insert("format", Value::String("sql".to_string()));
    client
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())
}
