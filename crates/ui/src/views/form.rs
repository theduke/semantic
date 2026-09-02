use std::rc::Rc;

use dioxus::prelude::*;
use futures::FutureExt;
use semantic_data::{
    builtin::{ATTR_ID, DEFAULT_COLLECTION},
    schema::ClassType,
    value::{Object, Value},
};
use semantic_ui_core::{
    DynamicClassForm, EntityTarget, SemanticFormMode, SemanticFormSubmit, SubmitError,
    components::{
        EmptyState, ErrorState, InlineNotice, LoadingSkeleton, NoticeVariant, RefreshingIndicator,
    },
    context::{Toast, use_toast_dispatcher},
    form::{SemanticFormActionLabels, SemanticFormSubmitFailure, SemanticFormSubmitOutcome},
    rpc_batch_upsert_submit_handler_with_primary_id, use_active_scope_id, use_rpc_client,
    use_ui_catalog, use_ui_catalog_reload,
};

use crate::{
    app::use_entity_edit_navigation,
    components::{
        ConfirmActionRequest, EntityCreateForm, EntityCreateOutcome, FormPage, UnsavedChangesPrompt,
    },
    views::Route,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct EditEntityQueryKey {
    scope_id: Option<String>,
    collection: Option<String>,
    id: String,
}

#[derive(Clone)]
struct EditEntityQueryResponse {
    key: EditEntityQueryKey,
    result: std::result::Result<Option<Rc<Object>>, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum EditSubmitFeedback {
    #[default]
    Idle,
    Succeeded,
    Failed(String),
}

#[component]
pub fn CreateEntityPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let navigator = use_navigator();
    let toast = use_toast_dispatcher();
    let mut dirty = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut selected_collection = use_signal(String::new);
    let mut cancel_confirm_open = use_signal(|| false);
    let submit = SemanticFormSubmit::async_(move |ctx| {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let catalog = catalog.clone();
        async move {
            let collection = ctx
                .collection
                .filter(|collection| !collection.trim().is_empty())
                .ok_or_else(|| SubmitError::message("collection is required"))?;
            let Value::Object(object) = ctx.value else {
                return Err(SubmitError::message("submitted value must be an object"));
            };
            let primary_id_field = primary_id_field_for_collection(&catalog, &collection);
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
                .map_err(|error| SubmitError::message(error.to_string()))
        }
        .boxed_local()
    });

    rsx! {
        FormPage {
            title: "Create entity",
            description: "Choose a catalog class and collection, then complete the schema-driven fields.",
            busy: submitting(),
            breadcrumbs: rsx! {
                Link { to: Route::HomePage, "Workspace" }
                span { aria_hidden: "true", "/" }
                span { "Create entity" }
            },
            header_actions: rsx! {
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    r#type: "button",
                    disabled: submitting(),
                    onclick: move |_| {
                        if dirty() {
                            cancel_confirm_open.set(true);
                        } else {
                            navigator.push(cancel_destination(&selected_collection()));
                        }
                    },
                    "Cancel"
                }
            },
            EntityCreateForm {
                submit,
                on_collection_change: move |collection| selected_collection.set(collection),
                on_dirty_change: move |next| dirty.set(next),
                on_submitting_change: move |next| submitting.set(next),
                on_created: move |outcome: EntityCreateOutcome| {
                    dirty.set(false);
                    toast.show(
                        Toast::success("The entity is ready to view.").title("Entity created"),
                    );
                    navigator.push(entity_destination(&outcome.collection, &outcome.id));
                },
            }
        }
        UnsavedChangesPrompt {
            open: cancel_confirm_open(),
            target: "entity list",
            body: "Leaving this page will discard the current entity draft.",
            on_open_change: move |open| cancel_confirm_open.set(open),
            on_discard: move |request: ConfirmActionRequest| {
                cancel_confirm_open.set(false);
                navigator.push(cancel_destination(&selected_collection()));
                request.complete(Ok(()));
            },
        }
    }
}

#[component]
pub fn DefaultEditEntityPage(id: String) -> Element {
    let scope_id = use_active_scope_id();
    let route_key = edit_route_key(scope_id.as_deref(), None, &id);
    rsx! {
        EditEntityPageView {
            key: "{route_key}",
            scope_id,
            collection: None,
            id
        }
    }
}

#[component]
pub fn CollectionEditEntityPage(collection: String, id: String) -> Element {
    let scope_id = use_active_scope_id();
    let route_key = edit_route_key(scope_id.as_deref(), Some(&collection), &id);
    rsx! {
        EditEntityPageView {
            key: "{route_key}",
            scope_id,
            collection: Some(collection),
            id
        }
    }
}

