use dioxus::prelude::*;
use semantic_data::{
    schema::ClassType,
    value::{Object, Value},
};
use semantic_ui_core::{
    DynamicClassForm, SemanticFormMode, default_value_for_class,
    rpc_batch_upsert_submit_handler_with_primary_id, use_active_scope_id, use_rpc_client,
    use_ui_catalog,
};

use crate::views::Route;

#[component]
pub fn CreateEntityPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let classes: Vec<ClassType> = catalog.classes().cloned().collect();
    let collections: Vec<String> = catalog
        .collections()
        .map(|collection| collection.name.clone())
        .collect();
    let default_collection = collections
        .first()
        .cloned()
        .unwrap_or_else(|| "entities".to_string());
    let initial_class_id = classes
        .first()
        .map(|class| class.id.clone())
        .unwrap_or_default();
    let mut selected_class_id = use_signal(|| Some(initial_class_id));
    let mut selected_collection = use_signal(|| Some(default_collection.clone()));
    let mut id = use_signal(new_entity_id);

    let class_id = selected_class_id.read().clone().unwrap_or_default();
    let collection = selected_collection
        .read()
        .clone()
        .unwrap_or_else(|| default_collection.clone());
    let entity_id = id.read().clone();
    let selected_class = classes
        .iter()
        .find(|class| class.id == class_id)
        .cloned()
        .or_else(|| classes.first().cloned());
    let primary_id_field = primary_id_field_for_collection(&catalog, &collection);
    let collection_options = if collections.is_empty() {
        vec!["entities".to_string()]
    } else {
        collections.clone()
    };

    rsx! {
        section { class: "semantic-form-screen semantic-create-entity",
            h2 { "Create Entity" }
            if classes.is_empty() {
                div { class: "semantic-empty", "No classes are registered in the catalog." }
            } else if let Some(class) = selected_class {
                div { class: "semantic-table-wrap semantic-form-screen__meta",
                    table { class: "semantic-field-table semantic-form-screen__meta-table",
                        tbody {
                            tr {
                                th { scope: "row", "Class" }
                                td {
                                    dxcomp::Combobox::<String> {
                                        value: Some(selected_class_id.into()),
                                        on_value_change: move |value| {
                                            if let Some(value) = value {
                                                selected_class_id.set(Some(value));
                                            }
                                        },
                                        placeholder: "Filter classes",
                                        aria_label: "Entity class",
                                        list_aria_label: "Entity classes",
                                        dxcomp::ComboboxEmpty { "No class found." }
                                        for (index, option) in classes.iter().enumerate() {
                                            dxcomp::ComboboxOption::<String> {
                                                index,
                                                value: option.id.clone(),
                                                text_value: class_label(option),
                                                "{class_label(option)}"
                                            }
                                        }
                                    }
                                }
                            }
                            tr {
                                th { scope: "row", "Collection" }
                                td {
                                    dxcomp::Combobox::<String> {
                                        value: Some(selected_collection.into()),
                                        on_value_change: move |value| {
                                            if let Some(value) = value {
                                                selected_collection.set(Some(value));
                                            }
                                        },
                                        placeholder: "Filter collections",
                                        aria_label: "Entity collection",
                                        list_aria_label: "Entity collections",
                                        dxcomp::ComboboxEmpty { "No collection found." }
                                        for (index, option) in collection_options.iter().enumerate() {
                                            dxcomp::ComboboxOption::<String> {
                                                index,
                                                value: option.clone(),
                                                text_value: option.clone(),
                                                "{option}"
                                            }
                                        }
                                    }
                                }
                            }
                            tr {
                                th { scope: "row", "ID" }
                                td {
                                    dxcomp::Input {
                                        class: "semantic-form-screen__id-input",
                                        value: "{entity_id}",
                                        oninput: move |event: FormEvent| id.set(event.value())
                                    }
                                }
                            }
                        }
                    }
                }
                {
                    let mut object = match default_value_for_class(&class, &catalog) {
                        Value::Object(object) => object,
                        _ => Object::new(),
                    };
                    object.insert("type", Value::String(class.id.clone()));
                    object.insert(primary_id_field.clone(), Value::String(entity_id.clone()));
                    let submit = rpc_batch_upsert_submit_handler_with_primary_id(
                        client.clone(),
                        scope_id.clone(),
                        collection.clone(),
                        entity_id.clone(),
                        primary_id_field.clone(),
                    );
                    rsx! {
                        div { key: "{class.id}:{collection}:{entity_id}",
                            DynamicClassForm {
                                class: class.clone(),
                                object,
                                mode: SemanticFormMode::Create,
                                collection: Some(collection.clone()),
                                id: Some(entity_id.clone()),
                                scope_id: scope_id.clone(),
                                submit: Some(submit)
                            }
                        }
                    }
                }
                div { class: "semantic-form-screen__footer",
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        r#type: "button",
                        onclick: move |_| {
                            navigator().push(Route::EntityPage {
                                collection: collection.clone(),
                                id: entity_id.clone(),
                            });
                        },
                        "Open Entity"
                    }
                }
            }
        }
    }
}

