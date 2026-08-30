use std::rc::Rc;

#[cfg(feature = "markdown")]
use std::time::Duration;

use dioxus::prelude::*;
use semantic_data::value::{Object, Value};

use crate::form::{
    AttributeFormRenderContext, AutoExpandingTextarea, ClassFormRenderContext,
    ClassFormRenderOptions, render_class_form_body_with_options, render_class_form_field_row,
};
use crate::ui_catalog::{
    ClassRenderContext, RenderCtx, UiCatalog, defaults::value_to_text, use_ui_catalog,
};

const NOTE_CLASS_ID: &str = "semantic:base:note";
const ATTR_NOTE_FORMAT: &str = "semantic:base:note:note_format";
const ATTR_NOTE_CONTENT: &str = "semantic:base:note:note_content";
const FIELD_NOTE_FORMAT: &str = "note_format";
const FIELD_NOTE_CONTENT: &str = "note_content";
const FORMAT_TEXT: &str = "text";
const FORMAT_MARKDOWN: &str = "markdown";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoteContentMode {
    Formatted,
    Raw,
}

pub(crate) fn register_note_renderers(catalog: &mut UiCatalog) {
    let content_renderer = Rc::new(|_ctx: RenderCtx, value: &Value, object: Option<&Object>| {
        let content = value_string(value);
        let format = object
            .and_then(note_format)
            .unwrap_or(FORMAT_TEXT)
            .to_string();
        rsx! {
            NoteContentView {
                content,
                format,
            }
        }
    });

    catalog
        .render_registry_mut()
        .register_attribute_renderer(FIELD_NOTE_CONTENT, content_renderer.clone());
    catalog
        .render_registry_mut()
        .register_attribute_renderer(ATTR_NOTE_CONTENT, content_renderer);
    catalog
        .render_registry_mut()
        .register_class_renderer(NOTE_CLASS_ID, Rc::new(render_note_class));
    catalog
        .form_registry_mut()
        .register_class_form_renderer(NOTE_CLASS_ID, Rc::new(render_note_class_form));

    catalog
        .form_registry_mut()
        .register_class_field_form_renderer(
            NOTE_CLASS_ID,
            FIELD_NOTE_FORMAT,
            Rc::new(render_note_format_form),
        );
    catalog
        .form_registry_mut()
        .register_attribute_form_renderer(ATTR_NOTE_FORMAT, Rc::new(render_note_format_form));
    catalog
        .form_registry_mut()
        .register_class_field_form_renderer(
            NOTE_CLASS_ID,
            FIELD_NOTE_CONTENT,
            Rc::new(render_note_content_form),
        );
    catalog
        .form_registry_mut()
        .register_attribute_form_renderer(ATTR_NOTE_CONTENT, Rc::new(render_note_content_form));
}

fn render_note_class(ctx: ClassRenderContext) -> Element {
    let content = object_string(&ctx.object, &[FIELD_NOTE_CONTENT, ATTR_NOTE_CONTENT])
        .unwrap_or_default()
        .to_string();
    let format = note_format(&ctx.object).unwrap_or(FORMAT_TEXT).to_string();
    let title = ctx
        .object
        .get("title")
        .or_else(|| ctx.object.get("semantic:title"))
        .and_then(Value::as_str)
        .unwrap_or("Note")
        .to_string();

    rsx! {
        article { class: "semantic-note",
            header { class: "semantic-note__header",
                h2 { "{title}" }
                if let Some(id) = ctx.id {
                    code { "{id}" }
                }
            }
            NoteContentView {
                content,
                format,
            }
        }
    }
}

