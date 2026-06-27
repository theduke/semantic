use std::rc::Rc;

use dioxus::prelude::*;
use dxform::{FormScope, ListSpec, ValidationStrategy};
use semantic_data::{schema::Type, value::Value};

use crate::form::{default_value_for_type, set_value_list, value_as_list};

pub fn value_list_spec(
    name: impl Into<String>,
    validation: ValidationStrategy,
) -> ListSpec<Value, Value> {
    ListSpec {
        name: name.into(),
        get: Rc::new(value_as_list),
        set: Rc::new(|parent: &mut Value, items: Vec<Value>| set_value_list(parent, items)),
        item_empty: Rc::new(crate::form::is_empty_value),
        validators: Vec::new(),
        validation,
    }
}

pub fn render_list_value_form(
    scope: FormScope<Value, Value>,
    item_type: Type,
    mode: crate::form::SemanticFormMode,
) -> Element {
    rsx! {
        ListValueForm {
            scope,
            item_type,
            mode,
        }
    }
}

#[component]
fn ListValueForm(
    scope: FormScope<Value, Value>,
    item_type: Type,
    mode: crate::form::SemanticFormMode,
) -> Element {
    let list = dxform::use_list(scope, || {
        value_list_spec("items", ValidationStrategy::submit())
    });
    let new_item_type = item_type.clone();
    rsx! {
        div { class: "semantic-form__list",
            div { class: "semantic-form__list-actions",
                button {
                    r#type: "button",
                    onclick: list.add_handler(Rc::new(move || default_value_for_type(&new_item_type))),
                    "Add"
                }
                button {
                    r#type: "button",
                    disabled: list.is_empty(),
                    onclick: list.clear_handler(),
                    "Clear"
                }
            }
            for item in list.items() {
                ListValueItem {
                    key: "{item.key()}:{item.index()}",
                    item,
                    item_type: item_type.clone(),
                    mode,
                }
            }
        }
    }
}

#[component]
fn ListValueItem(
    item: dxform::ListItemHandle<Value, Value>,
    item_type: Type,
    mode: crate::form::SemanticFormMode,
) -> Element {
    let scope = dxform::use_list_item_scope(item.clone());
    let path = scope.path();
    rsx! {
        div {
            class: "semantic-form__list-item",
            {crate::form::render_value_form_scope(crate::form::ValueFormRenderContext {
                scope,
                value_type: Some(item_type),
                mode,
                path,
            })}
            button {
                r#type: "button",
                onclick: item.remove_handler(),
                "Remove"
            }
        }
    }
}
