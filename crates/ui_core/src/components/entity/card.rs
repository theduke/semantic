use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_icons::lucide::{File, Pencil};
use semantic_data::filestore::{ATTR_FILE_FILENAME, FILE_CLASS_ID};
use semantic_data::schema::ClassType;
use semantic_data::value::{Object, Value};

use crate::components::{ClassView, ImageLightbox};
use crate::ui_catalog::{
    EntityActionContext, EntityActionPlacement, EntityTarget, MediaKind, RenderMode,
    media_kind_for_object, use_ui_catalog,
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
    #[props(default)] compact_preview: bool,
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
    let title = if compact_preview && id.as_deref() == Some(title.as_str()) {
        format!(
            "Untitled {}",
            class_name.as_deref().unwrap_or("entity").to_lowercase()
        )
    } else {
        title
    };
    let show_id = !compact_preview && id.as_deref().is_some_and(|id| id != title);
    let target = id
        .clone()
        .map(|id| EntityTarget::new(options.collection.clone(), id));
    let mode = if options.preview {
        RenderMode::Preview
    } else {
        RenderMode::Detail
    };

    rsx! {
        article { class: if compact_preview { "semantic-entity-card semantic-entity-card--summary" } else { "semantic-entity-card" },
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
                        if compact_preview && options.renderer == EntityDisplayRenderer::Custom {
                            EntitySummary { object: object.clone(), id: id.clone(), title: title.clone() }
                        } else {
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
}

#[component]
fn EntitySummary(object: Object, id: Option<String>, title: String) -> Element {
    let catalog = use_ui_catalog();
    let file_kind = (object.get("type").and_then(Value::as_str) == Some(FILE_CLASS_ID))
        .then(|| media_kind_for_object(&object));
    let media_source = matches!(file_kind, Some(MediaKind::Image | MediaKind::Audio))
        .then(|| {
            id.as_deref().map(|id| {
                format!(
                    "{}/{}",
                    catalog
                        .render_settings()
                        .file_api_prefix
                        .trim_end_matches('/'),
                    id
                )
            })
        })
        .flatten();
    let excerpt = entity_excerpt(&object, &title);

    rsx! {
        div { class: "semantic-entity-card__summary",
            if let Some(source) = media_source {
                if file_kind == Some(MediaKind::Image) {
                    ImageLightbox {
                        source,
                        title: title.clone(),
                        preview_class: "semantic-entity-card__thumbnail".to_string(),
                        trigger_class: "semantic-entity-card__thumbnail-button".to_string(),
                    }
                } else {
                    audio {
                        class: "semantic-entity-card__audio",
                        src: source,
                        controls: true,
                        preload: "metadata",
                        aria_label: "Audio: {title}",
                    }
                }
            } else if file_kind == Some(MediaKind::File) {
                div { class: "semantic-entity-card__file-fallback",
                    File { size: "1.5rem" }
                    div {
                        strong { "{title}" }
                        span { "File" }
                    }
                }
            }
            if let Some(excerpt) = excerpt {
                p { class: "semantic-entity-card__excerpt", "{excerpt}" }
            }
        }
    }
}

fn entity_excerpt(object: &Object, title: &str) -> Option<String> {
    const DESCRIPTION_FIELDS: [&str; 8] = [
        "semantic:base:note:note_content",
        "note_content",
        "semantic:description",
        "description",
        "summary",
        "excerpt",
        "content",
        "body",
    ];
    DESCRIPTION_FIELDS.iter().find_map(|field| {
        let value = object.get(*field).and_then(Value::as_str)?.trim();
        let text = value
            .lines()
            .map(|line| line.trim().trim_start_matches(['#', '*', '-', ' ']))
            .find(|line| !line.is_empty() && *line != title)?;
        let excerpt: String = text.chars().take(180).collect();
        (!excerpt.is_empty()).then_some(excerpt)
    })
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
            td { class: "semantic-entity-list__id-cell",
                if let Some(id) = id.clone() {
                    EntityLink {
                        target: EntityTarget::new(collection.clone(), id.clone()),
                        text: id
                    }
                }
            }
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
                        th { "title" }
                        th { "type" }
                        th { "id" }
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
    let placement_class = match placement {
        EntityActionPlacement::Card => "card",
        EntityActionPlacement::Detail => "detail",
        EntityActionPlacement::BrowseRow => "row",
    };
    rsx! {
        dxcomp::Toolbar {
            class: "semantic-entity-actions semantic-entity-actions--{placement_class}",
            aria_label: "Entity actions",
            dxcomp::ToolbarGroup {
                for action in actions {
                    div {
                        class: "semantic-entity-actions__item",
                        "data-action-id": action.id.clone(),
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
    let catalog = use_ui_catalog();
    let fields = object
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                catalog.attribute_title(key),
                value_preview(value),
            )
        })
        .collect::<Vec<_>>();

    rsx! {
        if fields.is_empty() {
            div { class: "semantic-entity-card__empty", "No preview fields" }
        } else {
            div { class: "semantic-table-wrap semantic-entity-card__field-table-wrap",
                table { class: "semantic-field-table semantic-entity-card__field-table",
                    tbody {
                    for (key, label, value) in fields {
                        tr {
                            th { scope: "row", title: "{key}", "{label}" }
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
    if object.get("type").and_then(Value::as_str) == Some(FILE_CLASS_ID)
        && let Some(filename) = object.get(ATTR_FILE_FILENAME).and_then(Value::as_str)
        && !filename.trim().is_empty()
    {
        return filename.to_string();
    }
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
    use semantic_data::{
        filestore::{ATTR_FILE_FILENAME, FILE_CLASS_ID},
        value::{Object, Value},
    };

    use super::{entity_excerpt, entity_title};

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

    #[test]
    fn summary_uses_document_content_without_repeating_title() {
        let mut object = Object::new();
        object.insert(
            "note_content",
            Value::String("# Roadmap\n\nFirst draft of the plan".to_string()),
        );
        assert_eq!(
            entity_excerpt(&object, "Roadmap"),
            Some("First draft of the plan".to_string())
        );
        object.insert("note_content", Value::Null);
        assert_eq!(entity_excerpt(&object, "Roadmap"), None);
    }

    #[test]
    fn file_title_prefers_the_canonical_filename_attribute() {
        let mut object = Object::new();
        object.insert("type", Value::String(FILE_CLASS_ID.to_string()));
        object.insert("title", Value::String("Untitled file".to_string()));
        object.insert(
            ATTR_FILE_FILENAME,
            Value::String("recording.wav".to_string()),
        );

        assert_eq!(
            entity_title(&object, Some("file-id"), Some("File")),
            "recording.wav"
        );
    }
}
