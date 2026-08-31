use dioxus::prelude::*;
use semantic_ui_core::{EntityTarget, use_ui_catalog_context};

use super::{CopyRequest, CopyableCode, PageHeader};
use crate::{app::entity_edit_route, views::Route};

/// Route-aware identity header shared by default and named entity pages.
///
/// The header owns navigation affordances and copy behavior. Registered entity
/// actions remain a slot so the catalog continues to decide domain behavior.
#[component]
pub fn EntityPageHeader(
    target: EntityTarget,
    title: String,
    #[props(default)] class_name: Option<String>,
    #[props(default)] registered_actions: Option<Element>,
    #[props(default)] refreshing: bool,
    #[props(default = true)] editable: bool,
    on_refresh: EventHandler<()>,
) -> Element {
    let href = use_ui_catalog_context()
        .catalog_signal()
        .read()
        .as_ref()
        .and_then(|catalog| catalog.entity_navigation().href.as_ref())
        .and_then(|builder| builder(&target));
    let edit_route = entity_edit_route(&target);
    let collection = target.collection.clone();

    rsx! {
        div { class: "semantic-entity-page-header",
            PageHeader {
                title: title.clone(),
                description: class_name.clone().map(|name| format!("{name} entity")),
                breadcrumbs: rsx! {
                    Link { to: Route::HomePage, "Workspace" }
                    span { aria_hidden: "true", "/" }
                    Link {
                        to: Route::BrowsePage {
                            collection: None,
                            view: None,
                            renderer: None,
                            page: None,
                            page_size: None,
                            filters: None,
                            sql: None,
                        },
                        "Browse"
                    }
                    span { aria_hidden: "true", "/" }
                    if let Some(collection) = collection.clone() {
                        Link {
                            to: Route::CollectionPage {
                                collection: collection.clone(),
                            },
                            "{collection}"
                        }
                    } else {
                        span { "Entities" }
                    }
                    span { aria_hidden: "true", "/" }
                    span {
                        aria_current: "page",
                        aria_label: "Entity {title}, ID {target.id}",
                        "{title}"
                    }
                },
                actions: rsx! {
                    if editable {
                        Link {
                            class: "semantic-button-link semantic-entity-page-header__edit",
                            to: edit_route,
                            "Edit"
                        }
                    }
                    dxcomp::Button {
                        variant: dxcomp::ButtonVariant::Outline,
                        disabled: refreshing,
                        aria_busy: refreshing,
                        onclick: move |_| on_refresh.call(()),
                        if refreshing { "Refreshing…" } else { "Refresh" }
                    }
                    if let Some(actions) = registered_actions {
                        div { class: "semantic-entity-page-header__registered-actions", {actions} }
                    }
                }
            }
            div { class: "semantic-entity-page-header__metadata", aria_label: "Entity identity",
                if let Some(collection) = collection {
                    span {
                        class: "semantic-entity-page-header__collection",
                        title: "Collection: {collection}",
                        "Collection: {collection}"
                    }
                }
                if let Some(class_name) = class_name {
                    span { class: "semantic-entity-page-header__class", "{class_name}" }
                }
                CopyableCode {
                    value: target.id.clone(),
                    label: "entity ID",
                    on_copy: copy_handler(false),
                }
                if let Some(href) = href {
                    CopyableCode {
                        value: href,
                        label: "entity link",
                        on_copy: copy_handler(true),
                    }
                }
            }
        }
    }
}

fn copy_handler(absolute_url: bool) -> EventHandler<CopyRequest> {
    EventHandler::new(move |request: CopyRequest| copy_value(request, absolute_url))
}

fn copy_value(request: CopyRequest, absolute_url: bool) {
    let value = request.value().to_string();
    spawn(async move {
        let eval = document::eval(
            r#"
                const [rawValue, absolute] = await dioxus.recv();
                const value = absolute
                    ? new URL(rawValue, window.location.href).href
                    : rawValue;
                await navigator.clipboard.writeText(value);
                return true;
            "#,
        );
        let result = match eval.send((value, absolute_url)) {
            Ok(()) => eval.await.map(|_| ()).map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        request.complete(result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_edit_destination_preserves_collection_identity() {
        let default = EntityTarget::default_collection("one");
        let named = EntityTarget::new(Some("photos".to_string()), "two");

        assert_eq!(
            entity_edit_route(&default),
            Route::DefaultEditEntityPage {
                id: "one".to_string(),
            }
        );
        assert_eq!(
            entity_edit_route(&named),
            Route::CollectionEditEntityPage {
                collection: "photos".to_string(),
                id: "two".to_string(),
            }
        );
    }
}
