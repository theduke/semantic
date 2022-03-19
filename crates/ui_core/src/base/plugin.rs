use std::rc::Rc;

use brass::dom::{builder::div, Attr, Tag, TagBuilder};
use factordb::prelude::{AttrMapExt, AttributeDescriptor, DataMap, EntityDescriptor, Id, Value};
use semantic_core::base::{self, AttrPreviewImageUrl};

use crate::{
    components::entity::{render_image, render_value, render_video},
    registry::RegisteredMediaRenderer,
    BrowserPlugin, BrowserPluginSpec, EntityRenderMode, EntityRendererSpec, Registry,
};

use super::collection::collection_create_page;

pub struct BasePlugin;

impl BrowserPlugin for BasePlugin {
    // fn init_ui_context(&self, _ctx: &brass::Context<()>) {}

    fn spec(&self) -> BrowserPluginSpec {
        BrowserPluginSpec {
            name: "base".to_string(),
            main_route: None,
        }
    }

    fn register(&self, registry: &mut Registry) {
        registry.register_attr_renderer(
            semantic_core::base::AttrPreviewImageUrl::QUALIFIED_NAME.to_string(),
            std::rc::Rc::new(render_attr_preview_image),
        );

        registry.register_attr_renderer(
            semantic_core::base::AttrBlobUri::QUALIFIED_NAME.to_string(),
            std::rc::Rc::new(render_blob_uri),
        );

        // Note

        registry.register_entity_renderer(EntityRendererSpec {
            name: "Create Note".to_string(),
            entity_type: base::Note::QUALIFIED_NAME.to_string(),
            mode: EntityRenderMode::CreatePage,
            renderer: Rc::new(super::note::note_create_page),
            is_default: false,
        });

        registry.register_entity_renderer(EntityRendererSpec {
            name: "Note View".to_string(),
            entity_type: base::Note::QUALIFIED_NAME.to_string(),
            mode: EntityRenderMode::Content,
            renderer: Rc::new(super::note::note_content),
            is_default: true,
        });

        // Files

        registry.register_entity_renderer(EntityRendererSpec {
            name: "Image Content".to_string(),
            entity_type: base::Image::QUALIFIED_NAME.to_string(),
            mode: EntityRenderMode::Content,
            renderer: Rc::new(super::file::image::image_content),
            is_default: true,
        });

        registry.register_entity_renderer(EntityRendererSpec {
            name: "Video Content".to_string(),
            entity_type: base::Video::QUALIFIED_NAME.to_string(),
            mode: EntityRenderMode::Content,
            renderer: Rc::new(super::file::video::video_content),
            is_default: true,
        });

        registry.register_media_renderer(RegisteredMediaRenderer {
            entity_type: base::Video::QUALIFIED_NAME.into(),
            render: Rc::new(super::file::video::video_media),
            supports_playback: true,
        });

        // // Notes.

        // registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
        //     name: "Note Body".to_string(),
        //     entity_type: base::Note::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::Content,
        //     renderer: Rc::new(super::notes::note_content),
        //     is_default: true,
        // });

        // // Habits.

        // registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
        //     name: "Create Habit".to_string(),
        //     entity_type: base::Habit::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::CreatePage,
        //     renderer: Rc::new(crate::components::base::habits::habit_create::habit_create),
        //     is_default: false,
        // });

        // // Collections.

        // // Content.
        registry.register_entity_renderer(EntityRendererSpec {
            name: "Collection View".to_string(),
            entity_type: base::Collection::QUALIFIED_NAME.to_string(),
            mode: EntityRenderMode::Content,
            renderer: Rc::new(super::collection::collection_content),
            is_default: true,
        });

        // // Create page.
        registry.register_entity_renderer(EntityRendererSpec {
            name: "Create Collection".to_string(),
            entity_type: base::Collection::QUALIFIED_NAME.to_string(),
            mode: EntityRenderMode::CreatePage,
            renderer: Rc::new(collection_create_page),
            is_default: false,
        });

        registry.ignore_entity_type(semantic_core::base::Tag::QUALIFIED_NAME.to_string());
        registry.ignore_entity_type(semantic_core::core::PluginSource::QUALIFIED_NAME.to_string());
    }
}

fn render_attr_preview_image(value: &Value, _entity: Option<&DataMap>) -> TagBuilder {
    tracing::trace!(?value, "rendering preview image");
    if let Some(url) = value.as_str().filter(|v| v.starts_with("http")) {
        div().and(
            Tag::Img
                .new()
                .attr(Attr::Src, url)
                .attr(Attr::Alt, "Preview Image")
                .style_raw("max-height: 200px;"),
        )
    } else {
        tracing::trace!(?value, "preview image does not start with http");
        let mut wrap = div();
        render_value(value, &mut wrap);
        wrap
    }
}

pub fn build_entity_blob_path(entity_id: Id) -> String {
    format!("/blob/file/{}", entity_id)
}

fn render_blob_uri(value: &Value, entity: Option<&DataMap>) -> TagBuilder {
    let id = entity.and_then(|e| e.get_id());

    if let (Some(id), Value::String(blob_path)) = (id, value) {
        // FIXME: use actual server URL!
        // Blocked on trunk proxy working - need to upgrade to 0.11.
        // let url = format!("/blob/{}", blob_uri);
        let url = build_entity_blob_path(id);

        // let ext = blob_uri.rsplit_once('.').map(|x| x.1);
        let ext = Some("");
        match ext {
            Some("jpg" | "jpeg") => render_image(&url, None, true),
            Some("mp4") => {
                let thumb = entity.and_then(|data| data.get_attr::<AttrPreviewImageUrl>());
                let thumb_url = thumb.as_ref().map(|t| t.as_str());

                render_video(&url, thumb_url, true)
            }
            _ => Tag::A
                .new()
                .attr(brass::dom::Attr::Href, url)
                .and(blob_path),
        }
    } else {
        let mut t = Tag::Span.new();
        render_value(value, &mut t);
        t
    }
}
