use std::rc::Rc;

use dioxus::prelude::use_hook;
use dxeditor::{
    EntityLinkCandidate, EntityLinkExtension, EntityLinkPreview, EntityLinkProvider,
    EntityPreviewField,
};
use futures::future::LocalBoxFuture;
use semantic_data::value::{Object, Value};
use semantic_rpc::RpcClient;

use crate::{
    context::{use_active_scope_id, use_rpc_client},
    form::ref_autocomplete_query,
    ui_catalog::{EntityOpenHandler, EntityTarget, use_ui_catalog},
};

const LABEL_FIELDS: [&str; 5] = [
    "semantic:title",
    "title",
    "name",
    "display_name",
    "semantic:base:person:display_name",
];

#[derive(Clone)]
struct SemanticEntityLinkProvider {
    client: RpcClient,
    scope_id: Option<String>,
    open: Option<EntityOpenHandler>,
}

impl EntityLinkProvider for SemanticEntityLinkProvider {
    fn search(&self, query: String) -> LocalBoxFuture<'static, Vec<EntityLinkCandidate>> {
        let provider = self.clone();
        Box::pin(async move {
            provider
                .query(ref_autocomplete_query(&query, &[], None))
                .await
                .map(entity_candidates)
                .unwrap_or_default()
        })
    }

    fn preview(&self, entity_id: String) -> LocalBoxFuture<'static, Option<EntityLinkPreview>> {
        let provider = self.clone();
        Box::pin(async move {
            let sql = preview_query(&entity_id);
            provider
                .query(sql)
                .await
                .and_then(first_row)
                .map(entity_preview)
        })
    }

    fn open(&self, entity_id: String) {
        if let Some(open) = &self.open {
            open(EntityTarget::default_collection(entity_id));
        }
    }
}

fn preview_query(entity_id: &str) -> String {
    format!(
        "SELECT * FROM {} WHERE \"id\" = '{}' LIMIT 1",
        semantic_data::builtin::DEFAULT_COLLECTION,
        entity_id.replace('\'', "''")
    )
}

impl SemanticEntityLinkProvider {
    async fn query(&self, sql: String) -> Option<Value> {
        let mut payload = Object::new();
        if let Some(scope_id) = self.scope_id.clone() {
            payload.insert("scope_id", Value::String(scope_id));
        }
        payload.insert("format", Value::String("sql".to_string()));
        payload.insert("query", Value::String(sql));
        self.client
            .invoke_value("semantic.db.query", Value::Object(payload))
            .await
            .ok()
    }
}

pub(crate) fn use_semantic_entity_links() -> EntityLinkExtension {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let open = use_ui_catalog().entity_navigation().open.clone();
    use_hook(move || {
        EntityLinkExtension::new(Rc::new(SemanticEntityLinkProvider {
            client,
            scope_id,
            open,
        }))
        .with_accent_color("var(--color-primary, #176b87)")
    })
}

fn rows(value: &Value) -> Option<&[Value]> {
    let Value::Object(object) = value else {
        return None;
    };
    let Some(Value::List(rows)) = object.get("rows") else {
        return None;
    };
    Some(rows)
}

fn first_row(value: Value) -> Option<Object> {
    rows(&value)?.first().and_then(|row| match row {
        Value::Object(object) => Some(object.clone()),
        _ => None,
    })
}

fn entity_candidates(value: Value) -> Vec<EntityLinkCandidate> {
    rows(&value)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| match row {
            Value::Object(object) => candidate(object),
            _ => None,
        })
        .collect()
}

fn candidate(object: &Object) -> Option<EntityLinkCandidate> {
    let id = object.get("id").and_then(Value::as_str)?.to_string();
    Some(EntityLinkCandidate {
        label: entity_label(object, &id),
        detail: object
            .get("type")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        id,
    })
}

fn entity_preview(object: Object) -> EntityLinkPreview {
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let detail = object
        .get("type")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let fields = object
        .iter()
        .filter(|(key, _)| {
            !["id", "type"].contains(&key.as_str()) && !LABEL_FIELDS.contains(&key.as_str())
        })
        .filter_map(|(key, value)| {
            let value = preview_value(value);
            (!value.is_empty()).then(|| EntityPreviewField {
                label: key.clone(),
                value,
            })
        })
        .take(8)
        .collect();
    EntityLinkPreview {
        label: entity_label(&object, &id),
        id,
        detail,
        fields,
    }
}

