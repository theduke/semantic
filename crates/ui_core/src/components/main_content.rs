use dioxus::prelude::*;
use semantic_base::content::MainContent;
use semantic_data::value::{FromValue, IntoValue, Value};

/// Controlled embedded document editor. Draft ownership stays with its caller.
#[component]
pub fn MainContentEditor(
    value: Value,
    on_change: EventHandler<Value>,
    #[props(default)] disabled: bool,
    #[props(default = "Description".to_string())] label: String,
) -> Element {
    let content = MainContent::from_value(value.clone());
    let Ok(content) = content else {
        return rsx! { p { class: "semantic-content__unsupported", role: "status", "This content format cannot be edited by this version." } };
    };
    let body = content.body().to_owned();
    #[cfg(feature = "markdown")]
    let format = content.format().to_owned();
    let change = move |body: String| {
        let MainContent::Note {
            format, created_at, ..
        } = content.clone();
        on_change.call(
            MainContent::Note {
                format,
                body,
                created_at,
                updated_at: semantic_data::value::DateTime::now_utc(),
            }
            .into_value(),
        );
    };
    #[cfg(feature = "markdown")]
    if format == "markdown" {
        return rsx! { div { class: "semantic-content-editor", role: "group", aria_label: "{label}",
            MarkdownContentEditor { body, disabled, on_change: change }
        } };
    }
    rsx! { div { class: "semantic-content-editor",
        style { {include_str!("comments.css")} }
        textarea { class: "semantic-content-editor__textarea", aria_label: "{label}", rows: 7,
            value: "{body}", disabled, placeholder: "Start writing…",
            oninput: move |event| change(event.value()),
        }
    } }
}

#[cfg(feature = "markdown")]
#[component]
fn MarkdownContentEditor(body: String, disabled: bool, on_change: EventHandler<String>) -> Element {
    let entity_links = crate::editor_entity_links::use_semantic_entity_links();
    rsx! { dxeditor::MarkdownEditor {
        value: body, readonly: disabled, entity_links: Some(entity_links),
        on_change: move |body| on_change.call(body),
    } }
}

#[component]
pub fn MainContentView(value: Value) -> Element {
    match MainContent::from_value(value) {
        Ok(content) => {
            let body = content.body().to_owned();
            #[cfg(feature = "markdown")]
            if content.format() == "markdown" {
                return rsx! { div { class: "semantic-content-view", style { {include_str!("comments.css")} } MarkdownContentEditor { body, disabled: true, on_change: |_| {} } } };
            }
            rsx! { div { style { {include_str!("comments.css")} } pre { class: "semantic-content-view semantic-content-view--text", "{body}" } } }
        }
        Err(_) => {
            rsx! { p { class: "semantic-content__unsupported", "This content format is not supported by this version." } }
        }
    }
}
