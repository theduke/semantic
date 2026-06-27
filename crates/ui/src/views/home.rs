use dioxus::prelude::*;
use semantic_ui_core::use_ui_catalog;

use crate::views::Route;

#[component]
pub fn HomePage() -> Element {
    let catalog = use_ui_catalog();
    let collections: Vec<String> = catalog
        .collections()
        .map(|collection| collection.name.clone())
        .collect();
    let class_count = catalog.classes().count();
    let attr_count = catalog.attributes().count();
    rsx! {
        section { class: "semantic-home",
            h2 { "Workspace" }
            div { class: "semantic-stats",
                span { "{collections.len()} collections" }
                span { "{class_count} classes" }
                span { "{attr_count} attributes" }
            }
            h3 { "Collections" }
            ul {
                for collection in collections {
                    li {
                        {
                            let collection_for_open = collection.clone();
                            rsx! {
                        Link {
                            to: Route::CollectionPage {
                                collection: collection_for_open
                            },
                            class: "dx-button",
                            "data-style": "link",
                            "data-size": "default",
                            "{collection}"
                        }
                            }
                        }
                    }
                }
            }
        }
    }
}
