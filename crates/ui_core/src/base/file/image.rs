use brass::{
    dom::{builder::div, Attr, ClickEvent, Tag, TagBuilder, View},
    signal::signal::{Mutable, SignalExt},
};
use factordb::{prelude::DataMap, schema::AttrMapExt};
use semantic_core::base::AttrDownloadUrl;

use crate::{components::util::modal::modal, EntityRenderOpts};

pub fn image_content(item: &DataMap, opts: &EntityRenderOpts) -> TagBuilder {
    let url = semantic_core::base::File::blob_uri_from_map(&item)
        .or_else(|| item.get_attr::<AttrDownloadUrl>().map(|x| x.to_string()));

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

    div().and(img).signal(modal)
}
