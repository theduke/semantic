use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{
    EntityActionPlacement, EntityCard, EntityDisplayRenderer, EntityRenderOptions, EntityTarget,
    components::{EmptyState, ErrorState, LoadingSkeleton, RefreshingIndicator},
    context::{Toast, use_toast_dispatcher},
    use_active_scope_id, use_rpc_client,
};

use crate::{
    app::{entity_edit_route, entity_route, use_entity_edit_navigation},
    views::Route,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct EntityQueryKey {
    scope_id: Option<String>,
    collection: Option<String>,
    id: String,
}

#[derive(Clone)]
struct EntityQueryResponse {
    key: EntityQueryKey,
    result: std::result::Result<Option<Rc<Object>>, String>,
}

#[component]
pub fn DefaultEntityPage(id: String) -> Element {
    rsx! {
        EntityPageView {
            collection: None,
            id
        }
    }
}

#[component]
pub fn CollectionEntityPage(collection: String, id: String) -> Element {
    rsx! {
        EntityPageView {
            collection: Some(collection),
            id
        }
    }
}

#[component]
fn EntityPageView(collection: Option<String>, id: String) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let toast = use_toast_dispatcher();
    let edit_navigation = use_entity_edit_navigation();
    let target = EntityTarget::new(collection.clone(), id.clone());
    let query_key = use_memo(use_reactive(
        &EntityQueryKey {
            scope_id,
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
                EntityQueryResponse { key, result }
            }
        }
    });
    let current_key = query_key();
    let loading = *resource.state().read() == UseResourceState::Pending;
    let response = resource.read().clone();
    let current_response = response
        .as_ref()
        .filter(|response| response.key == current_key);
    let object = current_response
        .and_then(|response| response.result.as_ref().ok())
        .and_then(|object| object.clone());
    let error = (!loading)
        .then(|| current_response)
        .flatten()
        .and_then(|response| response.result.as_ref().err().cloned());
    let not_found = !loading
        && current_response
            .and_then(|response| response.result.as_ref().ok())
            .is_some_and(Option::is_none);
    let target_key = entity_route(&target).to_string();
    let return_route = entity_return_route(&target);

    rsx! {
        section {
            key: "{target_key}",
            class: "semantic-page semantic-entity semantic-entity-page",
            aria_busy: loading,

            if loading && object.is_none() {
                LoadingSkeleton {
                    label: "Loading entity details",
                    line_count: 7,
                }
            } else if let Some(error) = error {
                ErrorState {
                    title: "Could not load this entity",
                    message: "The entity details are temporarily unavailable.",
                    details: error,
                    retry_label: "Retry",
                    on_retry: move |_| resource.restart(),
                }
                div { class: "semantic-entity-page__recovery",
                    Link {
                        class: "semantic-button-link semantic-button-link--secondary",
                        to: return_route.clone(),
                        "Return to records"
                    }
                }
            } else if not_found {
                EmptyState {
                    title: "Entity not found",
                    description: "It may have been deleted, moved to another collection, or opened from an outdated link.",
                    action_label: "Return to records",
                    on_action: move |_| {
                        navigator().push(return_route.clone());
                    },
                }
            } else if let Some(object) = object {
                if loading {
                    RefreshingIndicator { label: "Refreshing entity details" }
                }
                EntityCard {
                    object: object.as_ref().clone(),
                    options: EntityRenderOptions {
                        collection: target.collection.clone(),
                        id: Some(target.id.clone()),
                        renderer: EntityDisplayRenderer::Custom,
                        preview: false,
                        actions: true,
                    },
                    action_placement: EntityActionPlacement::Detail,
                    excluded_action_ids: vec!["open".to_string()],
                    on_edit: move |target: EntityTarget| {
                        edit_navigation.begin_from_detail(target.clone());
                        navigator().push(entity_edit_route(&target));
                    },
                    on_delete: move |_deleted_target: EntityTarget| {
                        toast.show(
                            Toast::success("The entity was permanently deleted.")
                                .title("Entity deleted"),
                        );
                        navigator().go_back();
                    },
                }
            }
        }
    }
}

fn entity_return_route(target: &EntityTarget) -> Route {
    match &target.collection {
        Some(collection) => Route::CollectionPage {
            collection: collection.clone(),
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

async fn load_entity(
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
    payload.insert("id", Value::String(id));
    let response = client
        .invoke_value("semantic.db.get", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())?;
    match response {
        Value::Null | Value::Void => Ok(None),
        Value::Object(mut response) => match response.remove("object") {
            Some(Value::Object(mut object)) => {
                if !object.contains_key("id")
                    && let Some(Value::String(id)) = response.remove("id")
                {
                    object.insert("id", Value::String(id));
                }
                Ok(Some(object))
            }
            _ => Err("get response missing object".to_string()),
        },
        _ => Err("get response must be an object or null".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_recovery_never_depends_on_browser_history() {
        let default = entity_return_route(&EntityTarget::default_collection("one"));
        let named = entity_return_route(&EntityTarget::new(Some("photos".to_string()), "two"));
        let explicit_default = entity_return_route(&EntityTarget::new(
            Some(semantic_data::builtin::DEFAULT_COLLECTION.to_string()),
            "three",
        ));

        assert!(matches!(default, Route::BrowsePage { .. }));
        assert_eq!(
            named,
            Route::CollectionPage {
                collection: "photos".to_string()
            }
        );
        assert_eq!(
            explicit_default,
            Route::CollectionPage {
                collection: semantic_data::builtin::DEFAULT_COLLECTION.to_string(),
            }
        );
    }

    #[test]
    fn entity_query_key_includes_scope_collection_and_id() {
        let first = EntityQueryKey {
            scope_id: Some("scope-a".to_string()),
            collection: None,
            id: "entity-1".to_string(),
        };
        let mut second = first.clone();
        second.scope_id = Some("scope-b".to_string());
        assert_ne!(first, second);
        second = first.clone();
        second.collection = Some("photos".to_string());
        assert_ne!(first, second);
        second = first.clone();
        second.id = "entity-2".to_string();
        assert_ne!(first, second);
    }
}