#[component]
fn EditEntityPageView(collection: Option<String>, id: String, scope_id: Option<String>) -> Element {
    let client = use_rpc_client();
    let catalog = use_ui_catalog();
    let reload_catalog = use_ui_catalog_reload().reload;
    let edit_navigation = use_entity_edit_navigation();
    let edit_target = EntityTarget::new(collection.clone(), id.clone());
    let return_to_previous_detail =
        use_signal(move || edit_navigation.consume_detail_return(&edit_target));
    let query_key = use_memo(use_reactive(
        &EditEntityQueryKey {
            scope_id: scope_id.clone(),
            collection: collection.clone(),
            id: id.clone(),
        },
        |key| key,
    ));
    let mut resource = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            let key = query_key();
            async move {
                let result = load_entity(
                    client,
                    key.scope_id.clone(),
                    key.collection.clone(),
                    key.id.clone(),
                )
                .await
                .map(|object| object.map(Rc::new));
                EditEntityQueryResponse { key, result }
            }
        }
    });
    let current_key = query_key();
    let loading = *resource.state().read() == UseResourceState::Pending;
    let response = resource.read().clone();
    let current_response = response
        .as_ref()
        .filter(|response| response.key == current_key);
    let completed_object = current_response
        .and_then(|response| response.result.as_ref().ok())
        .cloned();
    let error = (!loading)
        .then(|| current_response)
        .flatten()
        .and_then(|response| response.result.as_ref().err().cloned());
    let not_found = !loading && matches!(&completed_object, Some(None));
    let mut retained_object = use_signal(|| None::<Rc<Object>>);
    use_effect(use_reactive(
        (&current_key, &completed_object),
        move |(_key, completed_object)| {
            if let Some(object) = completed_object {
                retained_object.set(object);
            }
        },
    ));
    let object = match completed_object {
        Some(object) => object,
        None => retained_object.read().clone(),
    };
    let collection_label = collection
        .clone()
        .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
    let detail_route = edit_entity_destination(collection.as_deref(), &id);
    let return_route = edit_return_destination(collection.as_deref());
    let mut dirty = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut submit_feedback = use_signal(EditSubmitFeedback::default);
    let mut pending_cancel = use_signal(|| false);
    let navigator = use_navigator();
    let toast = use_toast_dispatcher();
    let class = object
        .as_ref()
        .and_then(|object| catalog.object_class(object.as_ref()).cloned());
    let class_name = class.as_ref().map(class_label);
    let status_message = edit_status_message(submitting(), dirty(), &submit_feedback.read());
    let form_key = edit_route_key(scope_id.as_deref(), collection.as_deref(), &id);
    let cancel_target = format!("entity details for `{id}`");
    let cancel_collection = collection.clone();
    let cancel_id = id.clone();

    rsx! {
        FormPage {
            title: "Edit entity",
            description: "Update schema-backed fields while keeping the entity identity fixed.",
            busy: loading || submitting(),
            breadcrumbs: rsx! {
                Link { to: return_route.clone(), "Entities" }
                span { aria_hidden: "true", "/" }
                Link { to: detail_route.clone(), "{id}" }
                span { aria_hidden: "true", "/" }
                span { "Edit" }
            },
            header_actions: rsx! {
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    r#type: "button",
                    disabled: loading || submitting(),
                    onclick: move |_| resource.restart(),
                    "Refresh"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    r#type: "button",
                    disabled: submitting(),
                    onclick: move |_| {
                        let (destination, confirm) = edit_cancel_action(
                            cancel_collection.as_deref(),
                            &cancel_id,
                            dirty(),
                        );
                        if confirm {
                            pending_cancel.set(true);
                        } else if return_to_previous_detail() {
                            navigator.go_back();
                        } else {
                            navigator.push(destination);
                        }
                    },
                    "Cancel"
                }
            },
            context_label: "Entity identity",
            context: rsx! {
                div { class: "semantic-edit-entity__identity",
                    dl {
                                div {
                                    dt { "Collection" }
                                    dd {
                                        Link {
                                            to: return_route.clone(),
                                            code { "{collection_label}" }
                                        }
                                    }
                                }
                                div {
                                    dt { "ID" }
                                    dd {
                                        Link {
                                            to: detail_route.clone(),
                                            code { "{id}" }
                                        }
                                    }
                                }
                        if let Some(class_name) = class_name {
                            div {
                                dt { "Class" }
                                dd { "{class_name}" }
                            }
                        }
                    }
                    p {
                        class: "semantic-edit-entity__status",
                        role: "status",
                        aria_live: "polite",
                        "{status_message}"
                    }
                }
            },
            if loading && object.is_none() {
                LoadingSkeleton {
                    label: "Loading entity form",
                    line_count: 7,
                }
            } else if object.is_none() {
                if let Some(error) = error.clone() {
                    ErrorState {
                        title: "Could not load this entity",
                        message: "The edit form is temporarily unavailable.",
                        details: error,
                        retry_label: "Retry",
                        on_retry: move |_| resource.restart(),
                    }
                    div { class: "semantic-edit-entity__recovery",
                        Link {
                            class: "semantic-button-link semantic-button-link--secondary",
                            to: return_route.clone(),
                            "Return to entities"
                        }
                    }
                } else if not_found {
                    EmptyState {
                        title: "Entity not found",
                        description: "It may have been deleted, moved to another collection, or opened from an outdated link.",
                        action_label: "Return to entities",
                        on_action: move |_| {
                            navigator.push(return_route.clone());
                        },
                    }
                }
            } else if let Some(object) = object {
                if loading {
                    RefreshingIndicator { label: "Refreshing entity data" }
                }
                if error.is_some() {
                    InlineNotice {
                        variant: NoticeVariant::Error,
                        title: "Refresh failed",
                        message: "Your current form has been kept. You can retry without losing the draft.",
                        action_label: "Retry",
                        on_action: move |_| resource.restart(),
                    }
                }
                if let Some(class) = class {
                    {
                        let primary_id_field = primary_id_field_for_collection(&catalog, &collection_label);
                        rsx! {
                            div {
                                class: "semantic-edit-entity__form",
                                key: "{form_key}",
                                DynamicClassForm {
                                    class,
                                    object: object.as_ref().clone(),
                                    mode: SemanticFormMode::Edit,
                                    collection: collection.clone(),
                                    id: Some(id.clone()),
                                    scope_id: scope_id.clone(),
                                    submit: Some(rpc_batch_upsert_submit_handler_with_primary_id(
                                        client.clone(),
                                        scope_id.clone(),
                                        collection_label.clone(),
                                        id.clone(),
                                        primary_id_field,
                                    )),
                                    action_labels: SemanticFormActionLabels::save_changes(),
                                    on_dirty_change: move |next_dirty| dirty.set(next_dirty),
                                    on_submitting_change: move |next_submitting| {
                                        submitting.set(next_submitting);
                                        if next_submitting {
                                            submit_feedback.set(EditSubmitFeedback::Idle);
                                        }
                                    },
                                    on_submit_success: move |_outcome: SemanticFormSubmitOutcome| {
                                        dirty.set(false);
                                        submit_feedback.set(EditSubmitFeedback::Succeeded);
                                        toast.show(
                                            Toast::success("The saved entity is ready to view.")
                                                .title("Changes saved"),
                                        );
                                        if return_to_previous_detail() {
                                            navigator.go_back();
                                        } else {
                                            navigator.push(detail_route.clone());
                                        }
                                    },
                                    on_submit_failure: move |failure: SemanticFormSubmitFailure| {
                                        let message = failure
                                            .errors
                                            .first()
                                            .map(|error| error.message.clone())
                                            .unwrap_or_else(|| {
                                                "Changes could not be saved. Review the form and retry.".to_string()
                                            });
                                        submit_feedback.set(EditSubmitFeedback::Failed(message));
                                    },
                                }
                            }
                        }
                    }
                } else {
                    EmptyState {
                        title: "No safe editor is available",
                        description: "This entity's class is not registered in the active catalog, so raw editing is disabled.",
                        action_label: "View entity details",
                        on_action: move |_| {
                            if return_to_previous_detail() {
                                navigator.go_back();
                            } else {
                                navigator.push(detail_route.clone());
                            }
                        },
                    }
                    div { class: "semantic-edit-entity__recovery",
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Secondary,
                            r#type: "button",
                            onclick: move |_| reload_catalog.call(()),
                            "Refresh catalog"
                        }
                    }
                }
            }
        }
        UnsavedChangesPrompt {
            open: pending_cancel(),
            target: cancel_target,
            body: "Leaving this edit form will discard the changes you have made.",
            on_open_change: move |open: bool| pending_cancel.set(open),
            on_discard: move |request: ConfirmActionRequest| {
                pending_cancel.set(false);
                if return_to_previous_detail() {
                    navigator.go_back();
                } else {
                    navigator.push(edit_entity_destination(collection.as_deref(), &id));
                }
                request.complete(Ok(()));
            },
        }
    }
}

