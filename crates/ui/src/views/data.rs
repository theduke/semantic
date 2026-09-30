use dioxus::prelude::*;

use crate::{components::PageHeader, views::Route};

#[component]
pub fn DataPage() -> Element {
    rsx! {
        div { class: "semantic-page semantic-data-page",
            PageHeader {
                title: "Data",
                description: Some(
                    "Inspect the data model, run a read-only query, or monitor background jobs."
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
                Link {
                    to: Route::JobsPage,
                    class: "semantic-route-panel semantic-surface",
                    h2 { "Jobs" }
                    p { "Monitor background jobs, track progress, and cancel running work." }
                    span { aria_hidden: "true", "Open jobs →" }
                }
            }
        }
    }
}
