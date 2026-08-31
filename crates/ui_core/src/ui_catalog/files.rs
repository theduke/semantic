use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::builtin::ATTR_ID;
use semantic_data::filestore::{
    ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILENAME, ATTR_TITLE, FILE_CLASS_ID,
};
use semantic_data::value::{Object, Value};

use crate::components::ValueView;
use crate::ui_catalog::{
    ClassRenderContext, MediaKind, RenderMode, UiCatalog, media_kind_for_object, use_ui_catalog,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileViewMode {
    Media,
    Data,
}

pub(crate) fn register_file_renderers(catalog: &mut UiCatalog) {
    catalog
        .render_registry_mut()
        .register_class_renderer(FILE_CLASS_ID, Rc::new(render_file_class));
}

fn render_file_class(ctx: ClassRenderContext) -> Element {
    rsx! {
        FileDetailView {
            object: ctx.object,
            id: ctx.id,
            mode: ctx.mode,
        }
    }
}

#[component]
fn FileDetailView(object: Object, id: Option<String>, mode: RenderMode) -> Element {
    let catalog = use_ui_catalog();
    let file_id = id.or_else(|| object_string(&object, &[ATTR_ID, "id"]).map(str::to_string));
    let title = object_string(
        &object,
        &[ATTR_FILE_FILENAME, "filename", ATTR_TITLE, "title"],
    )
    .or(file_id.as_deref())
    .unwrap_or("File")
    .to_string();
    let media_kind = media_kind_for_object(&object);
    let is_visual_media = matches!(media_kind, MediaKind::Image | MediaKind::Video);
    let can_show_data = is_visual_media && has_blob_hash(&object);
    let initial_mode = if is_visual_media {
        FileViewMode::Media
    } else {
        FileViewMode::Data
    };
    let mut view_mode = use_signal(move || initial_mode);
    let show_media = is_visual_media && (!can_show_data || view_mode() == FileViewMode::Media);
    let source = file_id.map(|id| {
        format!(
            "{}/{}",
            catalog
                .render_settings()
                .file_api_prefix
                .trim_end_matches('/'),
            id
        )
    });

    rsx! {
        article { class: "semantic-file-detail",
            header { class: "semantic-file-detail__header",
                div { class: "semantic-file-detail__title",
                    h2 { "{title}" }
                    if let Some(id) = object_string(&object, &[ATTR_ID, "id"]) {
                        code { "{id}" }
                    }
                }
            }
            if can_show_data || source.is_some() {
                div {
                    class: "semantic-file-detail__toolbar",
                    role: "group",
                    aria_label: "File actions",
                    if can_show_data {
                        dxcomp::Button {
                            r#type: "button",
                            variant: if show_media { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            size: dxcomp::ButtonSize::Sm,
                            aria_pressed: show_media,
                            onclick: move |_| view_mode.set(FileViewMode::Media),
                            "Media"
                        }
                        dxcomp::Button {
                            r#type: "button",
                            variant: if show_media { dxcomp::ButtonVariant::Outline } else { dxcomp::ButtonVariant::Primary },
                            size: dxcomp::ButtonSize::Sm,
                            aria_pressed: !show_media,
                            onclick: move |_| view_mode.set(FileViewMode::Data),
                            "Data"
                        }
                    }
                    if let Some(open_source) = source.clone() {
                        a {
                            class: "dx-button semantic-file-detail__open",
                            "data-style": "outline",
                            "data-size": "sm",
                            href: open_source,
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "Open"
                        }
                    }
                }
            }
            if show_media {
                if let Some(source) = source {
                    match media_kind {
                        MediaKind::Image => rsx! {
                            FileImageView { source, title: title.clone() }
                        },
                        MediaKind::Video => rsx! {
                            video {
                                class: "semantic-file-detail__video",
                                src: source,
                                controls: true,
                                preload: "metadata",
                                aria_label: "Video: {title}",
                            }
                        },
                        _ => rsx! {},
                    }
                } else {
                    p { class: "semantic-text-muted", "This file has no serving URL." }
                }
            } else {
                FileDataView { object: object.clone(), mode }
            }
        }
    }
}

#[component]
fn FileImageView(source: String, title: String) -> Element {
    let mut dialog_open = use_signal(|| false);
    let preview_source = source.clone();
    rsx! {
        button {
            class: "semantic-file-detail__image-button",
            r#type: "button",
            aria_label: "View {title} at full size",
            onclick: move |_| dialog_open.set(true),
            img {
                class: "semantic-file-detail__image",
                src: preview_source,
                alt: "{title}",
            }
        }
        dxcomp::Dialog {
            class: "semantic-file-detail__image-dialog",
            open: dialog_open(),
            on_open_change: move |open: bool| dialog_open.set(open),
            dxcomp::DialogTitle { "{title}" }
            div { class: "semantic-file-detail__image-dialog-body",
                img {
                    class: "semantic-file-detail__image-full",
                    src: source,
                    alt: "{title}",
                }
            }
            div { class: "semantic-file-detail__dialog-actions",
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    onclick: move |_| dialog_open.set(false),
                    "Close"
                }
            }
        }
    }
}

#[component]
fn FileDataView(object: Object, mode: RenderMode) -> Element {
    rsx! {
        div { class: "semantic-table-wrap semantic-file-detail__data",
            table { class: "semantic-field-table semantic-field-table--object",
                tbody {
                    for (key, value) in object.iter() {
                        tr {
                            th { scope: "row", "{key}" }
                            td {
                                ValueView {
                                    value: value.clone(),
                                    type_hint: None,
                                    mode,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn has_blob_hash(object: &Object) -> bool {
    object_string(
        object,
        &[ATTR_FILE_CONTENT_HASH_SHA256, "content_hash_sha256"],
    )
    .is_some_and(|hash| !hash.trim().is_empty())
}

fn object_string<'a>(object: &'a Object, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use semantic_data::value::{Object, Value};

    #[test]
    fn blob_hash_requires_non_empty_content_hash() {
        let mut object = Object::new();
        assert!(!super::has_blob_hash(&object));

        object.insert("content_hash_sha256", Value::String("  ".to_string()));
        assert!(!super::has_blob_hash(&object));

        object.insert("content_hash_sha256", Value::String("abc123".to_string()));
        assert!(super::has_blob_hash(&object));
    }
}
