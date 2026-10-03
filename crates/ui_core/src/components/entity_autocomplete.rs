use std::time::Duration;

use dioxus::prelude::*;
use semantic_data::value::Value;

use crate::{
    context::{use_active_scope_id, use_rpc_client},
    form::ref_autocomplete_query_ast,
    query_ast::query_payload,
};

const ENTITY_LABEL_FIELDS: [&str; 6] = [
    "id",
    "semantic:title",
    "title",
    "name",
    "display_name",
    "semantic:base:person:display_name",
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct EntityOption {
    id: String,
    label: String,
}

#[derive(Clone, PartialEq, Props)]
pub struct EntityAutocompleteProps {
    #[props(default)]
    pub collection: Option<String>,

    /// Canonical query fields for callers whose catalog contains ambiguous short aliases.
    #[props(default)]
    pub search_fields: Option<Vec<String>>,

    #[props(default)]
    pub value: Option<String>,

    pub on_value_change: EventHandler<Option<String>>,

    #[props(default)]
    pub allowed_class_ids: Vec<String>,

    #[props(default)]
    pub excluded_id: Option<String>,

    #[props(default)]
    pub disabled: bool,

    #[props(default = "Search entities".to_string())]
    pub placeholder: String,

    #[props(default = "Entity".to_string())]
    pub aria_label: String,
}

#[component]
pub fn EntityAutocomplete(props: EntityAutocompleteProps) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| props.value.clone());
    let collection = use_memo(use_reactive((&props.collection,), |(collection,)| {
        collection
    }));
    let search_fields = use_memo(use_reactive((&props.search_fields,), |(fields,)| fields));
    use_effect(use_reactive((&props.value,), move |(value,)| {
        selected.set(value);
    }));
    let allowed_class_ids = props.allowed_class_ids.clone();
    let excluded_id = props.excluded_id.clone();
    let options = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let search = query();
        let selected_id = selected();
        let collection = collection();
        let search_fields = search_fields();
        let allowed_class_ids = allowed_class_ids.clone();
        let excluded_id = excluded_id.clone();
        async move {
            dioxus_sdk_time::sleep(Duration::from_millis(250)).await;
            let query = autocomplete_query(
                &search,
                &allowed_class_ids,
                excluded_id.as_deref(),
                collection.as_deref(),
                search_fields.as_deref(),
            );
            let mut options = client
                .invoke_value(
                    "semantic.db.query",
                    query_payload(query, scope_id.as_deref(), None),
                )
                .await
                .map(entity_options_from_query_response)
                .unwrap_or_default();
            // A saved selection may be outside the bounded search page. Load it
            // explicitly so the control still displays its label on a deep link.
            if let Some(id) =
                selected_id.filter(|id| !options.iter().any(|option| option.id == *id))
            {
                let selected = client
                    .invoke_value(
                        "semantic.db.query",
                        query_payload(
                            selected_entity_query(&id, collection.as_deref()),
                            scope_id.as_deref(),
                            None,
                        ),
                    )
                    .await
                    .map(entity_options_from_query_response)
                    .unwrap_or_default();
                options.extend(selected);
            }
            options
        }
    });
    let options = options.read().clone().unwrap_or_default();
    let canonical_search = props.search_fields.is_some();
    rsx! {
        dxcomp::Combobox::<String> {
            value: Some(selected.into()),
            disabled: props.disabled,
            on_value_change: move |value: Option<String>| {
                selected.set(value.clone());
                props.on_value_change.call(value);
            },
            on_query_change: move |value| query.set(value),
            // Canonical fields can match an id even when its display title differs.
            filter: move |(query, text): (String, String)| canonical_search || dxcomp::primitives::combobox::default_combobox_filter(&query, &text),
            placeholder: props.placeholder,
            aria_label: props.aria_label.clone(),
            list_aria_label: format!("{} options", props.aria_label),
            dxcomp::ComboboxEmpty { "No entity found." }
            for (index, option) in options.iter().enumerate() {
                dxcomp::ComboboxOption::<String> {
                    index,
                    value: option.id.clone(),
                    text_value: option.label.clone(),
                    span { "{option.label}" }
                    if option.label != option.id {
                        code { " {option.id}" }
                    }
                }
            }
        }
    }
}

fn selected_entity_query(id: &str, collection: Option<&str>) -> semantic_data::query::SelectQuery {
    use crate::query_ast::{binary, field, string};
    use semantic_data::query::{BinaryOp, SelectQuery};
    SelectQuery::new()
        .with_collection(collection.unwrap_or(semantic_data::builtin::DEFAULT_COLLECTION))
        .with_predicate(binary(BinaryOp::Eq, field(&["id"]), string(id)))
        .with_limit(1)
}

fn autocomplete_query(
    search: &str,
    classes: &[String],
    excluded: Option<&str>,
    collection: Option<&str>,
    fields: Option<&[String]>,
) -> semantic_data::query::SelectQuery {
    use crate::query_ast::{all, any, field, ilike};
    let mut query = ref_autocomplete_query_ast(
        if fields.is_some() { "" } else { search },
        classes,
        excluded,
    )
    .with_collection(collection.unwrap_or(semantic_data::builtin::DEFAULT_COLLECTION));
    if let Some(fields) = fields.filter(|_| !search.trim().is_empty()) {
        let pattern = format!("%{}%", search.trim());
        let search = any(fields
            .iter()
            .map(|name| ilike(field(&[name.as_str()]), &pattern)));
        query.predicate = Some(match query.predicate.take() {
            Some(predicate) => all([predicate, search]),
            None => search,
        });
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::value::IntoValue;
    #[test]
    fn graph_picker_uses_canonical_fields_and_preserves_constraints() {
        let fields = vec!["id".into(), "semantic:title".into()];
        let query = autocomplete_query(
            "needle",
            &["class".into()],
            Some("excluded"),
            Some("custom"),
            Some(&fields),
        );
        let serialized = serde_json::to_string(&query.into_value()).unwrap();
        for required in ["semantic:title", "custom", "excluded", "needle", "class"] {
            assert!(serialized.contains(required));
        }
        for ambiguous in ["\"name\"", "display_name", "note_title", "label_name"] {
            assert!(!serialized.contains(ambiguous));
        }
        assert_eq!(
            autocomplete_query("needle", &[], None, None, None),
            ref_autocomplete_query_ast("needle", &[], None)
        );
    }
}

fn entity_options_from_query_response(value: Value) -> Vec<EntityOption> {
    let Value::Object(object) = value else {
        return Vec::new();
    };
    let Some(Value::List(rows)) = object.get("rows") else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let Value::Object(object) = row else {
                return None;
            };
            let id = object.get("id").and_then(Value::as_str)?.to_string();
            let label = ENTITY_LABEL_FIELDS
                .iter()
                .skip(1)
                .find_map(|field| object.get(*field).and_then(Value::as_str))
                .filter(|label| !label.is_empty())
                .unwrap_or(&id)
                .to_string();
            Some(EntityOption { id, label })
        })
        .collect()
}
