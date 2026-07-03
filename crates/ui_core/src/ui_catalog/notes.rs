use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::value::{Object, Value};

use crate::form::{AttributeFormRenderContext, AutoExpandingTextarea};
use crate::ui_catalog::{ClassRenderContext, RenderCtx, UiCatalog, defaults::value_to_text};

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
    let field = ctx.field.clone();
    let value = value_string(&field.value());

    rsx! {
        dxeditor::MarkdownEditor {
            value,
            on_change: move |value: String| field.set_value(Value::String(value)),
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
    let markdown_content = content.clone();
    let rendered_markdown = use_memo(use_reactive!(|(markdown_content,)| markdown_to_html(
        &markdown_content
    )));

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
                div {
                    class: "semantic-note__formatted",
                    dangerous_inner_html: rendered_markdown(),
                }
            } else {
                pre { class: "semantic-note__raw semantic-value semantic-value--raw", "{content}" }
            }
        }
    }
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

#[cfg(feature = "markdown")]
fn markdown_to_html(markdown: &str) -> String {
    let parser = pulldown_cmark::Parser::new_ext(markdown, pulldown_cmark::Options::all());
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

#[cfg(not(feature = "markdown"))]
fn markdown_to_html(markdown: &str) -> String {
    markdown.to_string()
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
}