fn render_note_class_form(ctx: ClassFormRenderContext) -> Element {
    let catalog = use_ui_catalog();
    let form_value = ctx.scope.value();
    let format = match &form_value {
        Value::Object(object) => note_format(object).unwrap_or(FORMAT_TEXT),
        _ => FORMAT_TEXT,
    }
    .to_string();
    let format_label = if format == FORMAT_MARKDOWN {
        "Markdown"
    } else {
        "Text"
    };
    let content_field = catalog
        .class_form_fields(&ctx.class)
        .into_iter()
        .find(|field| {
            field.field_name == FIELD_NOTE_CONTENT || field.storage_field_name == ATTR_NOTE_CONTENT
        });
    let metadata_options = ClassFormRenderOptions::default()
        .show_header(false)
        .exclude_field(FIELD_NOTE_CONTENT);

    rsx! {
        div {
            class: "semantic-note-form",
            "data-format": "{format}",
            header { class: "semantic-note-form__header",
                div {
                    h2 { "Note" }
                    p { "Write the note first, then adjust its details below." }
                }
                span { class: "semantic-note-form__format", "{format_label}" }
            }
            section {
                class: "semantic-note-form__document",
                aria_label: "Note content",
                if let Some(content_field) = content_field {
                    div { class: "semantic-table-wrap semantic-note-form__content-table-wrap",
                        table { class: "semantic-field-table semantic-note-form__content-table",
                            tbody {
                                {render_class_form_field_row(&ctx, content_field)}
                            }
                        }
                    }
                } else {
                    div { class: "semantic-form__unsupported", role: "alert",
                        "The note content field is not registered in the active catalog."
                    }
                }
            }
            section { class: "semantic-note-form__metadata", aria_label: "Note details",
                header { class: "semantic-note-form__section-header",
                    h3 { "Note details" }
                    p { "Format, timestamps, metadata, and other schema fields." }
                }
                {render_class_form_body_with_options(ctx.clone(), metadata_options)}
            }
        }
    }
}

fn render_note_format_form(ctx: AttributeFormRenderContext) -> Element {
    let field = ctx.field.clone();
    let value = match field.value() {
        Value::String(value) if value == FORMAT_MARKDOWN => FORMAT_MARKDOWN.to_string(),
        _ => FORMAT_TEXT.to_string(),
    };
    rsx! {
        select {
            class: "semantic-form__select",
            value,
            onchange: move |event| field.set_value(Value::String(event.value())),
            option { value: FORMAT_TEXT, "Text" }
            option { value: FORMAT_MARKDOWN, "Markdown" }
        }
    }
}

fn render_note_content_form(ctx: AttributeFormRenderContext) -> Element {
    #[cfg(feature = "markdown")]
    if note_content_format(&ctx) == FORMAT_MARKDOWN {
        return render_markdown_note_content_form(ctx);
    }

    let field = ctx.field.clone();
    let value = value_string(&field.value());
    let input_field = field.clone();
    let blur_field = field.clone();
    let focus_field = field;

    rsx! {
        AutoExpandingTextarea {
            value,
            rows: 8,
            oninput: move |value| input_field.set_value(Value::String(value)),
            onblur: move |_event: FocusEvent| blur_field.set_focused(false),
            onfocus: move |_event: FocusEvent| focus_field.set_focused(true),
        }
    }
}

