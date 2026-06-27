use dioxus::prelude::*;
use semantic_data::value::{Object, Value};

use crate::ui_catalog::{MediaRenderOptions, use_ui_catalog};

#[component]
pub fn MediaView(object: Object, options: MediaRenderOptions) -> Element {
    let catalog = use_ui_catalog();
    if let Some(Value::String(class_id)) = object.get("type") {
        if let Some(renderer) = catalog
            .media_renderers()
            .iter()
            .find(|renderer| renderer.class_id.as_deref() == Some(class_id.as_str()))
        {
            return (renderer.renderer)(object, options);
        }
    }

    let src = object
        .get("url")
        .or_else(|| object.get("path"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let content_type = object
        .get("mime")
        .or_else(|| object.get("content_type"))
        .and_then(Value::as_str)
        .unwrap_or("application/octet-stream")
        .to_string();

    rsx! {
        div { class: "semantic-media",
            if let Some(src) = src {
                a { href: "{src}", target: "_blank", "{src}" }
                span { " {content_type}" }
            } else {
                span { "media item" }
            }
        }
    }
}