pub async fn load_entity(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: Option<String>,
    id: String,
) -> std::result::Result<Option<Object>, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    if let Some(collection) = collection {
        payload.insert("collection", Value::String(collection));
    }
    payload.insert(ATTR_ID, Value::String(id));
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

fn entity_destination(collection: &str, id: &str) -> Route {
    if collection == DEFAULT_COLLECTION {
        Route::DefaultEntityPage { id: id.to_string() }
    } else {
        Route::CollectionEntityPage {
            collection: collection.to_string(),
            id: id.to_string(),
        }
    }
}

fn edit_entity_destination(collection: Option<&str>, id: &str) -> Route {
    match collection {
        Some(collection) => Route::CollectionEntityPage {
            collection: collection.to_string(),
            id: id.to_string(),
        },
        None => Route::DefaultEntityPage { id: id.to_string() },
    }
}

fn edit_return_destination(collection: Option<&str>) -> Route {
    match collection {
        Some(collection) => Route::CollectionPage {
            collection: collection.to_string(),
        },
        None => Route::BrowsePage {
            collection: None,
            view: None,
            renderer: None,
            page: None,
            page_size: None,
            filters: None,
            sql: None,
        },
    }
}

fn edit_cancel_action(collection: Option<&str>, id: &str, dirty: bool) -> (Route, bool) {
    (edit_entity_destination(collection, id), dirty)
}