#[cfg(feature = "markdown")]
fn render_markdown_note_content_form(ctx: AttributeFormRenderContext) -> Element {
    let entity_links = crate::editor_entity_links::use_semantic_entity_links();
    let field = ctx.field.clone();
    let value = value_string(&field.value());
    let initial_value = value.clone();
    let mut last_prop_value = use_signal(move || initial_value);
    let mut last_emitted_value = use_signal(|| None::<String>);
    let mut pending_value = use_signal(|| None::<String>);
    let mut debounce_revision = use_signal(|| 0_u64);

    let incoming_value = value.clone();
    use_effect(move || {
        if *last_prop_value.peek() == incoming_value {
            return;
        }

        last_prop_value.set(incoming_value.clone());
        if last_emitted_value.peek().as_ref() == Some(&incoming_value) {
            last_emitted_value.set(None);
            return;
        }

        debounce_revision += 1;
        pending_value.set(None);
    });

    let drop_field = field.clone();
    use_drop(move || {
        if let Some(value) = pending_value.take() {
            drop_field.set_value(Value::String(value));
        }
    });

    let change_field = field.clone();
    let blur_field = field.clone();
    let focus_field = field;
    rsx! {
        dxeditor::MarkdownEditor {
            value,
            entity_links: Some(entity_links),
            on_change: move |value: String| {
                pending_value.set(Some(value));
                debounce_revision += 1;
                let scheduled_revision = debounce_revision();
                let field = change_field.clone();
                spawn(async move {
                    dioxus_sdk_time::sleep(Duration::from_millis(500)).await;
                    if debounce_revision() != scheduled_revision {
                        return;
                    }
                    if let Some(value) = pending_value.take() {
                        last_emitted_value.set(Some(value.clone()));
                        field.set_value(Value::String(value));
                    }
                });
            },
            onblur: move |_event: FocusEvent| {
                debounce_revision += 1;
                if let Some(value) = pending_value.take() {
                    last_emitted_value.set(Some(value.clone()));
                    blur_field.set_value(Value::String(value));
                }
                blur_field.set_focused(false);
            },
            onfocus: move |_event: FocusEvent| focus_field.set_focused(true),
        }
    }
}

#[cfg(feature = "markdown")]
fn note_content_format(ctx: &AttributeFormRenderContext) -> String {
    match ctx.scope.root_value() {
        Value::Object(object) => note_format(&object).unwrap_or(FORMAT_TEXT).to_string(),
        _ => FORMAT_TEXT.to_string(),
    }
}

#[component]
fn NoteContentView(content: String, format: String) -> Element {
    let default_mode = if format == FORMAT_MARKDOWN && markdown_rendering_enabled() {
        NoteContentMode::Formatted
    } else {
        NoteContentMode::Raw
    };
    let mut mode = use_signal(move || default_mode);
    let can_render_formatted = format == FORMAT_MARKDOWN && markdown_rendering_enabled();
    let show_formatted = mode() == NoteContentMode::Formatted && can_render_formatted;
    rsx! {
        div { class: "semantic-note__content",
            div { class: "semantic-note__toolbar", role: "group", aria_label: "Note view mode",
                if can_render_formatted {
                    dxcomp::Button {
                        r#type: "button",
                        variant: if show_formatted { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                        size: dxcomp::ButtonSize::Sm,
                        onclick: move |_| mode.set(NoteContentMode::Formatted),
                        "Formatted"
                    }
                }
                dxcomp::Button {
                    r#type: "button",
                    variant: if show_formatted { dxcomp::ButtonVariant::Outline } else { dxcomp::ButtonVariant::Primary },
                    size: dxcomp::ButtonSize::Sm,
                    onclick: move |_| mode.set(NoteContentMode::Raw),
                    "Raw"
                }
            }
            if show_formatted {
                FormattedMarkdownView { content: content.clone() }
            } else {
                pre { class: "semantic-note__raw semantic-value semantic-value--raw", "{content}" }
            }
        }
    }
}

#[cfg(feature = "markdown")]
#[component]
fn FormattedMarkdownView(content: String) -> Element {
    let entity_links = crate::editor_entity_links::use_semantic_entity_links();
    rsx! {
        div { class: "semantic-note__formatted",
            dxeditor::MarkdownEditor {
                value: content,
                readonly: true,
                entity_links: Some(entity_links),
                on_change: move |_| {},
            }
        }
    }
}

#[cfg(not(feature = "markdown"))]
#[component]
fn FormattedMarkdownView(content: String) -> Element {
    rsx! { pre { class: "semantic-note__raw", "{content}" } }
}

fn note_format(object: &Object) -> Option<&str> {
    object_format(object, &[FIELD_NOTE_FORMAT, ATTR_NOTE_FORMAT])
}

fn object_string<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn object_format<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(value_format))
}

fn value_format(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value.as_str()),
        Value::Variant(value) => Some(value.variant.as_str()),
        _ => None,
    }
}

