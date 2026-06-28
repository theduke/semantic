use dioxus::prelude::*;
use semantic_data::value::{Object, Value};

use crate::components::{ClassView, ObjectView};
use crate::ui_catalog::{
    EntityActionContext, EntityActionPlacement, EntityTarget, RenderMode, use_ui_catalog,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityDisplayRenderer {
    Custom,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityDisplayMode {
    Card,
    Table,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityRenderOptions {
    pub collection: Option<String>,
    pub id: Option<String>,
    pub renderer: EntityDisplayRenderer,
    pub preview: bool,
    pub actions: bool,
}

#[component]
pub fn EntityCard(object: Object, options: EntityRenderOptions) -> Element {
    let catalog = use_ui_catalog();
    let class = catalog.object_class(&object).cloned();
    let id = options.id.clone().or_else(|| object_id(&object));
    let title = entity_title(
        &object,
        id.as_deref(),
        class.as_ref().map(|class| class.name.as_str()),
    );
    let target = id
        .clone()
        .map(|id| EntityTarget::new(options.collection.clone(), id));
    let mode = if options.preview {
        RenderMode::Preview
    } else {
        RenderMode::Detail
    };

    rsx! {
        article { class: "semantic-entity-card",
            header { class: "semantic-entity-card__header",
                h3 { class: "semantic-entity-card__title",
                    if let Some(target) = target.clone() {
                        EntityLink { target, text: title.clone() }
                    } else {
                        span { "{title}" }
                    }
                }
                if let Some(id) = id.clone() {
                    code { class: "semantic-entity-card__id", "{id}" }
                }
            }
            div { class: "semantic-entity-card__body",
                match (options.renderer, class.clone()) {
                    (EntityDisplayRenderer::Custom, Some(class)) => rsx! {
                        ClassView {
                            class,
                            object: object.clone(),
                            collection: options.collection.clone(),
                            id: id.clone(),
                            mode
                        }
                    },
                    (EntityDisplayRenderer::Table, Some(class)) => rsx! {
                        ClassView {
                            class,
                            object: object.clone(),
                            collection: options.collection.clone(),
                            id: id.clone(),
                            mode: RenderMode::Detail
                        }
                    },
                    _ => rsx! {
                        ObjectView {
                            object: object.clone(),
                            mode: RenderMode::Detail
                        }
                    },
                }
            }
            if options.actions {
                if let Some(target) = target {
                    EntityActions {
                        target,
                        object: object.clone(),
                        placement: EntityActionPlacement::Card
                    }
                }
            }
        }
    }
}

#[component]
pub fn EntityTableRow(object: Object, collection: Option<String>) -> Element {
    let id = object_id(&object);
    rsx! {
        tr {
            td {
                if let Some(id) = id.clone() {
                    EntityLink {
                        target: EntityTarget::new(collection.clone(), id.clone()),
                        text: id
                    }
                }
            }
            td { "{object.get(\"type\").and_then(Value::as_str).unwrap_or_default()}" }
            td { "{object.len()}" }
            td {
                if let Some(id) = id {
                    EntityActions {
                        target: EntityTarget::new(collection.clone(), id),
                        object: object.clone(),
                        placement: EntityActionPlacement::BrowseRow
                    }
                }
            }
        }
    }
}

#[component]
pub fn EntityList(
    objects: Vec<Object>,
    display_mode: EntityDisplayMode,
    renderer: EntityDisplayRenderer,
    collection: Option<String>,
) -> Element {
    match display_mode {
        EntityDisplayMode::Card => rsx! {
            div { class: "semantic-entity-list semantic-entity-list--cards",
                for object in objects {
                    EntityCard {
                        object,
                        options: EntityRenderOptions {
                            collection: collection.clone(),
                            id: None,
                            renderer,
                            preview: true,
                            actions: true,
                        }
                    }
                }
            }
        },
        EntityDisplayMode::Table => rsx! {
            table { class: "semantic-entity-list semantic-entity-list--table",
                thead {
                    tr {
                        th { "id" }
                        th { "type" }
                        th { "fields" }
                        th { "actions" }
                    }
                }
                tbody {
                    for object in objects {
                        EntityTableRow {
                            object,
                            collection: collection.clone()
                        }
                    }
                }
            }
        },
    }
}

#[component]
fn EntityLink(target: EntityTarget, text: String) -> Element {
    let catalog = use_ui_catalog();
    let content = rsx! { span { "{text}" } };
    if let Some(renderer) = catalog.entity_navigation().link_renderer.as_ref() {
        return renderer(target, content);
    }
    if let Some(href) = catalog
        .entity_navigation()
        .href
        .as_ref()
        .and_then(|builder| builder(&target))
    {
        return rsx! { a { href: "{href}", "{text}" } };
    }
    content
}

#[component]
fn EntityActions(
    target: EntityTarget,
    object: Object,
    placement: EntityActionPlacement,
) -> Element {
    let catalog = use_ui_catalog();
    let class = catalog.object_class(&object).cloned();
    let ctx = EntityActionContext {
        target,
        object,
        class,
        placement,
    };
    let actions = catalog.entity_actions_for(&ctx);
    rsx! {
        div { class: "semantic-entity-actions",
            for action in actions {
                {(action.render)(ctx.clone())}
            }
        }
    }
}

fn object_id(object: &Object) -> Option<String> {
    object.get("id").and_then(Value::as_str).map(str::to_string)
}

fn entity_title(object: &Object, id: Option<&str>, class_name: Option<&str>) -> String {
    for field in ["title", "name", "semantic:title"] {
        if let Some(title) = object.get(field).and_then(Value::as_str)
            && !title.trim().is_empty()
        {
            return title.to_string();
        }
    }
    id.or(class_name).unwrap_or("Entity").to_string()
}
