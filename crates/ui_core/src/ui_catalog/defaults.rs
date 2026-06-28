use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::builtin::ATTR_ID;
use semantic_data::filestore::{
    ATTR_FILE_FILENAME, ATTR_FILE_FILESTORE_LOCATOR, ATTR_FILE_MIME_TYPE,
};
use semantic_data::value::{Object, Value};
use tracing::{info, warn};

use crate::components::{EntityDeleteButton, EntityOpenButton};
use crate::ui_catalog::{
    EntityActionContext, EntityActionPlacement, EntityActionRegistration, RenderCtx, RenderMode,
    UiCatalog, ValueRenderContext,
};

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
    register_default_entity_actions(catalog);
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
            .and_then(|object| object_string(object, &["mime_type", ATTR_FILE_MIME_TYPE]))
            .unwrap_or_default();
        let filename = object
            .and_then(|object| object_string(object, &["filename", ATTR_FILE_FILENAME]))
            .unwrap_or(file_id);
        if ctx.settings.show_media && mime_type.starts_with("image/") {
            info!(
                target: "semantic_ui::file_render",
                file_id,
                href,
                mime_type,
                filename,
                mode = ?ctx.mode,
                "rendering file attribute as image"
            );
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
            warn!(
                target: "semantic_ui::file_render",
                file_id,
                href,
                mime_type,
                filename,
                show_media = ctx.settings.show_media,
                mode = ?ctx.mode,
                "rendering file attribute as link"
            );
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
    for attribute_id in ["filestore_locator", ATTR_FILE_FILESTORE_LOCATOR] {
        catalog
            .render_registry_mut()
            .register_attribute_renderer(attribute_id, file_renderer.clone());
    }
}

fn register_default_entity_actions(catalog: &mut UiCatalog) {
    catalog.register_entity_action(EntityActionRegistration {
        id: "open".to_string(),
        label: "Open".to_string(),
        icon: None,
        class_id: None,
        placements: vec![
            EntityActionPlacement::Card,
            EntityActionPlacement::Detail,
            EntityActionPlacement::BrowseRow,
        ],
        enabled: Rc::new(|ctx: &EntityActionContext| !ctx.target.id.is_empty()),
        render: Rc::new(|ctx: EntityActionContext| {
            rsx! {
                EntityOpenButton {
                    target: ctx.target
                }
            }
        }),
    });

    catalog.register_entity_action(EntityActionRegistration {
        id: "delete".to_string(),
        label: "Delete".to_string(),
        icon: None,
        class_id: None,
        placements: vec![
            EntityActionPlacement::Detail,
            EntityActionPlacement::BrowseRow,
        ],
        enabled: Rc::new(|ctx: &EntityActionContext| !ctx.target.id.is_empty()),
        render: Rc::new(|ctx: EntityActionContext| {
            rsx! {
                EntityDeleteButton {
                    target: ctx.target
                }
            }
        }),
    });

    catalog.register_entity_action(EntityActionRegistration {
        id: "open_external".to_string(),
        label: "Open External".to_string(),
        icon: None,
        class_id: None,
        placements: vec![
            EntityActionPlacement::Card,
            EntityActionPlacement::BrowseRow,
        ],
        enabled: Rc::new(|ctx: &EntityActionContext| external_url(&ctx.object).is_some()),
        render: Rc::new(|ctx: EntityActionContext| {
            let href = external_url(&ctx.object).unwrap_or_default();
            rsx! {
                a {
                    class: "dx-button",
                    "data-style": "outline",
                    "data-size": "sm",
                    href,
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "Open External"
                }
            }
        }),
    });
}

fn external_url(object: &Object) -> Option<String> {
    ["url", "href", "link"]
        .iter()
        .find_map(|field| object.get(*field).and_then(Value::as_str))
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .map(str::to_string)
}

fn object_string<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn file_link_id<'a>(value: &'a Value, object: Option<&'a Object>) -> Option<&'a str> {
    object
        .and_then(|object| object_string(object, &[ATTR_ID]))
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
        object.insert(ATTR_ID, Value::String("entity-id".to_string()));

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
