use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_icons::lucide::ExternalLink;
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

    super::notes::register_note_renderers(catalog);
    super::files::register_file_renderers(catalog);

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
        id: "labels".into(),
        label: "Edit labels".into(),
        icon: None,
        class_id: None,
        placements: vec![
            EntityActionPlacement::Card,
            EntityActionPlacement::Detail,
            EntityActionPlacement::BrowseRow,
        ],
        enabled: Rc::new(|ctx| !ctx.target.id.is_empty()),
        render: Rc::new(
            |ctx| rsx! { crate::components::EntityLabelsButton { target: ctx.target } },
        ),
    });
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
            EntityActionPlacement::Card,
            EntityActionPlacement::Detail,
            EntityActionPlacement::BrowseRow,
        ],
        enabled: Rc::new(|ctx: &EntityActionContext| !ctx.target.id.is_empty()),
        render: Rc::new(|ctx: EntityActionContext| {
            rsx! {
                EntityDeleteButton {
                    target: ctx.target,
                    on_deleted: ctx.on_delete
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
                    class: "dx-button semantic-entity-action",
                    "data-style": "outline",
                    "data-size": "icon-sm",
                    href,
                    target: "_blank",
                    rel: "noopener noreferrer",
                    title: "Open External",
                    aria_label: "Open external link",
                    ExternalLink { size: "1rem" }
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
        Value::Date(value) => format_date(time::Date::from(*value)),
        Value::DateTime(value) => format_datetime(time::OffsetDateTime::from(*value)),
        Value::Bytes(value) => format!("{} bytes", value.len()),
        Value::String(value) => value.clone(),
        Value::List(value) => format!("{} items", value.len()),
        Value::Map(value) => format!("{value:?}"),
        Value::Object(value) => format!("{} fields", value.len()),
        Value::Variant(value) => format!("{value:?}"),
    }
}

fn format_date(value: time::Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        value.year(),
        u8::from(value.month()),
        value.day()
    )
}

fn format_datetime(value: time::OffsetDateTime) -> String {
    let date = format_date(value.date());
    if value.second() == 0 && value.nanosecond() == 0 {
        format!("{date} {:02}:{:02}", value.hour(), value.minute())
    } else {
        format!(
            "{date} {:02}:{:02}:{:02}",
            value.hour(),
            value.minute(),
            value.second()
        )
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

    #[test]
    fn value_to_text_formats_dates() {
        let date = time::Date::from_calendar_date(2026, time::Month::August, 31).expect("date");

        assert_eq!(value_to_text(&Value::Date(date.into())), "2026-08-31");
    }

    #[test]
    fn value_to_text_formats_datetimes_with_optional_seconds() {
        let date = time::Date::from_calendar_date(2026, time::Month::August, 31).expect("date");
        let without_seconds = date.with_hms(2, 43, 0).expect("datetime").assume_utc();
        let with_seconds = date
            .with_hms_nano(2, 43, 58, 790_000_000)
            .expect("datetime")
            .assume_utc();

        assert_eq!(
            value_to_text(&Value::DateTime(without_seconds.into())),
            "2026-08-31 02:43"
        );
        assert_eq!(
            value_to_text(&Value::DateTime(with_seconds.into())),
            "2026-08-31 02:43:58"
        );
    }
}
