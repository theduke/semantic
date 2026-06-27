use std::rc::Rc;

use dioxus::prelude::*;
use dxform::{FieldHandle, FormError, FormErrorSource, FormScope, ValidationStrategy};
use semantic_data::{
    schema::{NumberType, StringFormat, Type, TypeKind},
    value::Value,
};

use crate::{
    ValueView,
    form::{ValueFormRenderContext, render_list_value_form, render_value_form_scope},
    ui_catalog::RenderMode,
};

pub fn register_default_form_renderers(catalog: &mut crate::UiCatalog) {
    let fallback = Rc::new(|ctx: ValueFormRenderContext| {
        rsx! {
            FallbackValueDisplay {
                scope: ctx.scope.clone(),
                value_type: ctx.value_type.clone(),
            }
        }
    });
    catalog
        .form_registry_mut()
        .set_fallback_form_renderer(fallback.clone());

    catalog
        .form_registry_mut()
        .register_type_form_renderer("string", Rc::new(render_string));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("char", Rc::new(render_string));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("bool", Rc::new(render_bool));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("number", Rc::new(render_number));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("optional", Rc::new(render_optional));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("list", Rc::new(render_list));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("array", Rc::new(render_list));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("set", Rc::new(render_list));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("class", Rc::new(render_nested_class));
    catalog
        .form_registry_mut()
        .register_type_form_renderer("record", fallback.clone());
    catalog
        .form_registry_mut()
        .register_type_form_renderer("object", fallback.clone());
}

fn render_string(ctx: ValueFormRenderContext) -> Element {
    rsx! {
        StringValueInput {
            scope: ctx.scope.clone(),
            value_type: ctx.value_type.clone(),
        }
    }
}

#[component]
fn StringValueInput(scope: FormScope<Value, Value>, value_type: Option<Type>) -> Element {
    let field = use_value_leaf_field(scope);
    let value = match field.value() {
        Value::String(value) => value,
        Value::Null | Value::Void => String::new(),
        value => crate::ui_catalog::defaults::value_to_text(&value),
    };
    let input_type = string_input_type(value_type.as_ref());
    let input_field = field.clone();
    let blur_field = field.clone();
    let focus_field = field.clone();
    rsx! {
        input {
            class: "semantic-form__input",
            r#type: "{input_type}",
            value,
            oninput: move |event| input_field.set_value(Value::String(event.value())),
            onblur: move |_| blur_field.set_focused(false),
            onfocus: move |_| focus_field.set_focused(true),
        }
    }
}

fn render_bool(ctx: ValueFormRenderContext) -> Element {
    rsx! { BoolValueInput { scope: ctx.scope } }
}

#[component]
fn BoolValueInput(scope: FormScope<Value, Value>) -> Element {
    let field = use_value_leaf_field(scope);
    let checked = matches!(field.value(), Value::Bool(true));
    rsx! {
        input {
            class: "semantic-form__checkbox",
            r#type: "checkbox",
            checked,
            onchange: move |event| field.set_value(Value::Bool(event.checked())),
        }
    }
}

fn render_number(ctx: ValueFormRenderContext) -> Element {
    rsx! {
        NumberValueInput {
            scope: ctx.scope,
            value_type: ctx.value_type.clone(),
        }
    }
}

#[component]
fn NumberValueInput(scope: FormScope<Value, Value>, value_type: Option<Type>) -> Element {
    let field = use_value_leaf_field(scope);
    let ty = value_type.clone();
    let value = number_to_string(&field.value());
    rsx! {
        input {
            class: "semantic-form__input",
            r#type: "number",
            value,
            oninput: move |event| {
                match parse_number_value(&event.value(), ty.as_ref()) {
                    Ok(value) => field.set_value(value),
                    Err(error) => field.set_value(Value::String(error.message)),
                }
            },
        }
    }
}

fn render_optional(ctx: ValueFormRenderContext) -> Element {
    let Some(Type {
        kind: TypeKind::Optional(optional),
        ..
    }) = ctx.value_type.clone()
    else {
        return fallback_edit(ctx);
    };
    let inner = (*optional.inner).clone();
    rsx! {
        div { class: "semantic-form__optional",
            OptionalClearButton { scope: ctx.scope.clone() }
            {render_value_form_scope(ValueFormRenderContext {
                scope: ctx.scope,
                value_type: Some(inner),
                mode: ctx.mode,
                path: ctx.path,
            })}
        }
    }
}

