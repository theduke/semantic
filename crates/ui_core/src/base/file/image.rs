use brass::{
    dom::{builder::div, Attr, ClickEvent, Tag, TagBuilder, View},
    signal::signal::{Mutable, SignalExt},
};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_core::base::{AttrBlobUri, AttrDownloadUrl};

use crate::{components::util::modal::modal, EntityRenderOpts};

use super::super::plugin::build_blob_url;

pub fn image_content(item: &Item, opts: &EntityRenderOpts) -> TagBuilder {
    let url = if let Some(blob_uri) = item.data.get_attr::<AttrBlobUri>() {
        Some(build_blob_url(&blob_uri))
    } else if let Some(url) = item.data.get_attr::<AttrDownloadUrl>() {
        Some(url.to_string())
    } else {
        None
    };

    if let Some(url) = url {
        if opts.preview {
            image_with_preview_modal(url)
        } else {
            Tag::Img
                .new()
                .attr(Attr::Src, url)
                .style_raw("max-height: 100%; max-width: 100%; object-fit: contain;")
        }
    } else {
        Tag::Span.new().and("Image without url")
    }
}

fn image_with_preview_modal(url: String) -> TagBuilder {
    let is_visible = Mutable::new(false);

    let is_visible2 = is_visible.clone();
    let img = Tag::Img
        .new()
        .attr(Attr::Src, &url)
        .style_raw("max-height: 200px; cursor: pointer;")
        .on(move |_: ClickEvent| {
            is_visible2.set(true);
        });

    let modal = is_visible.signal_cloned().map(move |flag| -> View {
        if flag {
            let full_img = Tag::Img
                .new()
                .attr(Attr::Src, &url)
                .style_raw("max-height: 100%; max-widht: 100%; object-fit: contain;");
            let inner = div()
                .style_raw("display: flex; width: 100%; height: 100%;")
                .and(full_img);

            let is_visible2 = is_visible.clone();
            let content = modal(
                inner,
                move || {
                    is_visible2.set(false);
                },
                true,
            );
            content.into()
        } else {
            View::Empty
        }
    });

    div().and(img).child_signal(modal)
}
