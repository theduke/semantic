use brass::{
    vdom::{self, span_with, Render},
    VNode,
};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_core::base::{AttrBlobUri, AttrDownloadUrl};
use semantic_ui_core::EntityRenderOpts;

use crate::components::base::plugin::build_blob_url;

pub fn image_content(item: &Item, opts: &EntityRenderOpts) -> VNode {
    let url = if let Some(blob_uri) = item.data.get_attr::<AttrBlobUri>() {
        Some(build_blob_url(&blob_uri))
    } else if let Some(url) = item.data.get_attr::<AttrDownloadUrl>() {
        Some(url.to_string())
    } else {
        None
    };

    if let Some(url) = url {
        if opts.preview {
            ImagePreviewModal { url: url.clone() }.render()
        } else {
            vdom::img(url)
                .style_raw("max-height: 100%; max-width: 100%; object-fit: contain;")
                .build()
        }
    } else {
        span_with("Image without url").build()
    }
}

pub struct ImagePreviewModal {
    url: String,
}

struct ImagePreviewModalComp {
    is_open: bool,
}

enum Msg {
    Toggle,
    Open,
    Close,
}

brass::enable_props!(wrapped ImagePreviewModal => ImagePreviewModalComp);

impl brass::PropComponent for ImagePreviewModalComp {
    type Properties = ImagePreviewModal;
    type Msg = Msg;

    fn init(_props: &Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self { is_open: false }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        _props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Toggle => {
                self.is_open = !self.is_open;
            }
            Msg::Open => {
                self.is_open = true;
            }
            Msg::Close => {
                self.is_open = false;
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        mut ctx: &mut brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> VNode {
        let img = vdom::img(props.url.clone())
            .style_raw("max-height: 200px; cursor: pointer;")
            .on_click(ctx.on_simple(|| Msg::Open));
        let img_div = vdom::div_with(img);

        let modal = if self.is_open {
            let full_img = vdom::img(props.url.clone())
                .style_raw("max-height: 100%; max-widht: 100%; object-fit: contain;");
            let content = vdom::div()
                .style_raw("display: flex; width: 100%; height: 100%;")
                .and(full_img);

            brass_bulma::modal(content, ctx.callback_map(|_| Msg::Close)).build()
        } else {
            VNode::Empty
        };

        vdom::div().and((img_div, modal)).build()
    }
}
