use dioxus::prelude::*;

use crate::views::Route;

#[component]
pub fn AppShell() -> Element {
    rsx! {
        div { class: "semantic-ui",
            header { class: "semantic-ui__header",
                h1 { "Semantic" }
                nav {
                    Link {
                        to: Route::HomePage,
                        class: "dx-button",
                        "data-style": "ghost",
                        "data-size": "default",
                        "Home"
                    }
                    Link {
                        to: Route::CatalogPage,
                        class: "dx-button",
                        "data-style": "ghost",
                        "data-size": "default",
                        "Catalog"
                    }
                    Link {
                        to: Route::CollectionPage {
                            collection: "entities".to_string()
                        },
                        class: "dx-button",
                        "data-style": "ghost",
                        "data-size": "default",
                        "Entities"
                    }
                    Link {
                        to: Route::CreateEntityPage,
                        class: "dx-button",
                        "data-style": "ghost",
                        "data-size": "default",
                        "Create"
                    }
                    Link {
                        to: Route::QueryPage,
                        class: "dx-button",
                        "data-style": "ghost",
                        "data-size": "default",
                        "Query"
                    }
                    Link {
                        to: Route::TreePage { root: None },
                        class: "dx-button",
                        "data-style": "ghost",
                        "data-size": "default",
                        "Tree"
                    }
                    Link {
                        to: Route::UploadPage,
                        class: "dx-button",
                        "data-style": "ghost",
                        "data-size": "default",
                        "Upload"
                    }
                }
            }
            main { class: "semantic-ui__main",
                Outlet::<Route> {}
            }
        }
    }
}
