use brass::{vdom, VNode};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_ui_core::EntityRenderOpts;
use semantics_core::base::{AttrBlobUri, AttrDownloadUrl};

use crate::components::base::plugin::build_blob_url;

pub fn image_content(item: &Item, opts: &EntityRenderOpts) -> VNode {
    let url = if let Some(blob_uri) = item.data.get_attr::<AttrBlobUri>() {
        build_blob_url(&blob_uri)
    } else if let Some(url) = item.data.get_attr::<AttrDownloadUrl>() {
        url.to_string()
    } else {
        todo!()
    };

    if opts.preview {
        // TODO: better size selection here!
        let img = vdom::img(url).style_raw("max-height: 200px;");
        img.build()
    } else {
        vdom::img(url).build()
    }
}
