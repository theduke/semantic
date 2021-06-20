use brass::vdom;
use factordb::{
    data::{DataMap, Value},
    schema::{AttrMapExt, AttributeDescriptor},
};
use semantics_core::base::AttrPreviewImageUrl;

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
            semantics_core::base::AttrPreviewImageUrl::QUALIFIED_NAME.to_string(),
            std::rc::Rc::new(render_attr_preview_image),
        );

        registry.register_attr_renderer(
            semantics_core::base::AttrBlobUri::QUALIFIED_NAME.to_string(),
            std::rc::Rc::new(render_blob_uri),
        );
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

fn render_blob_uri(value: &Value, entity: Option<&DataMap>) -> brass::VNode {
    if let Value::String(blob_uri) = value {
        // FIXME: use actual server URL!
        // Blocked on trunk proxy working - need to upgrade to 0.11.
        // let url = format!("/blob/{}", blob_uri);
        let url = format!("http://localhost:3000/blob/{}", blob_uri);

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