#[component]
fn OptionalClearButton(scope: FormScope<Value, Value>) -> Element {
    let field = use_value_leaf_field(scope);
    rsx! {
            button {
                r#type: "button",
                onclick: move |_| field.set_value(Value::Null),
                "Clear"
            }
    }
}

fn render_list(ctx: ValueFormRenderContext) -> Element {
    let Some(Type {
        kind: TypeKind::List(list),
        ..
    }) = ctx.value_type.clone()
    else {
        return fallback_edit(ctx);
    };
    render_list_value_form(ctx.scope, (*list.items).clone(), ctx.mode)
}

fn render_nested_class(ctx: ValueFormRenderContext) -> Element {
    let Some(Type {
        kind: TypeKind::Class(class),
        ..
    }) = ctx.value_type.clone()
    else {
        return fallback_edit(ctx);
    };
    let catalog = crate::use_ui_catalog();
    let class_ctx = crate::form::ClassFormRenderContext {
        form: crate::form::use_semantic_form_root(),
        scope: ctx.scope,
        class: class.clone(),
        collection: None,
        id: None,
        mode: ctx.mode,
    };
    let renderer = catalog
        .form_registry()
        .class_form_renderer(&class.id)
        .or_else(|| {
            class.inherits.as_ref().and_then(|parent| {
                catalog
                    .form_registry()
                    .class_form_renderer(&parent.id)
                    .filter(|_| catalog.class_inherits(&class.id, &parent.id))
            })
        });
    if let Some(renderer) = renderer {
        renderer(class_ctx)
    } else {
        crate::form::render_class_form_body(class_ctx)
    }
}

fn fallback_edit(ctx: ValueFormRenderContext) -> Element {
    rsx! {
        FallbackValueDisplay {
            scope: ctx.scope.clone(),
            value_type: ctx.value_type.clone(),
        }
    }
}

#[component]
fn FallbackValueDisplay(
    scope: dxform::FormScope<Value, Value>,
    value_type: Option<Type>,
) -> Element {
    rsx! {
        div { class: "semantic-form__unsupported",
            ValueView {
                value: scope.value(),
                type_hint: value_type,
                mode: RenderMode::Detail
            }
        }
    }
}

fn use_value_leaf_field(scope: FormScope<Value, Value>) -> FieldHandle<Value, Value> {
    dxform::use_field(scope, || dxform::FieldSpec {
        name: "value".to_string(),
        get: Rc::new(|value: &Value| value.clone()),
        set: Rc::new(|parent: &mut Value, value: Value| *parent = value),
        is_empty: Rc::new(crate::form::is_empty_value),
        validators: Vec::new(),
        validation: ValidationStrategy::submit(),
    })
}

fn string_input_type(ty: Option<&Type>) -> &'static str {
    match ty.and_then(|ty| match &ty.kind {
        TypeKind::String(string) => string.format.as_ref(),
        _ => None,
    }) {
        Some(StringFormat::Email) => "email",
        Some(StringFormat::Uri | StringFormat::Url) => "url",
        _ => "text",
    }
}

fn number_to_string(value: &Value) -> String {
    match value {
        Value::I8(value) => value.to_string(),
        Value::I16(value) => value.to_string(),
        Value::I32(value) => value.to_string(),
        Value::I64(value) => value.to_string(),
        Value::I128(value) => value.to_string(),
        Value::U8(value) => value.to_string(),
        Value::U16(value) => value.to_string(),
        Value::U32(value) => value.to_string(),
        Value::U64(value) => value.to_string(),
        Value::U128(value) => value.to_string(),
        Value::F32(value) => value.to_string(),
        Value::F64(value) => value.to_string(),
        _ => String::new(),
    }
}

fn parse_number_value(value: &str, ty: Option<&Type>) -> std::result::Result<Value, FormError> {
    let number_type = ty.and_then(|ty| match &ty.kind {
        TypeKind::Number(number) => Some(number),
        _ => None,
    });
    match number_type {
        Some(NumberType::UInt(_)) | Some(NumberType::BigUInt(_)) => {
            value.parse::<u64>().map(Value::U64).map_err(parse_error)
        }
        Some(NumberType::Int(_)) | Some(NumberType::BigInt(_)) => {
            value.parse::<i64>().map(Value::I64).map_err(parse_error)
        }
        _ => value.parse::<f64>().map(Value::from).map_err(parse_error),
    }
}

fn parse_error(err: impl std::fmt::Display) -> FormError {
    FormError::new(format!("invalid number: {err}")).with_source(FormErrorSource::Parse)
}
