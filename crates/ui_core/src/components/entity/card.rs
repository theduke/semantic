use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_icons::lucide::Pencil;
use semantic_data::schema::ClassType;
use semantic_data::value::{Object, Value};

use crate::components::ClassView;
use crate::ui_catalog::{
    EntityActionContext, EntityActionPlacement, EntityTarget, RenderMode, use_ui_catalog,
};

/// Object fields considered, in priority order, when deriving an entity title.
pub const ENTITY_TITLE_FIELDS: [&str; 8] = [
    "semantic:base:label:name",
    "semantic:title",
    "title",
    "name",
    "display_name",
    "semantic:base:person:display_name",
    "semantic:base:file:filename",
    "filename",
];

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
pub fn EntityCard(
    object: Object,
    options: EntityRenderOptions,
    #[props(default)] on_edit: Option<EventHandler<EntityTarget>>,
    #[props(default)] on_delete: Option<EventHandler<EntityTarget>>,
    #[props(default = EntityActionPlacement::Card)] action_placement: EntityActionPlacement,
    #[props(default)] excluded_action_ids: Vec<String>,
) -> Element {
    let catalog = use_ui_catalog();
    let class = catalog.object_class(&object).cloned();
    let id = options.id.clone().or_else(|| object_id(&object));
    let class_name = class.as_ref().map(|class| {
        class
            .meta
            .title
            .clone()
            .unwrap_or_else(|| class.name.clone())
    });
    let title = entity_title(&object, id.as_deref(), class_name.as_deref());
    let show_id = id.as_deref().is_some_and(|id| id != title);
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
            dxcomp::Card {
                dxcomp::CardHeader {
                    div { class: "semantic-entity-card__header-row",
                        div { class: "semantic-entity-card__title-block",
                            dxcomp::CardTitle {
                                if let Some(target) = target.clone() {
                                    EntityLink { target, text: title.clone() }
                                } else {
                                    span { "{title}" }
                                }
                            }
                            if class_name.is_some() || show_id {
                                dxcomp::CardDescription {
                                    div { class: "semantic-entity-card__meta",
                                        if let Some(class_name) = class_name.clone() {
                                            span { class: "semantic-entity-card__type", "{class_name}" }
                                        }
                                        if show_id {
                                            if let Some(id) = id.clone() {
                                                code { class: "semantic-entity-card__id", "{id}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if options.actions {
                            if let Some(target) = target.clone() {
                                EntityActions {
                                    target,
                                    object: object.clone(),
                                    on_edit,
                                    on_delete,
                                    placement: action_placement,
                                    excluded_action_ids,
                                }
                            }
                        }
                    }
                }
                dxcomp::CardContent {
                    div { class: "semantic-entity-card__body",
                        EntityRenderBody {
                            object: Rc::new(object.clone()),
                            class: class.clone(),
                            collection: options.collection.clone(),
                            id: id.clone(),
                            renderer: options.renderer,
                            mode,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EntityRenderBody(
    object: Rc<Object>,
    class: Option<ClassType>,
    collection: Option<String>,
    id: Option<String>,
    renderer: EntityDisplayRenderer,
    mode: RenderMode,
) -> Element {
    match (renderer, class) {
        (EntityDisplayRenderer::Custom, Some(class)) => rsx! {
            ClassView {
                class,
                object: object.as_ref().clone(),
                collection,
                id,
                mode,
            }
        },
        (EntityDisplayRenderer::Table, Some(class)) => rsx! {
            ClassView {
                class,
                object: object.as_ref().clone(),
                collection,
                id,
                mode: RenderMode::Detail,
            }
        },
        _ => rsx! {
            EntityPreviewBody {
                object: object.as_ref().clone(),
            }
        },
    }
}

#[component]
pub fn EntityTableRow(object: Object, collection: Option<String>) -> Element {
    let catalog = use_ui_catalog();
    let class = catalog.object_class(&object).cloned();
    let id = object_id(&object);
    let class_name = class.as_ref().map(|class| {
        class
            .meta
            .title
            .clone()
            .unwrap_or_else(|| class.name.clone())
    });
    let title = entity_title(&object, id.as_deref(), class_name.as_deref());
    let type_label = class_name.unwrap_or_else(|| {
        object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    });
    rsx! {
        tr {
            td { class: "semantic-entity-list__id-cell",
                if let Some(id) = id.clone() {
                    EntityLink {
                        target: EntityTarget::new(collection.clone(), id.clone()),
                        text: id
                    }
                }
            }
            td { class: "semantic-entity-list__title-cell",
                if let Some(id) = id.clone() {
                    EntityLink {
                        target: EntityTarget::new(collection.clone(), id),
                        text: title
                    }
                } else {
                    span { "{title}" }
                }
            }
            td { class: "semantic-entity-list__type-cell", "{type_label}" }
            td { class: "semantic-entity-list__actions-cell",
                if let Some(id) = id.clone() {
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
                        th { "title" }
                        th { "type" }
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
    #[props(default)] on_edit: Option<EventHandler<EntityTarget>>,
    on_delete: Option<EventHandler<EntityTarget>>,
    placement: EntityActionPlacement,
    #[props(default)] excluded_action_ids: Vec<String>,
) -> Element {
    let catalog = use_ui_catalog();
    let class = catalog.object_class(&object).cloned();
    let ctx = EntityActionContext {
        target,
        object,
        class,
        placement,
        on_delete,
    };
    let mut actions = catalog.entity_actions_for(&ctx);
    actions.retain(|action| !excluded_action_ids.contains(&action.id));
    actions.sort_by_key(|action| entity_action_order(&action.id));
    rsx! {
        dxcomp::Toolbar {
            class: "semantic-entity-actions",
            aria_label: "Entity actions",
            dxcomp::ToolbarGroup {
                for action in actions {
                    if action.id == "edit" {
                        if let Some(on_edit) = on_edit {
                            EntityEditButton {
                                target: ctx.target.clone(),
                                on_edit,
                            }
                        } else {
                            {(action.render)(ctx.clone())}
                        }
                    } else {
                        {(action.render)(ctx.clone())}
                    }
                }
            }
        }
    }
}

#[component]
fn EntityEditButton(target: EntityTarget, on_edit: EventHandler<EntityTarget>) -> Element {
    rsx! {
        dxcomp::Button {
            variant: dxcomp::ButtonVariant::Outline,
            size: dxcomp::ButtonSize::IconSm,
            title: "Edit",
            aria_label: "Edit entity",
            onclick: move |_| on_edit.call(target.clone()),
            Pencil { size: "1rem" }
        }
    }
}

fn entity_action_order(action_id: &str) -> u8 {
    match action_id {
        "open" => 0,
        "edit" => 1,
        "delete" => 2,
        "open_external" => 3,
        _ => 10,
    }
}

#[component]
fn EntityPreviewBody(object: Object) -> Element {
    let fields = object
        .iter()
        .map(|(key, value)| (key.clone(), value_preview(value)))
        .collect::<Vec<_>>();

    rsx! {
        if fields.is_empty() {
            div { class: "semantic-entity-card__empty", "No preview fields" }
        } else {
            div { class: "semantic-table-wrap semantic-entity-card__field-table-wrap",
                table { class: "semantic-field-table semantic-entity-card__field-table",
                    tbody {
                        for (key, value) in fields {
                            tr {
                                th { scope: "row", "{key}" }
                                td { "{value}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn object_id(object: &Object) -> Option<String> {
    object.get("id").and_then(Value::as_str).map(str::to_string)
}

pub fn entity_title(object: &Object, id: Option<&str>, class_name: Option<&str>) -> String {
    for field in ENTITY_TITLE_FIELDS {
        if let Some(title) = object.get(field).and_then(Value::as_str)
            && !title.trim().is_empty()
        {
            return title.to_string();
        }
    }
    id.or(class_name).unwrap_or("Entity").to_string()
}

fn value_preview(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
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
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::value::{Object, Value};

    use super::entity_title;

    #[test]
    fn entity_title_prefers_semantic_identity_fields_and_falls_back_to_id() {
        let mut object = Object::new();
        object.insert("filename", Value::String("photo.jpg".to_string()));
        assert_eq!(entity_title(&object, Some("file-1"), None), "photo.jpg");

        object.insert("name", Value::String("Summer photo".to_string()));
        assert_eq!(entity_title(&object, Some("file-1"), None), "Summer photo");

        object.insert("semantic:title", Value::String("Featured".to_string()));
        assert_eq!(entity_title(&object, Some("file-1"), None), "Featured");
        assert_eq!(
            entity_title(&Object::new(), Some("entity-1"), None),
            "entity-1"
        );
    }
}
