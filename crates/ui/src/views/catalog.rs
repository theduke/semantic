use dioxus::prelude::*;
use semantic_ui_core::use_ui_catalog;

#[component]
pub fn CatalogPage() -> Element {
    let catalog = use_ui_catalog();
    let collections: Vec<String> = catalog
        .collections()
        .map(|collection| collection.name.clone())
        .collect();
    let classes: Vec<(String, String)> = catalog
        .classes()
        .map(|class| (class.id.clone(), class.name.clone()))
        .collect();
    let attributes: Vec<(String, String)> = catalog
        .attributes()
        .map(|attr| (attr.id.clone(), attr.name.clone()))
        .collect();

    rsx! {
        section { class: "semantic-catalog",
            h2 { "Catalog" }
            div { class: "semantic-catalog__grid",
                section {
                    h3 { "Collections" }
                    ul {
                        for collection in collections {
                            li { "{collection}" }
                        }
                    }
                }
                section {
                    h3 { "Classes" }
                    ul {
                        for (id, name) in classes {
                            li {
                                strong { "{name}" }
                                code { " {id}" }
                            }
                        }
                    }
                }
                section {
                    h3 { "Attributes" }
                    ul {
                        for (id, name) in attributes {
                            li {
                                strong { "{name}" }
                                code { " {id}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
