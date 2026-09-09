use std::sync::atomic::{AtomicU32, Ordering};

use dioxus::prelude::*;
use semantic_data::{
    builtin::ATTR_ID,
    schema::ClassType,
    value::{Object, Value},
};
use semantic_ui_core::{
    DynamicClassForm, SemanticFormMode, SemanticFormSubmit, UiCatalog,
    components::{EmptyState, LoadingSkeleton},
    default_value_for_class,
    form::{SemanticFormActionLabels, SemanticFormSubmitFailure, SemanticFormSubmitOutcome},
    use_active_scope_id, use_ui_catalog, use_ui_catalog_context, use_ui_catalog_reload,
};

use super::{ConfirmActionRequest, UnsavedChangesPrompt};

static ENTITY_CREATE_ID_SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityCreateFailure {
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityCreateOutcome {
    pub collection: String,
    pub id: String,
    pub object: Object,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingSelection {
    Class(String),
    Collection(String),
}

/// Reusable schema-driven entity editor. Persistence is entirely supplied by the caller.
#[component]
pub fn EntityCreateForm(
    #[props(default)] fixed_collection: Option<String>,
    submit: SemanticFormSubmit,
    on_created: EventHandler<EntityCreateOutcome>,
    #[props(default)] on_dirty_change: Option<EventHandler<bool>>,
    #[props(default)] on_submitting_change: Option<EventHandler<bool>>,
    #[props(default)] on_collection_change: Option<EventHandler<String>>,
    #[props(default)] on_failure: Option<EventHandler<EntityCreateFailure>>,
) -> Element {
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let catalog_signal = use_ui_catalog_context().catalog_signal();
    let reload_catalog = use_ui_catalog_reload().reload;
    let classes = use_memo(move || {
        catalog_signal
            .read()
            .as_ref()
            .map(|catalog| catalog.creatable_classes().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    });
    let collections = use_memo(move || {
        catalog_signal
            .read()
            .as_ref()
            .map(|catalog| {
                catalog
                    .collections()
                    .map(|collection| collection.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    let initial_class = classes.read().first().map(|class| class.id.clone());
    let initial_collection = fixed_collection
        .clone()
        .or_else(|| collections.read().first().cloned());
    let mut selected_class_id = use_signal(move || initial_class);
    let mut selected_collection = use_signal(move || initial_collection);
    let mut draft_id = use_signal(new_entity_id);
    let mut dirty = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut pending_selection = use_signal(|| None::<PendingSelection>);
    let class_id = selected_class_id.read().clone().unwrap_or_default();
    let selected_class = classes
        .read()
        .iter()
        .find(|class| class.id == class_id)
        .cloned();
    let collection = selected_collection.read().clone().unwrap_or_default();
    let primary_id_field = primary_id_field_for_collection(&catalog, &collection);
    let picker_collection = collection.clone();
    let announced_collection = collection.clone();
    use_effect(move || {
        if !announced_collection.is_empty() {
            if let Some(on_collection_change) = on_collection_change {
                on_collection_change.call(announced_collection.clone());
            }
        }
    });
    let apply_selection = Callback::new(move |selection: PendingSelection| {
        match selection {
            PendingSelection::Class(value) => selected_class_id.set(Some(value)),
            PendingSelection::Collection(value) => {
                selected_collection.set(Some(value.clone()));
                if let Some(on_collection_change) = on_collection_change {
                    on_collection_change.call(value);
                }
            }
        }
        draft_id.set(new_entity_id());
        dirty.set(false);
        submitting.set(false);
        if let Some(on_dirty_change) = on_dirty_change {
            on_dirty_change.call(false);
        }
    });

    if classes.read().is_empty() {
        if catalog_signal.read().is_none() {
            return rsx! { LoadingSkeleton { line_count: 6 } };
        }
        return rsx! {
            EmptyState {
                title: "No entity classes available",
                description: "The active catalog does not define a class that can supply this form.",
                action_label: "Refresh catalog",
                on_action: move |_| reload_catalog.call(()),
            }
        };
    }
    if collection.is_empty() {
        return rsx! {
            EmptyState {
                title: "No collections available",
                description: "Add or load a collection before creating an entity.",
                action_label: "Refresh catalog",
                on_action: move |_| reload_catalog.call(()),
            }
        };
    }

    rsx! {
        div { class: "semantic-create-entity__context semantic-create-entity__context--embedded",
            div { class: "semantic-create-entity__picker",
                label { class: "semantic-create-entity__picker-label", "Class" }
                p { class: "semantic-create-entity__picker-help", "Controls the fields and validation shown below." }
                dxcomp::Combobox::<String> {
                    value: Some(selected_class_id.into()),
                    on_value_change: move |value: Option<String>| {
                        if value.as_deref() != Some(class_id.as_str()) {
                            if let Some(value) = value {
                                let selection = PendingSelection::Class(value);
                                if dirty() {
                                    pending_selection.set(Some(selection));
                                } else {
                                    apply_selection.call(selection);
                                }
                            }
                        }
                    },
                    disabled: submitting(),
                    placeholder: "Filter classes",
                    aria_label: "Entity class",
                    list_aria_label: "Entity classes",
                    dxcomp::ComboboxEmpty { "No class found." }
                    for (index, option) in classes.read().iter().enumerate() {
                        dxcomp::ComboboxOption::<String> {
                            index,
                            value: option.id.clone(),
                            text_value: class_label(option),
                            "{class_label(option)}"
                        }
                    }
                }
            }
            div { class: "semantic-create-entity__picker",
                label { class: "semantic-create-entity__picker-label", "Collection" }
                if fixed_collection.is_some() {
                    p { class: "semantic-create-entity__picker-help", "Entities created here are stored in the directory-compatible collection." }
                    code { "{collection}" }
                } else {
                    p { class: "semantic-create-entity__picker-help", "Determines where the entity is stored and its detail route." }
                    dxcomp::Combobox::<String> {
                        value: Some(selected_collection.into()),
                        on_value_change: move |value: Option<String>| {
                            if let Some(value) = value {
                                if value != picker_collection {
                                    let selection = PendingSelection::Collection(value);
                                    if dirty() {
                                        pending_selection.set(Some(selection));
                                    } else {
                                        apply_selection.call(selection);
                                    }
                                }
                            }
                        },
                        disabled: submitting(),
                        placeholder: "Filter collections",
                        aria_label: "Entity collection",
                        list_aria_label: "Entity collections",
                        dxcomp::ComboboxEmpty { "No collection found." }
                        for (index, option) in collections.read().iter().enumerate() {
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
            p {
                class: "semantic-create-entity__status",
                role: "status",
                aria_live: "polite",
                if submitting() {
                    "Creating entity…"
                } else if dirty() {
                    "Unsaved changes"
                } else {
                    "Ready to create"
                }
            }
        }
        if let Some(class) = selected_class {
            {
                let mut object = match default_value_for_class(&class, &catalog) {
                    Value::Object(object) => object,
                    _ => Object::new(),
                };
                object.insert("type", Value::String(class.id.clone()));
                object.insert(primary_id_field.clone(), Value::String(draft_id()));
                let outcome_collection = collection.clone();
                let outcome_primary_id_field = primary_id_field.clone();
                rsx! {
                    div {
                        class: "semantic-create-entity__form semantic-create-entity__form--embedded",
                        key: "{class.id}:{collection}:{draft_id}",
                        DynamicClassForm {
                            class,
                            object,
                            mode: SemanticFormMode::Create,
                            collection: Some(collection.clone()),
                            id: None,
                            scope_id,
                            submit: Some(submit),
                            action_labels: SemanticFormActionLabels::create_entity(),
                            on_dirty_change: move |next_dirty| {
                                dirty.set(next_dirty);
                                if let Some(on_dirty_change) = on_dirty_change {
                                    on_dirty_change.call(next_dirty);
                                }
                            },
                            on_submitting_change: move |next_submitting| {
                                submitting.set(next_submitting);
                                if let Some(on_submitting_change) = on_submitting_change {
                                    on_submitting_change.call(next_submitting);
                                }
                            },
                            on_submit_success: move |outcome: SemanticFormSubmitOutcome| {
                                if let Value::Object(object) = outcome.value {
                                    match submitted_primary_id(&object, &outcome_primary_id_field) {
                                        Ok(id) => on_created.call(EntityCreateOutcome {
                                            collection: outcome_collection.clone(),
                                            id,
                                            object,
                                        }),
                                        Err(message) => if let Some(on_failure) = on_failure {
                                            on_failure.call(EntityCreateFailure { message });
                                        },
                                    }
                                } else if let Some(on_failure) = on_failure {
                                    on_failure.call(EntityCreateFailure {
                                        message: "Created entity response did not contain an object.".to_string(),
                                    });
                                }
                            },
                            on_submit_failure: move |failure: SemanticFormSubmitFailure| {
                                if let Some(on_failure) = on_failure {
                                    on_failure.call(EntityCreateFailure {
                                        message: failure.errors.first()
                                            .map(|error| error.message.clone())
                                            .unwrap_or_else(|| "Entity creation failed. Review the form and retry.".to_string()),
                                    });
                                }
                            },
                        }
                    }
                }
            }
        } else {
            EmptyState {
                title: "Selected class is unavailable",
                description: "The catalog changed and no longer contains this class. Refresh to choose an available class.",
                action_label: "Refresh catalog",
                on_action: move |_| reload_catalog.call(()),
            }
        }
        UnsavedChangesPrompt {
            open: pending_selection.read().is_some(),
            target: "another entity type or collection",
            body: "Changing this selection will discard the current entity draft.",
            on_open_change: move |open: bool| if !open { pending_selection.set(None) },
            on_discard: move |request: ConfirmActionRequest| {
                let selection = { pending_selection.peek().clone() };
                if let Some(selection) = selection {
                    pending_selection.set(None);
                    apply_selection.call(selection);
                }
                request.complete(Ok(()));
            },
        }
    }
}

fn primary_id_field_for_collection(catalog: &UiCatalog, collection: &str) -> String {
    catalog
        .collection_by_name(collection)
        .and_then(|collection| {
            collection
                .field_ids
                .iter()
                .find(|field| field.canonical_field == ATTR_ID)
                .or_else(|| {
                    collection
                        .field_ids
                        .iter()
                        .find(|field| field.canonical_field == "semantic:catalog:id")
                })
        })
        .map(|field| field.canonical_field.clone())
        .unwrap_or_else(|| ATTR_ID.to_string())
}

fn class_label(class: &ClassType) -> String {
    class
        .meta
        .title
        .clone()
        .unwrap_or_else(|| class.name.clone())
}

fn submitted_primary_id(
    object: &Object,
    primary_id_field: &str,
) -> std::result::Result<String, String> {
    match object.get(primary_id_field) {
        Some(Value::String(id)) if !id.trim().is_empty() => Ok(id.clone()),
        Some(Value::String(_)) | None => {
            Err("Created entity did not contain a usable ID.".to_string())
        }
        Some(_) => Err("Created entity ID must be a string.".to_string()),
    }
}

fn new_entity_id() -> String {
    let nanos = time::UtcDateTime::now().unix_timestamp_nanos().max(0) as u128;
    let sequence = ENTITY_CREATE_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    new_entity_id_from_parts(nanos, sequence)
}

fn new_entity_id_from_parts(nanos: u128, sequence: u32) -> String {
    format!("entity-{nanos:032x}-{sequence:08x}")
}

#[cfg(test)]
mod tests {
    use super::new_entity_id_from_parts;

    #[test]
    fn generated_entity_ids_are_stable_and_sequence_sensitive() {
        assert_eq!(
            new_entity_id_from_parts(0x1234, 0x2a),
            "entity-00000000000000000000000000001234-0000002a"
        );
        assert_ne!(
            new_entity_id_from_parts(0x1234, 0x2a),
            new_entity_id_from_parts(0x1234, 0x2b)
        );
    }
}
