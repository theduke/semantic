use dioxus::prelude::*;
use futures::FutureExt;
use semantic_data::{
    schema::ClassType,
    value::{Object, Value},
};
use semantic_ui_core::{
    DynamicClassForm, SemanticFormMode, SemanticFormSubmit, SubmitError, default_value_for_class,
    rpc_batch_upsert_submit_handler_with_primary_id, use_active_scope_id, use_rpc_client,
    use_ui_catalog,
};
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

    let class_id = selected_class_id.read().clone().unwrap_or_default();
    let collection = selected_collection
        .read()
        .clone()
        .unwrap_or_else(|| default_collection.clone());
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
                        }
                    }
                }
                {
                    let entity_id = new_entity_id();
                    let mut object = match default_value_for_class(&class, &catalog) {
                        Value::Object(object) => object,
                        _ => Object::new(),
                    };
                    object.insert("type", Value::String(class.id.clone()));
                    object.insert(primary_id_field.clone(), Value::String(entity_id.clone()));
                    let submit = rpc_batch_upsert_submit_handler_from_primary_id_field(
                        client.clone(),
                        scope_id.clone(),
                        collection.clone(),
                        primary_id_field.clone(),
                    );
                    rsx! {
                        div { key: "{class.id}:{collection}",
                            DynamicClassForm {
                                class: class.clone(),
                                object,
                                mode: SemanticFormMode::Create,
                                collection: Some(collection.clone()),
                                id: None,
                                scope_id: scope_id.clone(),
                                submit: Some(submit)
                            }
                        }
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
                .find(|field| field.canonical_field == "id")
                .or_else(|| {
                    collection
                        .field_ids
                        .iter()
                        .find(|field| field.canonical_field == "semantic:catalog:id")
                })
        })
        .map(|field| field.canonical_field.clone())
        .unwrap_or_else(|| "id".to_string())
}

fn rpc_batch_upsert_submit_handler_from_primary_id_field(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: String,
    primary_id_field: String,
) -> SemanticFormSubmit {
    SemanticFormSubmit::async_(move |ctx| {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let collection = collection.clone();
        let primary_id_field = primary_id_field.clone();
        async move {
            let Value::Object(object) = ctx.value else {
                return Err(SubmitError::message("submitted value must be an object"));
            };
            let id = submitted_primary_id(&object, &primary_id_field)?;
            let mut payload = Object::new();
            if let Some(scope_id) = scope_id {
                payload.insert("scope_id", Value::String(scope_id));
            }
            payload.insert(
                "operations",
                Value::List(vec![Value::Object(batch_upsert_operation(
                    collection, id, object,
                ))]),
            );
            client
                .invoke_value("semantic.db.batch", Value::Object(payload))
                .await
                .map(|_| ())
                .map_err(|err| SubmitError::message(err.to_string()))
        }
        .boxed_local()
    })
}

fn submitted_primary_id(
    object: &Object,
    primary_id_field: &str,
) -> std::result::Result<String, SubmitError> {
    match object.get(primary_id_field) {
        Some(Value::String(id)) if !id.trim().is_empty() => Ok(id.clone()),
        Some(Value::String(_)) | None => Err(SubmitError::message("id is required")),
        Some(_) => Err(SubmitError::message("id must be a string")),
    }
}

fn batch_upsert_operation(collection: String, id: String, object: Object) -> Object {
    let mut operation = Object::new();
    operation.insert("kind", Value::String("upsert".to_string()));
    operation.insert("collection", Value::String(collection));
    operation.insert("id", Value::String(id));
    operation.insert("object", Value::Object(object));
    operation
}
