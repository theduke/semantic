use std::rc::Rc;

use brass::vdom;
use factordb::{
    data::{DataMap, Value},
    schema::{AttrMapExt, AttributeDescriptor, EntityDescriptor},
};
use semantic_core::base::{self, AttrPreviewImageUrl};
use semantic_ui_core::registry::RegisteredMediaRenderer;
use wasm_bindgen::convert::IntoWasmAbi;

use crate::components::entity::{self, render_value};

pub struct BasePlugin;

impl semantic_ui_core::BrowserPlugin for BasePlugin {
    fn spec(&self) -> semantic_ui_core::BrowserPluginSpec {
        semantic_ui_core::BrowserPluginSpec {
            name: "base".to_string(),
        }
    }

    fn register(&self, registry: &mut semantic_ui_core::Registry) {
        registry.register_attr_renderer(
            semantic_core::base::AttrPreviewImageUrl::QUALIFIED_NAME.to_string(),
            std::rc::Rc::new(render_attr_preview_image),
        );

        registry.register_attr_renderer(
            semantic_core::base::AttrBlobUri::QUALIFIED_NAME.to_string(),
            std::rc::Rc::new(render_blob_uri),
        );

        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Create Note".to_string(),
            entity_type: base::Note::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::CreatePage,
            renderer: Rc::new(crate::components::base::notes::note_create::note_create),
            is_default: false,
        });

        // registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
        //     name: "Update Note".to_string(),
        //     entity_type: base::Note::QUALIFIED_NAME.to_string(),
        //     mode: semantic_ui_core::EntityRenderMode::ViewPage,
        //     renderer: Rc::new(crate::components::base::notes::note_update::note_update),
        //     is_default: true,
        // });

        // Files

        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Image Content".to_string(),
            entity_type: base::Image::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::Content,
            renderer: Rc::new(super::files::image::image_content),
            is_default: true,
        });

        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Video Content".to_string(),
            entity_type: base::Video::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::Content,
            renderer: Rc::new(super::files::video::video_content),
            is_default: true,
        });

        registry.register_media_renderer(RegisteredMediaRenderer {
            entity_type: base::Video::QUALIFIED_NAME.into(),
            render: Rc::new(super::files::video::video_media),
            supports_playback: true,
        });

        // Notes.

        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Note Body".to_string(),
            entity_type: base::Note::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::Content,
            renderer: Rc::new(super::notes::note_content),
            is_default: true,
        });

        // Habits.

        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Create Habit".to_string(),
            entity_type: base::Habit::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::CreatePage,
            renderer: Rc::new(crate::components::base::habits::habit_create::habit_create),
            is_default: false,
        });

        // Collections.

        // Content.
        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Collection View".to_string(),
            entity_type: base::Collection::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::Content,
            renderer: Rc::new(super::collections::collection_content),
            is_default: true,
        });

        // Create page.
        registry.register_entity_renderer(semantic_ui_core::EntityRendererSpec {
            name: "Create Collection".to_string(),
            entity_type: base::Collection::QUALIFIED_NAME.to_string(),
            mode: semantic_ui_core::EntityRenderMode::CreatePage,
            renderer: Rc::new(
                crate::components::base::collections::collection_create::collection_create,
            ),
            is_default: false,
        });
    }

    fn can_import_url(&self, _url: &str) -> bool {
        false
    }

    fn import(
        &self,
        _url: url::Url,
        _api: &semantic_ui_core::api::BrowserApiClient,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<factordb::query::select::ItemPage, factordb::AnyError>,
                > + 'static,
        >,
    > {
        unimplemented!()
    }
}

fn render_attr_preview_image(value: &Value, _entity: Option<&DataMap>) -> brass::VNode {
    if let Value::String(v) = value {
        vdom::div()
            .and(vdom::img(v).style_raw("max-height: 200px;"))
            .build()
    } else {
        render_value(value)
    }
}

pub fn build_blob_url(uri: &str) -> String {
    format!("http://localhost:3000/blob/{}", uri)
}

fn render_blob_uri(value: &Value, entity: Option<&DataMap>) -> brass::VNode {
    if let Value::String(blob_uri) = value {
        // FIXME: use actual server URL!
        // Blocked on trunk proxy working - need to upgrade to 0.11.
        // let url = format!("/blob/{}", blob_uri);
        let url = build_blob_url(&blob_uri);

        let ext = blob_uri.rsplit_once('.').map(|x| x.1);
        match ext {
            Some("jpg" | "jpeg") => entity::render_image(&url, None, true).build(),
            Some("mp4") => {
                let thumb = entity.and_then(|data| data.get_attr::<AttrPreviewImageUrl>());
                let thumb_url = thumb.as_ref().map(|t| t.as_str());

                entity::render_video(&url, thumb_url, true).build()
            }
            _ => {
                let filename = blob_uri.rsplit('/').next().unwrap();
                // TODO: use link?
                vdom::a()
                    .attr(brass::dom::Attr::Href, url)
                    .and(filename)
                    .build()
            }
        }
    } else {
        render_value(value)
    }
}