fn edit_route_key(scope_id: Option<&str>, collection: Option<&str>, id: &str) -> String {
    format!("scope={scope_id:?};collection={collection:?};id={id:?}")
}

fn edit_status_message(submitting: bool, dirty: bool, feedback: &EditSubmitFeedback) -> String {
    if submitting {
        "Saving changes…".to_string()
    } else {
        match feedback {
            EditSubmitFeedback::Succeeded => "Changes saved.".to_string(),
            EditSubmitFeedback::Failed(message) => message.clone(),
            EditSubmitFeedback::Idle if dirty => "Unsaved changes".to_string(),
            EditSubmitFeedback::Idle => "No unsaved changes".to_string(),
        }
    }
}

fn cancel_destination(collection: &str) -> Route {
    if collection.trim().is_empty() {
        Route::HomePage
    } else {
        Route::CollectionPage {
            collection: collection.to_string(),
        }
    }
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
    operation.insert(ATTR_ID, Value::String(id));
    operation.insert("object", Value::Object(object));
    operation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_destination_uses_default_and_named_detail_routes() {
        assert_eq!(
            entity_destination(DEFAULT_COLLECTION, "entity-1"),
            Route::DefaultEntityPage {
                id: "entity-1".to_string()
            }
        );
        assert_eq!(
            entity_destination("notes", "entity-2"),
            Route::CollectionEntityPage {
                collection: "notes".to_string(),
                id: "entity-2".to_string(),
            }
        );
        assert_eq!(
            cancel_destination("notes"),
            Route::CollectionPage {
                collection: "notes".to_string(),
            }
        );
        assert_eq!(cancel_destination(""), Route::HomePage);
    }

    #[test]
    fn edit_route_key_covers_scope_explicit_collection_and_id() {
        let default = edit_route_key(Some("scope-a"), None, "entity-1");
        assert_ne!(default, edit_route_key(Some("scope-b"), None, "entity-1"));
        assert_ne!(
            default,
            edit_route_key(Some("scope-a"), Some(DEFAULT_COLLECTION), "entity-1")
        );
        assert_ne!(default, edit_route_key(Some("scope-a"), None, "entity-2"));
    }

    #[test]
    fn edit_destination_preserves_default_and_explicit_named_routes() {
        assert_eq!(
            edit_entity_destination(None, "entity-1"),
            Route::DefaultEntityPage {
                id: "entity-1".to_string(),
            }
        );
        assert_eq!(
            edit_entity_destination(Some("notes"), "entity-2"),
            Route::CollectionEntityPage {
                collection: "notes".to_string(),
                id: "entity-2".to_string(),
            }
        );
        assert_eq!(
            edit_entity_destination(Some(DEFAULT_COLLECTION), "entity-3"),
            Route::CollectionEntityPage {
                collection: DEFAULT_COLLECTION.to_string(),
                id: "entity-3".to_string(),
            }
        );
        assert_eq!(
            edit_return_destination(Some("notes")),
            Route::CollectionPage {
                collection: "notes".to_string(),
            }
        );
        assert_eq!(
            edit_return_destination(Some(DEFAULT_COLLECTION)),
            Route::CollectionPage {
                collection: DEFAULT_COLLECTION.to_string(),
            }
        );
    }

    #[test]
    fn edit_cancel_preserves_a_deterministic_detail_fallback() {
        let (clean_destination, clean_confirm) = edit_cancel_action(None, "entity-1", false);
        assert!(!clean_confirm);
        assert_eq!(
            clean_destination,
            Route::DefaultEntityPage {
                id: "entity-1".to_string(),
            }
        );

        let (dirty_destination, dirty_confirm) =
            edit_cancel_action(Some("notes"), "entity-2", true);
        assert!(dirty_confirm);
        assert_eq!(
            dirty_destination,
            Route::CollectionEntityPage {
                collection: "notes".to_string(),
                id: "entity-2".to_string(),
            }
        );
    }
}
