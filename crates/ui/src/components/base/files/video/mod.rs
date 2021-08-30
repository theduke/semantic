mod video_player;
pub use video_player::VideoPlayer;

use brass::{
    dom::Attr,
    vdom::{self, s, Render},
    VNode,
};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_core::base::{AttrBlobUri, AttrDownloadUrl};
use semantic_ui_core::{registry::MediaRenderOpts, EntityRenderOpts};

use crate::components::base::plugin::build_blob_url;

pub struct VideoInfo {
    pub url: String,
    pub mime_type: Option<String>,
    pub supports_browser: bool,
}

impl VideoInfo {
    pub fn from_item(item: &Item) -> Option<Self> {
        let url = item
            .data
            .get_attr::<AttrBlobUri>()
            .map(|url| build_blob_url(&url))
            .or_else(|| {
                item.data
                    .get_attr::<AttrDownloadUrl>()
                    .map(|x| x.to_string())
            })?;

        let mime_type = item.data.get_attr::<semantic_core::base::AttrMimeType>();

        let supports_browser = url.ends_with(".mp4")
            || url.ends_with(".webm")
            || mime_type
                .as_ref()
                .map(|ty| ty == "video/mp4" || ty == "video/webm")
                .unwrap_or_default();

        Some(VideoInfo {
            url,
            mime_type,
            supports_browser,
        })
    }

    pub fn from_item_for_browser(item: &Item) -> Option<Self> {
        Self::from_item(item).filter(|v| v.supports_browser)
    }
}

/// Get the video URL from an item.
pub fn video_content(item: &Item, opts: &EntityRenderOpts) -> VNode {
    let info = if let Some(info) = VideoInfo::from_item(item) {
        info
    } else {
        return brass_bulma::notification(brass_bulma::Color::Default, s("Video can't be played."))
            .build();
    };

    if info.supports_browser {
        let source = vdom::tag(brass::dom::Tag::Source).attr(Attr::Src, info.url);
        let video = vdom::tag(brass::dom::Tag::Video)
            .attr_toggle(Attr::Controls)
            .and(source);

        let video = if opts.preview {
            video.style_raw("max-height: 200px; object-fit: contain;")
        } else {
            video
        };

        video.build()
    } else {
        brass_bulma::notification(brass_bulma::Color::Default, s("Video can't be played.")).build()
    }
}

pub fn video_media(item: &Item, opts: &MediaRenderOpts) -> VNode {
    if let Some(info) = VideoInfo::from_item_for_browser(item) {
        VideoPlayer {
            info,
            options: opts.clone(),
        }
        .render()
    } else {
        brass_bulma::notification(brass_bulma::Color::Default, "Video can't be played").build()
    }
}
