use std::{
    rc::Rc,
    sync::atomic::{AtomicU32, Ordering},
};

use dioxus::prelude::*;
use futures::FutureExt;
use semantic_data::{
    builtin::{ATTR_ID, DEFAULT_COLLECTION},
    schema::ClassType,
    value::{Object, Value},
};
use semantic_ui_core::{
    DynamicClassForm, SemanticFormMode, SemanticFormSubmit, SubmitError,
    components::{
        EmptyState, ErrorState, InlineNotice, LoadingSkeleton, NoticeVariant, RefreshingIndicator,
    },
    context::{Toast, use_toast_dispatcher},
    default_value_for_class,
    form::{SemanticFormActionLabels, SemanticFormSubmitFailure, SemanticFormSubmitOutcome},
    rpc_batch_upsert_submit_handler_with_primary_id, use_active_scope_id, use_rpc_client,
    use_ui_catalog, use_ui_catalog_context, use_ui_catalog_reload,
};

use crate::{
    components::{ConfirmActionRequest, FormPage, UnsavedChangesPrompt},
    views::Route,
};

static ENTITY_ID_SEQUENCE: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Debug, Default, PartialEq)]
struct CreateEntityCatalogOptions {
    classes: Rc<[ClassType]>,
    collections: Rc<[String]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorKind {
    Class,
    Collection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingSelection {
    kind: SelectorKind,
    value: String,
}

impl PendingSelection {
    fn target_label(&self) -> String {
        match self.kind {
            SelectorKind::Class => format!("class `{}`", self.value),
            SelectorKind::Collection => format!("collection `{}`", self.value),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum PendingCreateAction {
    Selection(PendingSelection),
    Cancel(Route),
}

impl PendingCreateAction {
    fn target_label(&self) -> String {
        match self {
            Self::Selection(selection) => selection.target_label(),
            Self::Cancel(_) => "entity list".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SelectorTransition {
    Unchanged,
    Apply(PendingSelection),
    Confirm(PendingSelection),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum CreateSubmitFeedback {
    #[default]
    Idle,
    Succeeded,
    Failed(String),
}

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

fn selector_transition(
    kind: SelectorKind,
    current: &str,
    next: String,
    dirty: bool,
) -> SelectorTransition {
    if current == next {
        SelectorTransition::Unchanged
    } else {
        let selection = PendingSelection { kind, value: next };
        if dirty {
            SelectorTransition::Confirm(selection)
        } else {
            SelectorTransition::Apply(selection)
        }
    }
}

#[component]
pub fn CreateEntityPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let catalog_signal = use_ui_catalog_context().catalog_signal();
    let reload_catalog = use_ui_catalog_reload().reload;
    let catalog_options = use_memo(move || {
        catalog_signal
            .read()
            .as_ref()
            .map(|catalog| CreateEntityCatalogOptions {
                classes: catalog.classes().cloned().collect::<Vec<_>>().into(),
                collections: catalog
                    .collections()
                    .map(|collection| collection.name.clone())
                    .collect::<Vec<_>>()
                    .into(),
            })
            .unwrap_or_default()
    });
    let classes = catalog_options.read().classes.clone();
    let collections = catalog_options.read().collections.clone();
    let initial_class_id = classes.first().map(|class| class.id.clone());
    let initial_collection = collections.first().cloned();
    let mut selected_class_id = use_signal(move || initial_class_id);
    let mut selected_collection = use_signal(move || initial_collection);
    let mut draft_id = use_signal(new_entity_id);
    let mut dirty = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut submit_feedback = use_signal(CreateSubmitFeedback::default);
    let mut pending_action = use_signal(|| None::<PendingCreateAction>);
    let navigator = use_navigator();
    let toast = use_toast_dispatcher();

    let class_id = selected_class_id.read().clone().unwrap_or_default();
    let collection = selected_collection.read().clone().unwrap_or_default();
    let selected_class = classes.iter().find(|class| class.id == class_id).cloned();
    let primary_id_field = primary_id_field_for_collection(&catalog, &collection);
    let current_status = if submitting() {
        "Creating entity…".to_string()
    } else {
        match &*submit_feedback.read() {
            CreateSubmitFeedback::Failed(message) => message.clone(),
            CreateSubmitFeedback::Succeeded => "Entity created.".to_string(),
            CreateSubmitFeedback::Idle if dirty() => "Unsaved changes".to_string(),
            CreateSubmitFeedback::Idle => "Ready to create".to_string(),
        }
    };
    let apply_selection = Callback::new(move |selection: PendingSelection| {
        match selection.kind {
            SelectorKind::Class => selected_class_id.set(Some(selection.value)),
            SelectorKind::Collection => selected_collection.set(Some(selection.value)),
        }
        draft_id.set(new_entity_id());
        dirty.set(false);
        submitting.set(false);
        submit_feedback.set(CreateSubmitFeedback::Idle);
    });
    let pending_target = pending_action
        .read()
        .as_ref()
        .map(PendingCreateAction::target_label)
        .unwrap_or_else(|| "another destination".to_string());
    let cancel_collection = collection.clone();
    let picker_collection = collection.clone();

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
                        let destination = cancel_destination(&cancel_collection);
                        if dirty() {
                            pending_action.set(Some(PendingCreateAction::Cancel(destination)));
                        } else {
                            navigator.push(destination);
                        }
                    },
                    "Cancel"
                }
            },
            context_label: "New entity destination",
            context: rsx! {
                div { class: "semantic-create-entity__context",
                    div { class: "semantic-create-entity__picker",
                        label { class: "semantic-create-entity__picker-label", "Class" }
                        p { class: "semantic-create-entity__picker-help", "Controls the fields and validation shown below." }
                        if !classes.is_empty() {
                            dxcomp::Combobox::<String> {
                                value: Some(selected_class_id.into()),
                                on_value_change: move |value| {
                                    if let Some(value) = value {
                                        match selector_transition(SelectorKind::Class, &class_id, value, dirty()) {
                                            SelectorTransition::Unchanged => {}
                                            SelectorTransition::Apply(selection) => apply_selection.call(selection),
                                            SelectorTransition::Confirm(selection) => pending_action.set(
                                                Some(PendingCreateAction::Selection(selection)),
                                            ),
                                        }
                                    }
                                },
                                disabled: submitting(),
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
                    div { class: "semantic-create-entity__picker",
                        label { class: "semantic-create-entity__picker-label", "Collection" }
                        p { class: "semantic-create-entity__picker-help", "Determines where the entity is stored and its detail route." }
                        if !collections.is_empty() {
                            dxcomp::Combobox::<String> {
                                value: Some(selected_collection.into()),
                                on_value_change: move |value| {
                                    if let Some(value) = value {
                                        match selector_transition(
                                            SelectorKind::Collection,
                                            &picker_collection,
                                            value,
                                            dirty(),
                                        ) {
                                            SelectorTransition::Unchanged => {}
                                            SelectorTransition::Apply(selection) => apply_selection.call(selection),
                                            SelectorTransition::Confirm(selection) => pending_action.set(
                                                Some(PendingCreateAction::Selection(selection)),
                                            ),
                                        }
                                    }
                                },
                                disabled: submitting(),
                                placeholder: "Filter collections",
                                aria_label: "Entity collection",
                                list_aria_label: "Entity collections",
                                dxcomp::ComboboxEmpty { "No collection found." }
                                for (index, option) in collections.iter().enumerate() {
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
                        "{current_status}"
                    }
                }
            },
            if classes.is_empty() {
                EmptyState {
                    title: "No entity classes available",
                    description: "The active catalog does not define a class that can supply this form.",
                    action_label: "Refresh catalog",
                    on_action: move |_| reload_catalog.call(()),
                }
            } else if collections.is_empty() {
                EmptyState {
                    title: "No collections available",
                    description: "Add or load a collection before creating an entity.",
                    action_label: "Refresh catalog",
                    on_action: move |_| reload_catalog.call(()),
                }
            } else if let Some(class) = selected_class {
                {
                    let mut object = match default_value_for_class(&class, &catalog) {
                        Value::Object(object) => object,
                        _ => Object::new(),
                    };
                    object.insert("type", Value::String(class.id.clone()));
                    object.insert(primary_id_field.clone(), Value::String(draft_id()));
                    let submit = rpc_batch_upsert_submit_handler_from_primary_id_field(
                        client.clone(),
                        scope_id.clone(),
                        collection.clone(),
                        primary_id_field.clone(),
                    );
                    rsx! {
                        div {
                            class: "semantic-create-entity__form",
                            key: "{class.id}:{collection}:{draft_id}",
                            DynamicClassForm {
                                class: class.clone(),
                                object,
                                mode: SemanticFormMode::Create,
                                collection: Some(collection.clone()),
                                id: None,
                                scope_id: scope_id.clone(),
                                submit: Some(submit),
                                action_labels: SemanticFormActionLabels::create_entity(),
                                on_dirty_change: move |next_dirty| dirty.set(next_dirty),
                                on_submitting_change: move |next_submitting| {
                                    submitting.set(next_submitting);
                                    if next_submitting {
                                        submit_feedback.set(CreateSubmitFeedback::Idle);
                                    }
                                },
                                on_submit_success: move |outcome: SemanticFormSubmitOutcome| {
                                    let Value::Object(object) = outcome.value else {
                                        submit_feedback.set(CreateSubmitFeedback::Failed(
                                            "Created entity response did not contain an object.".to_string(),
                                        ));
                                        return;
                                    };
                                    match submitted_primary_id(&object, &primary_id_field) {
                                        Ok(submitted_id) => {
                                            submit_feedback.set(CreateSubmitFeedback::Succeeded);
                                            dirty.set(false);
                                            toast.show(
                                                Toast::success("The entity is ready to view.")
                                                    .title("Entity created"),
                                            );
                                            navigator.push(entity_destination(&collection, &submitted_id));
                                        }
                                        Err(error) => submit_feedback.set(CreateSubmitFeedback::Failed(
                                            error.message.unwrap_or_else(|| {
                                                "Created entity did not contain a usable ID.".to_string()
                                            }),
                                        )),
                                    }
                                },
                                on_submit_failure: move |failure: SemanticFormSubmitFailure| {
                                    let message = failure
                                        .errors
                                        .first()
                                        .map(|error| error.message.clone())
                                        .unwrap_or_else(|| "Entity creation failed. Review the form and retry.".to_string());
                                    submit_feedback.set(CreateSubmitFeedback::Failed(message));
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
        }
        UnsavedChangesPrompt {
            open: pending_action.read().is_some(),
            target: pending_target,
            body: "Changing destination or leaving this page will discard the current entity draft.",
            on_open_change: move |open: bool| {
                if !open {
                    pending_action.set(None);
                }
            },
            on_discard: move |request: ConfirmActionRequest| {
                let action = pending_action.peek().clone();
                pending_action.set(None);
                match action {
                    Some(PendingCreateAction::Selection(selection)) => apply_selection.call(selection),
                    Some(PendingCreateAction::Cancel(destination)) => {
                        navigator.push(destination);
                    }
                    None => {}
                }
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
                                        navigator.push(detail_route.clone());
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
                            navigator.push(detail_route.clone());
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
                navigator.push(edit_entity_destination(collection.as_deref(), &id));
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

fn new_entity_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let sequence = ENTITY_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    new_entity_id_from_parts(nanos, sequence)
}

fn new_entity_id_from_parts(nanos: u128, sequence: u32) -> String {
    format!("entity-{nanos:032x}-{sequence:08x}")
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
    operation.insert(ATTR_ID, Value::String(id));
    operation.insert("object", Value::Object(object));
    operation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_entity_id_parts_have_stable_collision_resistant_format() {
        assert_eq!(
            new_entity_id_from_parts(0x1234, 0x2a),
            "entity-00000000000000000000000000001234-0000002a"
        );
        assert_ne!(
            new_entity_id_from_parts(0x1234, 0x2a),
            new_entity_id_from_parts(0x1234, 0x2b)
        );
    }

    #[test]
    fn selector_change_requires_confirmation_only_for_a_dirty_draft() {
        let pending = PendingSelection {
            kind: SelectorKind::Class,
            value: "semantic:Note".to_string(),
        };
        assert_eq!(
            selector_transition(
                SelectorKind::Class,
                "semantic:Document",
                pending.value.clone(),
                false,
            ),
            SelectorTransition::Apply(pending.clone())
        );
        assert_eq!(
            selector_transition(
                SelectorKind::Class,
                "semantic:Document",
                pending.value.clone(),
                true,
            ),
            SelectorTransition::Confirm(pending)
        );
        assert_eq!(
            selector_transition(SelectorKind::Collection, "notes", "notes".to_string(), true,),
            SelectorTransition::Unchanged
        );
    }

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
    fn edit_cancel_only_prompts_for_a_dirty_form_and_never_uses_history() {
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
