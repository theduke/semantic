use dioxus::prelude::*;

use crate::{components::PageHeader, views::Route};

#[component]
pub fn HomePage() -> Element {
    rsx! {
        div { class: "semantic-page semantic-home",
            PageHeader {
                title: "Workspace",
                description: Some("Choose a view to get started.".to_string()),
            }

            section { class: "semantic-route-panels semantic-home__panels", aria_label: "Workspace pages",
                Link {
                    to: Route::BrowsePage {
                        collection: None,
                        view: None,
                        renderer: None,
                        page: None,
                        page_size: None,
                        sql: None,
                    },
                    class: "semantic-route-panel semantic-surface",
                    h2 { "Browse" }
                    p { "Find entities across collections and view them as cards or a table." }
                    span { aria_hidden: "true", "Browse entities →" }
                }
                Link {
                    to: Route::TreePage { root: None },
                    class: "semantic-route-panel semantic-surface",
                    h2 { "Tree" }
                    p { "Arrange entities in directories and move through their hierarchy." }
                    span { aria_hidden: "true", "Open tree →" }
                }
                Link {
                    to: Route::PlayPage,
                    class: "semantic-route-panel semantic-surface",
                    h2 { "Player" }
                    p { "Play media entities and manage the current queue." }
                    span { aria_hidden: "true", "Open player →" }
                }
                Link {
                    to: Route::DataPage,
                    class: "semantic-route-panel semantic-surface",
                    h2 { "Data" }
                    p { "Open the catalog or run a read-only query." }
                    span { aria_hidden: "true", "Open data tools →" }
                }
            }
        }
    }
}
