use crate::{
    MainContentView,
    ui_catalog::{ClassRenderContext, UiCatalog},
};
use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use std::rc::Rc;

pub(crate) fn register_task_comment_renderers(catalog: &mut UiCatalog) {
    catalog
        .render_registry_mut()
        .register_class_renderer("semantic:tasks:task", Rc::new(render_task));
    catalog
        .render_registry_mut()
        .register_class_renderer("semantic:comments:comment", Rc::new(render_comment));
    catalog.render_registry_mut().register_attribute_renderer(
        "semantic:base:main_content",
        Rc::new(|_, value, _| rsx! { MainContentView { value: value.clone() } }),
    );
}

fn field<'a>(object: &'a Object, name: &str, canonical: &str) -> Option<&'a Value> {
    object.get(name).or_else(|| object.get(canonical))
}
fn text(object: &Object, name: &str, canonical: &str) -> String {
    match field(object, name, canonical) {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Variant(value)) => value.variant.clone(),
        Some(value) => crate::ui_catalog::defaults::value_to_text(value),
        None => String::new(),
    }
}
fn render_task(ctx: ClassRenderContext) -> Element {
    let title = text(&ctx.object, "title", "semantic:title");
    let status = text(&ctx.object, "status", "semantic:tasks:task:status").replace('_', " ");
    let priority = text(&ctx.object, "priority", "semantic:tasks:task:priority");
    let due = text(&ctx.object, "due_date", "semantic:tasks:task:due_date");
    let progress = text(&ctx.object, "progress", "semantic:tasks:task:progress");
    let content = field(&ctx.object, "main_content", "semantic:base:main_content").cloned();
    rsx! { article { class: "semantic-task-card",
        h2 { "{title}" }
        div { class: "semantic-task-card__metadata", style: "display:flex;gap:.75rem;flex-wrap:wrap;font-size:.85rem;margin-bottom:1rem",
            span { "{status}" } span { "{priority} priority" }
            if !due.is_empty() { span { "Due {due}" } }
            span { "{progress}% complete" }
        }
        if let Some(value) = content { MainContentView { value } }
    } }
}
fn render_comment(ctx: ClassRenderContext) -> Element {
    let author = text(
        &ctx.object,
        "author_id",
        "semantic:comments:comment:author_id",
    );
    let created = text(&ctx.object, "created_at", "semantic:created_at");
    let updated = text(&ctx.object, "updated_at", "semantic:updated_at");
    let edited = !updated.is_empty() && updated != created;
    let deleted = field(&ctx.object, "deleted", "semantic:comments:comment:deleted")
        == Some(&Value::Bool(true));
    let content = field(&ctx.object, "main_content", "semantic:base:main_content").cloned();
    rsx! { article { class: "semantic-comment-card",
        header { style: "display:flex;gap:.75rem;flex-wrap:wrap;margin-bottom:.75rem", strong { "{author}" } small { "{created}" } if edited && !deleted { small { "Edited" } } }
        if deleted { p { "This comment was deleted." } }
        else if let Some(value) = content { MainContentView { value } }
    } }
}
