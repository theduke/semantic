use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::builtin::ID_ATTRIBUTE_ID;
use semantic_data::filestore::{
    FILE_FILENAME_ATTRIBUTE_ID, FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID, FILE_MIME_TYPE_ATTRIBUTE_ID,
};
use semantic_data::value::{Object, Value};

use crate::ui_catalog::{RenderCtx, RenderMode, UiCatalog, ValueRenderContext};

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

    let file_renderer = Rc::new(|ctx: RenderCtx, value: &Value, object: Option<&Object>| {
        let Some(file_id) = file_link_id(value, object) else {
            let text = value_to_text(value);
            return rsx! { span { class: "semantic-value semantic-value--scalar", "{text}" } };
        };
        let href = format!("{}/{}", ctx.settings.file_api_prefix, file_id);
        let mime_type = object
            .and_then(|object| object_string(object, &["mime_type", FILE_MIME_TYPE_ATTRIBUTE_ID]))
            .unwrap_or_default();
        let filename = object
            .and_then(|object| object_string(object, &["filename", FILE_FILENAME_ATTRIBUTE_ID]))
            .unwrap_or(file_id);
        if ctx.settings.show_media && mime_type.starts_with("image/") {
            rsx! {
                a {
                    class: "semantic-file semantic-file--image",
                    href: "{href}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    img {
                        class: "semantic-file__image",
                        src: "{href}",
                        alt: "{filename}"
                    }
                }
            }
        } else {
            rsx! {
                a {
                    class: "semantic-file semantic-file--link",
                    href: "{href}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "{filename}"
                }
            }
        }
    });
    for attribute_id in ["filestore_locator", FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID] {
        catalog
            .render_registry_mut()
            .register_attribute_renderer(attribute_id, file_renderer.clone());
    }
}

fn object_string<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn file_link_id<'a>(value: &'a Value, object: Option<&'a Object>) -> Option<&'a str> {
    object
        .and_then(|object| object_string(object, &[ID_ATTRIBUTE_ID]))
        .or_else(|| value.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_link_id_prefers_object_id() {
        let mut object = Object::new();
        object.insert(ID_ATTRIBUTE_ID, Value::String("entity-id".to_string()));

        assert_eq!(
            file_link_id(&Value::String("locator".to_string()), Some(&object)),
            Some("entity-id")
        );
    }

    #[test]
    fn file_link_id_falls_back_to_locator_value() {
        assert_eq!(
            file_link_id(&Value::String("file-locator".to_string()), None),
            Some("file-locator")
        );
    }
}
