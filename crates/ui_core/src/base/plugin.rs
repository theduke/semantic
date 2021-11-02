use std::rc::Rc;

use brass::dom::{builder::div, Tag, TagBuilder};
use factordb::{Value, data::DataMap, schema::{AttrMapExt, AttributeDescriptor, EntityDescriptor}};
use semantic_core::base::{self, AttrPreviewImageUrl};

use crate::{BrowserPlugin, BrowserPluginSpec, EntityRenderMode, EntityRendererSpec, Registry, components::entity::{render_image, render_value, render_video}};

pub struct BasePlugin;

impl BrowserPlugin for BasePlugin {
    // fn init_ui_context(&self, _ctx: &brass::Context<()>) {}

    fn spec(&self) -> BrowserPluginSpec {
        BrowserPluginSpec {
            name: "base".to_string(),
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

        // registry.register_entity_renderer(EntityRendererSpec {
        //     name: "Create Note".to_string(),
        //     entity_type: base::Note::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::CreatePage,
        //     renderer: Rc::new(crate::components::base::notes::note_create::note_create),
        //     is_default: false,
        // });

        // registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
        //     name: "Update Note".to_string(),
        //     entity_type: base::Note::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::ViewPage,
        //     renderer: Rc::new(crate::components::base::notes::note_update::note_update),
        //     is_default: true,
        // });

        // Files

        registry.register_entity_renderer(EntityRendererSpec {
            name: "Image Content".to_string(),
            entity_type: base::Image::QUALIFIED_NAME.to_string(),
            mode: EntityRenderMode::Content,
            renderer: Rc::new(super::file::image_content),
            is_default: true,
        });

        // registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
        //     name: "Video Content".to_string(),
        //     entity_type: base::Video::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::Content,
        //     renderer: Rc::new(super::files::video::video_content),
        //     is_default: true,
        // });

        // registry.register_media_renderer(RegisteredMediaRenderer {
        //     entity_type: base::Video::QUALIFIED_NAME.into(),
        //     render: Rc::new(super::files::video::video_media),
        //     supports_playback: true,
        // });

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
        // registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
        //     name: "Collection View".to_string(),
        //     entity_type: base::Collection::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::Content,
        //     renderer: Rc::new(super::collections::collection_content),
        //     is_default: true,
        // });

        // // Create page.
        // registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
        //     name: "Create Collection".to_string(),
        //     entity_type: base::Collection::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::CreatePage,
        //     renderer: Rc::new(
        //         crate::components::base::collections::collection_create::collection_create,
        //     ),
        //     is_default: false,
        // });
    }
}

fn render_attr_preview_image(value: &Value, _entity: Option<&DataMap>) -> TagBuilder {
    if let Value::String(v) = value {
        div().and(Tag::Image.new().style_raw("max-height: 200px;"))
    } else {
        let mut wrap = div();
        render_value(value, &mut wrap);
        wrap
    }
}

pub fn build_blob_url(uri: &str) -> String {
    format!("http://localhost:3000/blob/{}", uri)
}

fn render_blob_uri(value: &Value, entity: Option<&DataMap>) -> TagBuilder {
    if let Value::String(blob_uri) = value {
        // FIXME: use actual server URL!
        // Blocked on trunk proxy working - need to upgrade to 0.11.
        // let url = format!("/blob/{}", blob_uri);
        let url = build_blob_url(&blob_uri);

        let ext = blob_uri.rsplit_once('.').map(|x| x.1);
        match ext {
            Some("jpg" | "jpeg") => render_image(&url, None, true),
            Some("mp4") => {
                let thumb = entity.and_then(|data| data.get_attr::<AttrPreviewImageUrl>());
                let thumb_url = thumb.as_ref().map(|t| t.as_str());

                render_video(&url, thumb_url, true)
            }
            _ => {
                let filename = blob_uri.rsplit('/').next().unwrap();
                // TODO: use link?
                Tag::A.new().attr(brass::dom::Attr::Href, url).and(filename)
            }
        }
    } else {
        let mut t = Tag::Span.new();
        render_value(value, &mut t);
        t
    }
}
