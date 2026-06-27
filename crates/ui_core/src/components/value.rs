use dioxus::prelude::*;
use semantic_data::schema::Type;
use semantic_data::value::Value;

use crate::components::ObjectView;
use crate::form::{DynamicValueForm, SemanticFormMode, SemanticFormOptions};
use crate::ui_catalog::{RenderMode, ValueRenderContext, defaults::value_to_text, use_ui_catalog};

#[component]
pub fn ValueView(value: Value, type_hint: Option<Type>, mode: RenderMode) -> Element {
    if matches!(mode, RenderMode::Edit | RenderMode::Create) && type_hint.is_some() {
        let form_mode = if matches!(mode, RenderMode::Create) {
            SemanticFormMode::Create
        } else {
            SemanticFormMode::Edit
        };
        let mut options = SemanticFormOptions::new(value);
        options.mode = form_mode;
        options.type_hint = type_hint;
        options.show_actions = false;
        return rsx! { DynamicValueForm { options } };
    }

    match &value {
        Value::Object(object) => {
            return rsx! {
                ObjectView {
                    object: object.clone(),
                    mode
                }
            };
        }
        Value::List(values) => {
            if values.is_empty() {
                return rsx! { span { class: "semantic-value semantic-value--null", "empty list" } };
            }
            return rsx! {
                ol { class: "semantic-list",
                    for (index, item) in values.iter().enumerate() {
                        li { class: "semantic-list__item",
                            span { class: "semantic-list__index", "{index}" }
                            div { class: "semantic-list__value",
                                ValueView {
                                    value: item.clone(),
                                    type_hint: None,
                                    mode
                                }
                            }
                        }
                    }
                }
            };
        }
        Value::Map(map) => {
            if map.is_empty() {
                return rsx! { span { class: "semantic-value semantic-value--null", "empty map" } };
            }
            return rsx! {
                div { class: "semantic-map",
                    for (key, item) in map.iter() {
                        div { class: "semantic-map__entry",
                            div { class: "semantic-map__key",
                                ValueView {
                                    value: key.clone(),
                                    type_hint: None,
                                    mode: RenderMode::Inline
                                }
                            }
                            div { class: "semantic-map__value",
                                ValueView {
                                    value: item.clone(),
                                    type_hint: None,
                                    mode
                                }
                            }
                        }
                    }
                }
            };
        }
        Value::Null | Value::Void => {
            let text = value_to_text(&value);
            return rsx! { span { class: "semantic-value semantic-value--null", "{text}" } };
        }
        _ => {}
    }

    let catalog = use_ui_catalog();
    if let Some(ty) = &type_hint {
        if let Some(renderer) = catalog.render_registry().type_renderer(&ty.kind) {
            return renderer(ValueRenderContext {
                value,
                type_hint,
                mode,
            });
        }
    }
    if let Some(renderer) = catalog.render_registry().fallback_renderer() {
        return renderer(ValueRenderContext {
            value,
            type_hint,
            mode,
        });
    }
    let text = value_to_text(&value);
    rsx! { span { class: "semantic-value semantic-value--scalar", "{text}" } }
}
