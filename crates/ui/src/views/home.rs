use std::rc::Rc;

use dioxus::prelude::*;
use semantic_ui_core::{
    components::EmptyState, use_ui_catalog, use_ui_catalog_context, use_ui_catalog_reload,
};

use crate::{components::PageHeader, views::Route};

const INITIAL_VISIBLE_COLLECTIONS: usize = 12;

#[derive(Clone, PartialEq)]
struct CollectionRow {
    name: String,
    field_summary: String,
}

#[component]
pub fn HomePage() -> Element {
    let catalog = use_ui_catalog();
    let catalog_signal = use_ui_catalog_context().catalog_signal();
    let reload_catalog = use_ui_catalog_reload().reload;
    let collection_count = catalog.collections().count();
    let class_count = catalog.classes().count();
    let attribute_count = catalog.attributes().count();
    let has_collections = collection_count > 0;
    let can_create_entity = has_collections && class_count > 0;

    let mut collection_search = use_signal(String::new);
    let mut show_all_collections = use_signal(|| false);
    let filtered_collections = use_memo(move || -> Rc<[CollectionRow]> {
        let query = collection_search.read().trim().to_lowercase();
        let catalog = catalog_signal.read();
        let Some(catalog) = catalog.as_ref() else {
            return Vec::new().into();
        };
        catalog
            .collections()
            .filter(|collection| collection_matches(&collection.name, &query))
            .map(|collection| CollectionRow {
                name: collection.name.clone(),
                field_summary: format!(
                    "{} {}",
                    collection.field_ids.len(),
                    pluralize(collection.field_ids.len(), "field", "fields")
                ),
            })
            .collect::<Vec<_>>()
            .into()
    });

    let collection_rows = filtered_collections();
    let filtered_count = collection_rows.len();
    let visible_count = visible_collection_count(filtered_count, show_all_collections());
    let hidden_count = filtered_count.saturating_sub(visible_count);
    let search_value = collection_search();
    let has_search = !search_value.trim().is_empty();
    let result_status = if has_search {
        format!(
            "{} {} found",
            filtered_count,
            pluralize(filtered_count, "collection", "collections")
        )
    } else {
        format!(
            "{} {} available",
            collection_count,
            pluralize(collection_count, "collection", "collections")
        )
    };

    rsx! {
        div { class: "semantic-page semantic-home",
            PageHeader {
                title: "Workspace",
                description: Some(
                    "Explore the loaded catalog, open a collection, or start a data workflow."
                        .to_string(),
                ),
                actions: Some(rsx! {
                    Link {
                        to: Route::CatalogPage,
                        class: "dx-button semantic-home__quick-action",
                        "data-style": "outline",
                        "data-size": "default",
                        "View catalog"
                    }
                    if has_collections {
                        Link {
                            to: Route::BrowsePage {
                                collection: None,
                                view: None,
                                renderer: None,
                                page: None,
                                page_size: None,
                                sql: None,
                            },
                            class: "dx-button semantic-home__quick-action",
                            "data-style": "outline",
                            "data-size": "default",
                            "Browse data"
                        }
                        Link {
                            to: Route::UploadPage,
                            class: "dx-button semantic-home__quick-action",
                            "data-style": "outline",
                            "data-size": "default",
                            "Upload files"
                        }
                    }
                    if can_create_entity {
                        Link {
                            to: Route::CreateEntityPage,
                            class: "dx-button semantic-home__quick-action",
                            "data-style": "primary",
                            "data-size": "default",
                            "Create entity"
                        }
                    }
                }),
            }

            section { class: "semantic-home__summary", aria_label: "Catalog summary",
                Link {
                    to: Route::CatalogPage,
                    class: "semantic-home__metric semantic-surface",
                    aria_label: "View {collection_count} collections in the catalog",
                    span { class: "semantic-home__metric-value", "{collection_count}" }
                    span { class: "semantic-home__metric-label",
                        {pluralize(collection_count, "Collection", "Collections")}
                    }
                    span { class: "semantic-home__metric-action", aria_hidden: "true", "View catalog →" }
                }
                Link {
                    to: Route::CatalogPage,
                    class: "semantic-home__metric semantic-surface",
                    aria_label: "View {class_count} classes in the catalog",
                    span { class: "semantic-home__metric-value", "{class_count}" }
                    span { class: "semantic-home__metric-label",
                        {pluralize(class_count, "Class", "Classes")}
                    }
                    span { class: "semantic-home__metric-action", aria_hidden: "true", "View catalog →" }
                }
                Link {
                    to: Route::CatalogPage,
                    class: "semantic-home__metric semantic-surface",
                    aria_label: "View {attribute_count} attributes in the catalog",
                    span { class: "semantic-home__metric-value", "{attribute_count}" }
                    span { class: "semantic-home__metric-label",
                        {pluralize(attribute_count, "Attribute", "Attributes")}
                    }
                    span { class: "semantic-home__metric-action", aria_hidden: "true", "View catalog →" }
                }
            }

            section {
                id: "home-collections",
                class: "semantic-section semantic-home__collections",
                aria_labelledby: "home-collections-heading",
                div { class: "semantic-home__section-header",
                    div { class: "semantic-home__section-copy",
                        h2 { id: "home-collections-heading", class: "semantic-section__title", "Collections" }
                        p { "Open a collection to inspect and work with its records." }
                    }
                    if has_collections {
                        label { class: "semantic-home__search", r#for: "home-collection-search",
                            span { "Search collections" }
                            input {
                                id: "home-collection-search",
                                class: "semantic-home__search-input",
                                r#type: "search",
                                value: search_value,
                                placeholder: "Search by name",
                                autocomplete: "off",
                                aria_describedby: "home-collection-results",
                                oninput: move |event: FormEvent| {
                                    collection_search.set(event.value());
                                    show_all_collections.set(false);
                                },
                            }
                        }
                    }
                }

                if !has_collections {
                    EmptyState {
                        title: "No collections in this catalog".to_string(),
                        description: Some(
                            "Refresh the catalog after configuring collections, or inspect the catalog to verify what is loaded."
                                .to_string(),
                        ),
                        action_label: Some("Refresh catalog".to_string()),
                        on_action: move |_| reload_catalog.call(()),
                    }
                } else {
                    p {
                        id: "home-collection-results",
                        class: "semantic-home__results-status",
                        role: "status",
                        aria_live: "polite",
                        "{result_status}"
                    }

                    if filtered_count == 0 {
                        EmptyState {
                            title: "No matching collections".to_string(),
                            description: Some(
                                "Try another name or clear the search to show every collection."
                                    .to_string(),
                            ),
                            action_label: Some("Clear search".to_string()),
                            on_action: move |_| {
                                collection_search.set(String::new());
                                show_all_collections.set(false);
                            },
                        }
                    } else {
                        ul { class: "semantic-home__collection-list",
                            for collection in collection_rows.iter().take(visible_count) {
                                li { key: "{collection.name}",
                                    Link {
                                        to: Route::CollectionPage {
                                            collection: collection.name.clone(),
                                        },
                                        class: "semantic-home__collection-link semantic-surface",
                                        span { class: "semantic-home__collection-name", "{collection.name}" }
                                        span { class: "semantic-home__collection-meta",
                                            "{collection.field_summary}"
                                        }
                                        span {
                                            class: "semantic-home__collection-open",
                                            aria_hidden: "true",
                                            "Open →"
                                        }
                                    }
                                }
                            }
                        }

                        if filtered_count > INITIAL_VISIBLE_COLLECTIONS {
                            div { class: "semantic-home__disclosure",
                                if show_all_collections() {
                                    dxcomp::Button {
                                        variant: dxcomp::ButtonVariant::Outline,
                                        aria_controls: "home-collections",
                                        aria_expanded: true,
                                        onclick: move |_| show_all_collections.set(false),
                                        "Show fewer"
                                    }
                                } else {
                                    dxcomp::Button {
                                        variant: dxcomp::ButtonVariant::Outline,
                                        aria_controls: "home-collections",
                                        aria_expanded: false,
                                        onclick: move |_| show_all_collections.set(true),
                                        "Show {hidden_count} more"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn collection_matches(name: &str, normalized_query: &str) -> bool {
    normalized_query.is_empty() || name.to_lowercase().contains(normalized_query)
}

fn visible_collection_count(filtered_count: usize, show_all: bool) -> usize {
    if show_all {
        filtered_count
    } else {
        filtered_count.min(INITIAL_VISIBLE_COLLECTIONS)
    }
}

fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::{INITIAL_VISIBLE_COLLECTIONS, collection_matches, visible_collection_count};

    #[test]
    fn collection_search_is_case_insensitive() {
        assert!(collection_matches("MediaLibrary", "media"));
        assert!(!collection_matches("Documents", "media"));
        assert!(collection_matches("Anything", ""));
    }

    #[test]
    fn collection_disclosure_caps_only_the_collapsed_list() {
        let total = INITIAL_VISIBLE_COLLECTIONS + 5;

        assert_eq!(
            visible_collection_count(total, false),
            INITIAL_VISIBLE_COLLECTIONS
        );
        assert_eq!(visible_collection_count(total, true), total);
        assert_eq!(visible_collection_count(3, false), 3);
    }
}
