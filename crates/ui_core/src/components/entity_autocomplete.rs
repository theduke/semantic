use std::time::Duration;

use dioxus::prelude::*;
use semantic_data::value::{Object, Value};

use crate::{
    context::{use_active_scope_id, use_rpc_client},
    form::ref_autocomplete_query,
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
    use_effect(use_reactive((&props.value,), move |(value,)| {
        selected.set(value);
    }));
    let allowed_class_ids = props.allowed_class_ids.clone();
    let excluded_id = props.excluded_id.clone();
    let options = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let search = query();
        let allowed_class_ids = allowed_class_ids.clone();
        let excluded_id = excluded_id.clone();
        async move {
            dioxus_sdk_time::sleep(Duration::from_millis(250)).await;
            let sql = ref_autocomplete_query(&search, &allowed_class_ids, excluded_id.as_deref());
            let mut payload = Object::new();
            if let Some(scope_id) = scope_id {
                payload.insert("scope_id", Value::String(scope_id));
            }
            payload.insert("format", Value::String("sql".to_string()));
            payload.insert("query", Value::String(sql));
            client
                .invoke_value("semantic.db.query", Value::Object(payload))
                .await
                .map(entity_options_from_query_response)
                .unwrap_or_default()
        }
    });
    let options = options.read().clone().unwrap_or_default();
    rsx! {
        dxcomp::Combobox::<String> {
            value: Some(selected.into()),
            disabled: props.disabled,
            on_value_change: move |value: Option<String>| {
                selected.set(value.clone());
                props.on_value_change.call(value);
            },
            on_query_change: move |value| query.set(value),
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
