use semantic_data::attr::AttrDescriptorConst;
use semantic_data::query::AttrEntityObject;
use std::rc::Rc;

use dioxus::prelude::*;
use futures::FutureExt;
use semantic_data::{
    builtin::{ATTR_ID, DEFAULT_COLLECTION},
    schema::ClassType,
    value::{Object, Value},
};
use semantic_ui_core::{
    DynamicClassForm, EntityTarget, FormRoot, SemanticFormMode, SemanticFormSubmit, SubmitError,
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
    components::{EntityCreateFailure, EntityCreateForm, EntityCreateOutcome, FormPage},
    navigation_guard::{PageNavigationGuard, use_page_navigation_guard},
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
pub fn CreateEntityPage(#[props(default)] initial_class: Option<String>) -> Element {
    let creating_note = initial_class.as_deref() == Some("semantic:base:note");
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let navigator = use_navigator();
    let toast = use_toast_dispatcher();
    let mut dirty = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut selected_collection = use_signal(String::new);
    let page_guard = use_page_navigation_guard();
    let form_handle = use_guarded_form_save(page_guard);
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
            title: if creating_note { "New note" } else { "New entity" },
            description: if creating_note { "Write a document and choose where to save it." } else { "Choose a type and add its details." },
            busy: submitting(),
            breadcrumbs: rsx! {
                Link { to: Route::HomePage, "Workspace" }
                span { aria_hidden: "true", "/" }
                span { if creating_note { "New note" } else { "New entity" } }
            },
            header_actions: rsx! {
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    r#type: "button",
                    disabled: submitting(),
                    onclick: move |_| {
                        navigator.push(cancel_destination(&selected_collection()));
                    },
                    "Cancel"
                }
            },
            EntityCreateForm {
                submit,
                initial_class,
                on_collection_change: move |collection| selected_collection.set(collection),
                on_dirty_change: move |next| {
                    dirty.set(next);
                    page_guard.set_dirty(next);
                },
                on_submitting_change: move |next| submitting.set(next),
                on_form_ready: form_handle,
                on_created: move |outcome: EntityCreateOutcome| {
                    dirty.set(false);
                    toast.show(
                        Toast::success("The entity is ready to view.").title("Entity created"),
                    );
                    if !page_guard.finish_save() {
                        navigator.push(entity_destination(&outcome.collection, &outcome.id));
                    }
                },
                on_failure: move |_failure: EntityCreateFailure| page_guard.save_failed(),
            }
        }
    }
}

#[component]
pub fn CreateNotePage() -> Element {
    rsx! { CreateEntityPage { initial_class: Some("semantic:base:note".to_string()) } }
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
    let page_guard = use_page_navigation_guard();
    let form_handle = use_guarded_form_save(page_guard);
    let navigator = use_navigator();
    let toast = use_toast_dispatcher();
    let class = object
        .as_ref()
        .and_then(|object| catalog.object_class(object.as_ref()).cloned());
    let class_name = class.as_ref().map(class_label);
    let is_note = class
        .as_ref()
        .is_some_and(|class| class.id == "semantic:base:note");
    let status_message = edit_status_message(submitting(), dirty(), &submit_feedback.read());
    let form_key = edit_route_key(scope_id.as_deref(), collection.as_deref(), &id);
    let cancel_route = detail_route.clone();

    rsx! {
        FormPage {
            title: if is_note { "Edit document" } else { "Edit entity" },
            description: if is_note { None } else { Some("Update schema-backed fields while keeping the entity identity fixed.".to_string()) },
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
                        if return_to_previous_detail() {
                            navigator.go_back();
                        } else {
                            navigator.push(cancel_route.clone());
                        }
                    },
                    "Cancel"
                }
            },
            context_label: "Entity identity",
            context: rsx! {
                if is_note {
                    details { class: "semantic-edit-entity__identity-details",
                        summary { "Document location and identity" }
                        EntityIdentity {
                            collection_label: collection_label.clone(),
                            id: id.clone(),
                            class_name: class_name.clone(),
                            return_route: return_route.clone(),
                            detail_route: detail_route.clone(),
                            status_message: status_message.clone(),
                        }
                    }
                } else {
                    EntityIdentity {
                        collection_label: collection_label.clone(),
                        id: id.clone(),
                        class_name: class_name.clone(),
                        return_route: return_route.clone(),
                        detail_route: detail_route.clone(),
                        status_message: status_message.clone(),
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
                                    on_dirty_change: move |next_dirty| {
                                        dirty.set(next_dirty);
                                        page_guard.set_dirty(next_dirty);
                                    },
                                    on_form_ready: form_handle,
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
                                        if page_guard.finish_save() {
                                            return;
                                        }
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
                                        page_guard.save_failed();
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
    }
}

/// Lets the navigation guard prompt save the page's form. Returns the handler
/// to pass as the form's `on_form_ready`.
fn use_guarded_form_save(page_guard: PageNavigationGuard) -> EventHandler<FormRoot<Value>> {
    let mut form = use_signal(|| None::<FormRoot<Value>>);
    let save = use_callback(move |()| {
        if let Some(form) = form.peek().clone() {
            spawn(async move {
                let _ = form.submit().await;
            });
        }
    });
    use_hook(|| page_guard.set_save_handler(save));
    EventHandler::new(move |next: FormRoot<Value>| form.set(Some(next)))
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
        Value::Object(mut response) => match response.remove(AttrEntityObject::ID) {
            Some(Value::Object(object)) => Ok(Some(object)),
            _ => Err("get response missing object".to_string()),
        },
        _ => Err("get response must be an object or null".to_string()),
    }
}

#[component]
fn EntityIdentity(
    collection_label: String,
    id: String,
    class_name: Option<String>,
    return_route: Route,
    detail_route: Route,
    status_message: String,
) -> Element {
    rsx! {
        div { class: "semantic-edit-entity__identity",
            dl {
                div {
                    dt { "Collection" }
                    dd { Link { to: return_route, code { "{collection_label}" } } }
                }
                div {
                    dt { "ID" }
                    dd { Link { to: detail_route, code { "{id}" } } }
                }
                if let Some(class_name) = class_name {
                    div { dt { "Class" } dd { "{class_name}" } }
                }
            }
            p {
                class: "semantic-edit-entity__status",
                role: "status",
                aria_live: "polite",
                "{status_message}"
            }
        }
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
}