fn value_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null | Value::Void => String::new(),
        other => value_to_text(other),
    }
}

fn markdown_rendering_enabled() -> bool {
    cfg!(feature = "markdown")
}

#[cfg(all(feature = "markdown", test))]
fn markdown_to_html(markdown: &str) -> String {
    let parser = pulldown_cmark::Parser::new_ext(markdown, pulldown_cmark::Options::all())
        .map(sanitize_markdown_event);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

#[cfg(all(feature = "markdown", test))]
fn sanitize_markdown_event(event: pulldown_cmark::Event<'_>) -> pulldown_cmark::Event<'_> {
    use pulldown_cmark::{Event, Tag};

    match event {
        Event::Html(html) | Event::InlineHtml(html) => Event::Text(html),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: if safe_markdown_url(&dest_url) {
                dest_url
            } else {
                "#".into()
            },
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: if safe_markdown_url(&dest_url) {
                dest_url
            } else {
                "about:blank".into()
            },
            title,
            id,
        }),
        other => other,
    }
}

#[cfg(all(feature = "markdown", test))]
fn safe_markdown_url(url: &str) -> bool {
    let normalized = url
        .chars()
        .filter(|character| !character.is_ascii_control() && !character.is_ascii_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let Some(colon) = normalized.find(':') else {
        return true;
    };
    if normalized[..colon]
        .chars()
        .any(|character| matches!(character, '/' | '?' | '#'))
    {
        return true;
    }
    matches!(
        &normalized[..colon],
        "http" | "https" | "mailto" | "tel" | "semantic"
    )
}

#[cfg(test)]
mod tests {
    use super::{FIELD_NOTE_FORMAT, FORMAT_MARKDOWN, note_format, value_string};
    use semantic_data::value::{Object, Value, VariantValue};

    #[test]
    fn note_format_prefers_field_name() {
        let mut object = Object::new();
        object.insert(
            FIELD_NOTE_FORMAT.to_string(),
            Value::String(FORMAT_MARKDOWN.to_string()),
        );

        assert_eq!(note_format(&object), Some(FORMAT_MARKDOWN));
    }

    #[test]
    fn note_format_accepts_attribute_id() {
        let mut object = Object::new();
        object.insert(
            super::ATTR_NOTE_FORMAT.to_string(),
            Value::String(FORMAT_MARKDOWN.to_string()),
        );

        assert_eq!(note_format(&object), Some(FORMAT_MARKDOWN));
    }

    #[test]
    fn note_format_accepts_variant_values() {
        let mut object = Object::new();
        object.insert(
            FIELD_NOTE_FORMAT.to_string(),
            Value::Variant(Box::new(VariantValue {
                r#type: None,
                variant: FORMAT_MARKDOWN.to_string(),
                value: Value::Null,
            })),
        );

        assert_eq!(note_format(&object), Some(FORMAT_MARKDOWN));
    }

    #[test]
    fn value_string_handles_empty_values() {
        assert_eq!(value_string(&Value::Null), "");
        assert_eq!(
            value_string(&Value::String("body".to_string())),
            "body".to_string()
        );
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_to_html_renders_markdown() {
        assert!(super::markdown_to_html("# Title").contains("<h1>Title</h1>"));
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_to_html_escapes_raw_html() {
        let html = super::markdown_to_html(
            "<script>alert('xss')</script>\n\ntext <img src=x onerror=alert(1)>",
        );
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_to_html_blocks_active_url_schemes() {
        let html = super::markdown_to_html(
            "[bad](javascript:alert(1)) ![bad](data:text/html,boom) [ok](https://example.com)",
        );
        assert!(!html.to_ascii_lowercase().contains("javascript:"));
        assert!(!html.to_ascii_lowercase().contains("data:text/html"));
        assert!(html.contains("href=\"#\""));
        assert!(html.contains("src=\"about:blank\""));
        assert!(html.contains("href=\"https://example.com\""));
    }
}