#[component]
pub fn EditEntityPage(collection: String, id: String) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let collection_for_load = collection.clone();
    let id_for_load = id.clone();
    let client_for_load = client.clone();
    let scope_id_for_load = scope_id.clone();
    let resource = use_resource(move || {
        let client = client_for_load.clone();
        let scope_id = scope_id_for_load.clone();
        let collection = collection_for_load.clone();
        let id = id_for_load.clone();
        async move { load_entity(client, scope_id, collection, id).await }
    });

    rsx! {
        section { class: "semantic-form-screen semantic-edit-entity",
            h2 { "Edit {collection}/{id}" }
            match &*resource.read_unchecked() {
                Some(Ok(Some(object))) => {
                    let class = catalog.object_class(object).cloned();
                    let primary_id_field = primary_id_field_for_collection(&catalog, &collection);
                    rsx! {
                        if let Some(class) = class {
                            div { class: "semantic-table-wrap semantic-form-screen__meta",
                                table { class: "semantic-field-table semantic-form-screen__meta-table",
                                    tbody {
                                        tr {
                                            th { scope: "row", "Collection" }
                                            td { code { "{collection}" } }
                                        }
                                        tr {
                                            th { scope: "row", "ID" }
                                            td { code { "{id}" } }
                                        }
                                        tr {
                                            th { scope: "row", "Class" }
                                            td { "{class_label(&class)}" }
                                        }
                                    }
                                }
                            }
                            DynamicClassForm {
                                class,
                                object: object.clone(),
                                mode: SemanticFormMode::Edit,
                                collection: Some(collection.clone()),
                                id: Some(id.clone()),
                                scope_id: scope_id.clone(),
                                submit: Some(rpc_batch_upsert_submit_handler_with_primary_id(
                                    client.clone(),
                                    scope_id.clone(),
                                    collection.clone(),
                                    id.clone(),
                                    primary_id_field,
                                ))
                            }
                        } else {
                            div { class: "semantic-error", "This object has no registered class form." }
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

pub async fn load_entity(
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

fn class_label(class: &ClassType) -> String {
    class
        .meta
        .title
        .clone()
        .unwrap_or_else(|| class.name.clone())
}

fn new_entity_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("entity-{millis}")
}

fn primary_id_field_for_collection(
    catalog: &semantic_ui_core::UiCatalog,
    collection: &str,
) -> String {
    catalog
        .collection_by_name(collection)
        .and_then(|collection| {
            collection
                .field_ids
                .iter()
                .find(|field| field.canonical_field == "semantic:id")
                .or_else(|| {
                    collection
                        .field_ids
                        .iter()
                        .find(|field| field.canonical_field == "semantic:catalog:id")
                })
                .or_else(|| {
                    collection
                        .field_ids
                        .iter()
                        .find(|field| field.canonical_field == "id")
                })
        })
        .map(|field| field.canonical_field.clone())
        .unwrap_or_else(|| "id".to_string())
}
