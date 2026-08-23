use std::rc::Rc;

use dioxus::prelude::{Element, EventHandler};
use semantic_data::value::Object;

use crate::ui_catalog::UiCatalog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    File,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaRenderOptions {
    pub playing: bool,
    pub muted: bool,
    pub controls: bool,
}

impl Default for MediaRenderOptions {
    fn default() -> Self {
        Self {
            playing: false,
            muted: false,
            controls: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaRenderEvent {
    Loaded,
    Finished,
    Paused,
    Resumed,
    Failed(String),
}

pub trait MediaHandle {
    fn play(&self);
    fn pause(&self);
    fn set_muted(&self, muted: bool);
}

pub type PlaybackSessionId = u64;

#[derive(Clone, Debug, PartialEq)]
pub enum MediaPlaybackEventKind {
    Mounted,
    Loaded {
        duration_seconds: Option<f64>,
    },
    Playing,
    Paused,
    Progress {
        current_seconds: f64,
        duration_seconds: Option<f64>,
    },
    Finished,
    Failed {
        message: String,
        fatal: bool,
    },
    PlayRejected {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaPlaybackEvent {
    pub session_id: PlaybackSessionId,
    pub occurrence_id: u64,
    pub kind: MediaPlaybackEventKind,
}

pub trait PlaybackMediaHandle {
    fn play(&self);
    fn pause(&self);
    fn set_muted(&self, muted: bool);
    fn seek(&self, seconds: f64);
}

#[derive(Clone)]
pub struct RegisteredPlaybackHandle(pub Rc<dyn PlaybackMediaHandle>);

impl PartialEq for RegisteredPlaybackHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, PartialEq)]
pub struct MediaHandleRegistration {
    pub session_id: PlaybackSessionId,
    pub occurrence_id: u64,
    pub handle: Option<RegisteredPlaybackHandle>,
}

#[derive(Clone, PartialEq)]
pub struct MediaPlaybackRenderOptions {
    pub session_id: PlaybackSessionId,
    pub occurrence_id: u64,
    pub playing: bool,
    pub muted: bool,
    pub controls: bool,
    pub on_handle: EventHandler<MediaHandleRegistration>,
    pub on_event: EventHandler<MediaPlaybackEvent>,
}

#[derive(Clone)]
pub struct MediaPlaybackRendererRegistration {
    pub name: String,
    pub class_id: Option<String>,
    pub media_kind: MediaKind,
    pub renderer: Rc<dyn Fn(Object, MediaPlaybackRenderOptions) -> Element>,
}

#[derive(Clone)]
pub struct MediaRendererRegistration {
    pub name: String,
    pub class_id: Option<String>,
    pub media_kind: MediaKind,
    pub supports_playback: bool,
    pub renderer: Rc<dyn Fn(Object, MediaRenderOptions) -> Element>,
}

impl UiCatalog {
    pub fn media_renderer_for_object(&self, object: &Object) -> Option<&MediaRendererRegistration> {
        let class_id = object
            .get("type")
            .and_then(semantic_data::value::Value::as_str);
        if let Some(class_id) = class_id {
            if let Some(renderer) = self.media_renderer_for_class(class_id) {
                return Some(renderer);
            }
        }
        let media_kind = media_kind_for_object(object);
        self.media_renderers()
            .iter()
            .find(|renderer| renderer.class_id.is_none() && renderer.media_kind == media_kind)
    }

    pub fn media_renderer_for_class(&self, class_id: &str) -> Option<&MediaRendererRegistration> {
        self.media_renderers()
            .iter()
            .find(|renderer| renderer.class_id.as_deref() == Some(class_id))
            .or_else(|| {
                self.media_renderers().iter().find(|renderer| {
                    renderer
                        .class_id
                        .as_deref()
                        .is_some_and(|renderer_class_id| {
                            renderer_class_id != class_id
                                && self.class_inherits(class_id, renderer_class_id)
                        })
                })
            })
    }

    pub fn playback_renderer_for_object(
        &self,
        object: &Object,
    ) -> Option<&MediaPlaybackRendererRegistration> {
        let class_id = object
            .get("type")
            .and_then(semantic_data::value::Value::as_str);
        if let Some(class_id) = class_id {
            if let Some(renderer) = self.playback_renderer_for_class(class_id) {
                return Some(renderer);
            }
        }
        let media_kind = media_kind_for_object(object);
        self.media_playback_renderers()
            .iter()
            .find(|renderer| renderer.class_id.is_none() && renderer.media_kind == media_kind)
    }

    pub fn playback_renderer_for_class(
        &self,
        class_id: &str,
    ) -> Option<&MediaPlaybackRendererRegistration> {
        self.media_playback_renderers()
            .iter()
            .find(|renderer| renderer.class_id.as_deref() == Some(class_id))
            .or_else(|| {
                self.media_playback_renderers().iter().find(|renderer| {
                    renderer
                        .class_id
                        .as_deref()
                        .is_some_and(|renderer_class_id| {
                            renderer_class_id != class_id
                                && self.class_inherits(class_id, renderer_class_id)
                        })
                })
            })
    }
}

pub fn media_kind_for_object(object: &Object) -> MediaKind {
    let content_type = object
        .get("mime_type")
        .or_else(|| object.get(semantic_data::filestore::ATTR_FILE_MIME_TYPE))
        .or_else(|| object.get("mime"))
        .or_else(|| object.get("content_type"))
        .and_then(semantic_data::value::Value::as_str)
        .unwrap_or_default();
    if content_type.starts_with("image/") {
        MediaKind::Image
    } else if content_type.starts_with("audio/") {
        MediaKind::Audio
    } else if content_type.starts_with("video/") {
        MediaKind::Video
    } else if object.contains_key("url")
        || object.contains_key("path")
        || object
            .get("type")
            .and_then(semantic_data::value::Value::as_str)
            == Some(semantic_data::filestore::FILE_CLASS_ID)
    {
        MediaKind::File
    } else {
        MediaKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use dioxus::prelude::*;
    use semantic_data::schema::{ClassRef, ClassType, Meta};
    use semantic_db_core::catalog::{CatalogStorageSnapshot, LocalClassId, StoredClass};

    use super::*;

    #[test]
    fn media_renderer_lookup_prefers_exact_then_inherited_then_none() {
        let mut catalog = UiCatalog::from_snapshot(snapshot_with_classes(vec![
            class("base", None),
            class("child", Some("base")),
            class("exact", Some("base")),
        ]));
        catalog.register_media_renderer(renderer("base", Some("base")));
        catalog.register_media_renderer(renderer("exact", Some("exact")));

        assert_eq!(
            catalog
                .media_renderer_for_class("exact")
                .map(|renderer| renderer.name.as_str()),
            Some("exact")
        );
        assert_eq!(
            catalog
                .media_renderer_for_class("child")
                .map(|renderer| renderer.name.as_str()),
            Some("base")
        );
        assert!(catalog.media_renderer_for_class("missing").is_none());
    }

    #[test]
    fn media_kind_recognizes_current_filestore_mime_keys() {
        for key in ["mime_type", semantic_data::filestore::ATTR_FILE_MIME_TYPE] {
            let mut object = Object::new();
            object.insert(
                key,
                semantic_data::value::Value::String("audio/ogg".to_string()),
            );
            assert_eq!(media_kind_for_object(&object), MediaKind::Audio);
        }
    }

    #[test]
    fn playback_renderer_lookup_prefers_exact_then_inherited() {
        let mut catalog = UiCatalog::from_snapshot(snapshot_with_classes(vec![
            class("base", None),
            class("child", Some("base")),
            class("exact", Some("base")),
        ]));
        catalog.register_media_playback_renderer(playback_renderer("base", Some("base")));
        catalog.register_media_playback_renderer(playback_renderer("exact", Some("exact")));
        assert_eq!(
            catalog
                .playback_renderer_for_class("exact")
                .map(|renderer| renderer.name.as_str()),
            Some("exact")
        );
        assert_eq!(
            catalog
                .playback_renderer_for_class("child")
                .map(|renderer| renderer.name.as_str()),
            Some("base")
        );
    }

    #[test]
    fn current_file_without_playable_mime_is_a_file() {
        let mut object = Object::new();
        object.insert(
            "type",
            semantic_data::value::Value::String(
                semantic_data::filestore::FILE_CLASS_ID.to_string(),
            ),
        );
        object.insert(
            "mime_type",
            semantic_data::value::Value::String("application/pdf".to_string()),
        );
        assert_eq!(media_kind_for_object(&object), MediaKind::File);
    }

    fn renderer(name: &str, class_id: Option<&str>) -> MediaRendererRegistration {
        MediaRendererRegistration {
            name: name.to_string(),
            class_id: class_id.map(str::to_string),
            media_kind: MediaKind::Unknown,
            supports_playback: false,
            renderer: Rc::new(|_, _| rsx! { span {} }),
        }
    }

    fn playback_renderer(name: &str, class_id: Option<&str>) -> MediaPlaybackRendererRegistration {
        MediaPlaybackRendererRegistration {
            name: name.to_string(),
            class_id: class_id.map(str::to_string),
            media_kind: MediaKind::Unknown,
            renderer: Rc::new(|_, _| rsx! { span {} }),
        }
    }

    fn class(id: &str, inherits: Option<&str>) -> ClassType {
        ClassType {
            id: id.to_string(),
            name: id.to_string(),
            inherits: inherits.map(|id| ClassRef { id: id.to_string() }),
            extends: Vec::new(),
            attributes: BTreeMap::new(),
            constraints: Vec::new(),
            meta: Meta::default(),
        }
    }

    fn snapshot_with_classes(classes: Vec<ClassType>) -> CatalogStorageSnapshot {
        CatalogStorageSnapshot {
            attributes: Vec::new(),
            type_defs: Vec::new(),
            record_types: Vec::new(),
            classes: classes
                .into_iter()
                .enumerate()
                .map(|(index, class)| StoredClass {
                    lid: LocalClassId::from(index),
                    class,
                })
                .collect(),
            collections: Vec::new(),
            indexes: Vec::new(),
            relationships: Vec::new(),
            packages: Vec::new(),
            applied_migrations: Vec::new(),
            next_field_id: 0,
            auto_index_enabled: false,
        }
    }
}
