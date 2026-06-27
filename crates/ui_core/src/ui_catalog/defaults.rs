use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::value::Value;

use crate::ui_catalog::{RenderMode, UiCatalog, ValueRenderContext};

pub fn register_defaults(catalog: &mut UiCatalog) {
    let fallback = Rc::new(|ctx: ValueRenderContext| {
        let text = value_to_text(&ctx.value);
        match ctx.mode {
            RenderMode::Inline | RenderMode::Preview => rsx! { span { "{text}" } },
            RenderMode::Detail | RenderMode::Edit | RenderMode::Create => rsx! {
                pre { class: "semantic-value semantic-value--raw", "{text}" }
            },
        }
    });

    catalog
        .render_registry_mut()
        .set_fallback_renderer(fallback.clone());
    for key in [
        "any",
        "unknown",
        "null",
        "bool",
        "char",
        "number",
        "string",
        "bytes",
        "temporal",
        "uuid",
        "ip_addr",
        "json",
        "optional",
        "array",
        "list",
        "tuple",
        "map",
        "set",
        "record",
        "attribute",
        "class",
        "union",
        "intersection",
        "variant",
        "enum",
        "result",
        "opaque",
        "extension",
        "ref",
    ] {
        catalog
            .render_registry_mut()
            .register_type_renderer(key, fallback.clone());
    }
}

pub fn value_to_text(value: &Value) -> String {
    match value {
        Value::Void => String::new(),
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
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
        Value::Uuid(value) => format!("{value:?}"),
        Value::IpAddr(value) => value.to_string(),
        Value::Duration(value) => format!("{value:?}"),
        Value::Time(value) => format!("{value:?}"),
        Value::Date(value) => format!("{value:?}"),
        Value::DateTime(value) => format!("{value:?}"),
        Value::Bytes(value) => format!("{} bytes", value.len()),
        Value::String(value) => value.clone(),
        Value::List(value) => format!("{} items", value.len()),
        Value::Map(value) => format!("{value:?}"),
        Value::Object(value) => format!("{} fields", value.len()),
        Value::Variant(value) => format!("{value:?}"),
    }
}
