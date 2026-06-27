use dioxus::prelude::*;
use semantic_ui_core::use_ui_catalog;

#[component]
pub fn HomeScreen(on_open_collection: EventHandler<String>) -> Element {
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
                        dxcomp::Button {
                            variant: dxcomp::ButtonVariant::Link,
                            onclick: move |_| on_open_collection.call(collection_for_open.clone()),
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
