//! Shared RPC boundary for label management and assignment components.
use crate::EntityTarget;
use semantic_base::labels::{Label, decode_labels};
use semantic_data::value::{Object, Value};
use semantic_rpc::RpcClient;

pub fn payload(scope_id: Option<String>) -> Object {
    let mut object = Object::new();
    if let Some(id) = scope_id {
        object.insert("scope_id", Value::String(id));
    }
    object
}

pub async fn list(client: &RpcClient, scope: Option<String>) -> Result<Vec<Label>, String> {
    let value = call(client, "list", payload(scope)).await?;
    decode_labels(value).map_err(|e| e.message)
}

pub async fn load(
    client: &RpcClient,
    scope: Option<String>,
    target: &EntityTarget,
) -> Result<Vec<Label>, String> {
    let value = call(client, "load", entity_payload(scope, target)).await?;
    decode_labels(value).map_err(|e| e.message)
}

pub async fn replace(
    client: &RpcClient,
    scope: Option<String>,
    target: &EntityTarget,
    ids: Vec<String>,
) -> Result<Vec<Label>, String> {
    let mut object = entity_payload(scope, target);
    object.insert(
        "label_ids",
        Value::List(ids.into_iter().map(Value::String).collect()),
    );
    let value = call(client, "replace", object).await?;
    decode_labels(value).map_err(|e| e.message)
}

pub async fn remove(
    client: &RpcClient,
    scope: Option<String>,
    target: &EntityTarget,
    ids: Vec<String>,
) -> Result<Vec<Label>, String> {
    let mut object = entity_payload(scope, target);
    object.insert(
        "label_ids",
        Value::List(ids.into_iter().map(Value::String).collect()),
    );
    let value = call(client, "remove", object).await?;
    decode_labels(value).map_err(|e| e.message)
}

pub async fn save(
    client: &RpcClient,
    scope: Option<String>,
    label: Label,
) -> Result<Label, String> {
    let mut object = payload(scope);
    object.insert("label", Value::Object(label.to_object()));
    match call(client, "save", object).await? {
        Value::Object(object) => Label::from_object(&object).map_err(|e| e.message),
        _ => Err("Unexpected label response".into()),
    }
}

pub async fn delete(client: &RpcClient, scope: Option<String>, id: String) -> Result<(), String> {
    let mut object = payload(scope);
    object.insert("id", Value::String(id));
    call(client, "delete", object).await.map(|_| ())
}

fn entity_payload(scope: Option<String>, target: &EntityTarget) -> Object {
    let mut object = payload(scope);
    object.insert("id", Value::String(target.id.clone()));
    object.insert(
        "collection",
        Value::String(target.collection_or_default().into()),
    );
    object
}

async fn call(client: &RpcClient, command: &str, object: Object) -> Result<Value, String> {
    client
        .invoke_value(
            format!("semantic.base.labels.{command}"),
            Value::Object(object),
        )
        .await
        .map_err(|e| e.to_string())
}