fn entity_label(object: &Object, id: &str) -> String {
    LABEL_FIELDS
        .iter()
        .find_map(|field| object.get(*field).and_then(Value::as_str))
        .filter(|label| !label.trim().is_empty())
        .unwrap_or(id)
        .to_string()
}

fn preview_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::I8(value) => value.to_string(),
        Value::I16(value) => value.to_string(),
        Value::I32(value) => value.to_string(),
        Value::I64(value) => value.to_string(),
        Value::I128(value) => value.to_string(),
        Value::U8(value) => value.to_string(),
        Value::U16(value) => value.to_string(),
        Value::U32(value) => value.to_string(),
        Value::U64(value) => value.to_string(),
        Value::U128(value) => value.to_string(),
        Value::F32(value) => value.to_string(),
        Value::F64(value) => value.to_string(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    use semantic_rpc::{RpcClientDyn, RpcClientError};

    struct MockClient {
        calls: Rc<RefCell<Vec<(String, Value)>>>,
        response: Value,
    }

    impl RpcClientDyn for MockClient {
        fn invoke_value(
            &self,
            command: String,
            payload: Value,
        ) -> LocalBoxFuture<'static, std::result::Result<Value, RpcClientError>> {
            self.calls.borrow_mut().push((command, payload));
            let response = self.response.clone();
            Box::pin(async move { Ok(response) })
        }
    }

    fn response_row() -> Object {
        let mut row = Object::new();
        row.insert("id", Value::String("person-1".to_string()));
        row.insert("title", Value::String("Ada Lovelace".to_string()));
        row.insert("type", Value::String("Person".to_string()));
        row.insert("email", Value::String("ada@example.test".to_string()));
        row
    }

    fn response(row: Object) -> Value {
        let mut response = Object::new();
        response.insert("rows", Value::List(vec![Value::Object(row)]));
        Value::Object(response)
    }

    #[test]
    fn candidates_and_previews_use_application_entity_display_data() {
        let row = response_row();

        assert_eq!(
            entity_candidates(response(row.clone())),
            vec![EntityLinkCandidate {
                id: "person-1".to_string(),
                label: "Ada Lovelace".to_string(),
                detail: Some("Person".to_string()),
            }]
        );
        let preview = entity_preview(row);
        assert_eq!(preview.label, "Ada Lovelace");
        assert_eq!(preview.fields[0].value, "ada@example.test");
    }

    #[test]
    fn preview_query_escapes_entity_ids() {
        assert_eq!(
            preview_query("person' OR 1=1 --"),
            "SELECT * FROM entities WHERE \"id\" = 'person'' OR 1=1 --' LIMIT 1"
        );
    }

    #[test]
    fn provider_queries_semantic_rpc_with_scope_for_search_and_preview() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let client = RpcClient::new(MockClient {
            calls: calls.clone(),
            response: response(response_row()),
        });
        let provider = SemanticEntityLinkProvider {
            client,
            scope_id: Some("scope-1".to_string()),
            open: None,
        };

        let candidates = futures::executor::block_on(provider.search("Ada".to_string()));
        let preview = futures::executor::block_on(provider.preview("person-1".to_string()));
        assert_eq!(candidates[0].label, "Ada Lovelace");
        assert_eq!(preview.unwrap().fields[0].label, "email");

        let calls = calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|(command, payload)| {
            command == "semantic.db.query"
                && matches!(payload, Value::Object(value) if value.get("scope_id").and_then(Value::as_str) == Some("scope-1"))
        }));
        assert!(
            matches!(&calls[0].1, Value::Object(value) if value.get("query").and_then(Value::as_str).is_some_and(|query| query.contains("ILIKE '%Ada%'")))
        );
        assert!(
            matches!(&calls[1].1, Value::Object(value) if value.get("query").and_then(Value::as_str).is_some_and(|query| query.contains("WHERE \"id\" = 'person-1'")))
        );
    }
}
