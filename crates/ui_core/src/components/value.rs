use dioxus::prelude::*;
use semantic_data::schema::Type;
use semantic_data::value::Value;

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
    rsx! { span { "{text}" } }
}
