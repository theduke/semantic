use dioxus::prelude::*;

use crate::{components::PageHeader, views::Route};

#[component]
pub fn DataPage() -> Element {
    rsx! {
        div { class: "semantic-page semantic-data-page",
            PageHeader {
                title: "Data",
                description: Some(
                    "Inspect the data model or run a read-only query when you need a closer look."
                        .to_string(),
                ),
            }

            section { class: "semantic-route-panels", aria_label: "Data tools",
                Link {
                    to: Route::CatalogPage,
                    class: "semantic-route-panel semantic-surface",
                    h2 { "Catalog" }
                    p { "See the collections, classes, and attributes currently loaded." }
                    span { aria_hidden: "true", "Open catalog →" }
                }
                Link {
                    to: Route::QueryPage,
                    class: "semantic-route-panel semantic-surface",
                    h2 { "Query" }
                    p { "Run read-only SQL and inspect the returned rows." }
                    span { aria_hidden: "true", "Open query →" }
                }
            }
        }
    }
}
