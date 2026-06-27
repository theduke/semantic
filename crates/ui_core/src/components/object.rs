use dioxus::prelude::*;
use semantic_data::value::Object;

use crate::components::ValueView;
use crate::ui_catalog::{RenderMode, use_ui_catalog};

#[component]
pub fn ObjectView(object: Object, mode: RenderMode) -> Element {
    let catalog = use_ui_catalog();
    let class = catalog.object_class(&object).cloned();
    rsx! {
        div { class: "semantic-object",
            div { class: "semantic-table-wrap semantic-table-wrap--object",
                table { class: "semantic-field-table semantic-field-table--object",
                    tbody {
                        if let Some(class) = class {
                            tr {
                                th { scope: "row", "type" }
                                td {
                                    div { class: "semantic-value semantic-value--scalar", "{class.name}" }
                                }
                            }
                        }
                        for (key, value) in object.iter() {
                            tr {
                                th { scope: "row", "{key}" }
                                td {
                                    ValueView {
                                        value: value.clone(),
                                        type_hint: None,
                                        mode
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
