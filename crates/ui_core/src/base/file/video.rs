use wasm_bindgen::JsCast;

use brass::dom::{Attr, Event, Tag, TagBuilder};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_core::base::{AttrBlobUri, AttrDownloadUrl, AttrPreviewImageUrl};

use crate::{
    base::plugin::build_blob_url,
    components::util::notification_warning,
    registry::{DynMediaHandle, MediaHandle, MediaRenderEvent, MediaRenderOpts},
    EntityRenderOpts,
};

pub struct VideoInfo {
    pub url: String,
    pub mime_type: Option<String>,
    pub supports_browser: bool,
    pub preview_image_url: Option<url::Url>,
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

        let preview_image_url = item.data.get_attr::<AttrPreviewImageUrl>();

        Some(VideoInfo {
            url,
            mime_type,
            supports_browser,
            preview_image_url,
        })
    }

    pub fn from_item_for_browser(item: &Item) -> Option<Self> {
        Self::from_item(item).filter(|v| v.supports_browser)
    }
}

/// Get the video URL from an item.
pub fn video_content(item: &Item, opts: &EntityRenderOpts) -> TagBuilder {
    let info = if let Some(info) = VideoInfo::from_item(item) {
        info
    } else {
        return notification_warning().and("Video can't be played.");
    };

    if info.supports_browser {
        let source = Tag::Source.new().attr(Attr::Src, info.url);
        let video = Tag::Video.new().attr_toggle(Attr::Controls).and(source);

        if opts.preview {
            video.style_raw("max-width: 200px; max-height: 200px; object-fit: contain;")
        } else {
            video
        }
    } else {
        notification_warning().and("Video can't be played.")
    }
}

pub fn video_media(item: &Item, opts: &MediaRenderOpts) -> (TagBuilder, Option<DynMediaHandle>) {
    if let Some(info) = VideoInfo::from_item_for_browser(item) {
        let (tag, handle) = video_player(info, opts);
        (tag, Some(handle))
    } else {
        (notification_warning().and("Video can't be played"), None)
    }
}

struct VideoPlayerHandle {
    elem: web_sys::HtmlVideoElement,
}

impl MediaHandle for VideoPlayerHandle {
    fn play(&self) {
        if let Err(err) = self.elem.play() {
            tracing::error!(?err, "Could not start video playback");
        }
    }

    fn pause(&self) {
        tracing::trace!("calling pause!");
        if let Err(err) = self.elem.pause() {
            tracing::error!(?err, "Could not pause video playback");
        }
    }

    fn set_muted(&self, muted: bool) {
        self.elem.set_muted(muted);
    }
}

pub fn video_player(info: VideoInfo, options: &MediaRenderOpts) -> (TagBuilder, DynMediaHandle) {
    // TODO: muted event handling
    let source = Tag::Source.new().attr(Attr::Src, &info.url);

    let callback = options.callback.clone();
    let on_finished = move |_: web_sys::Event| callback(MediaRenderEvent::Finished(Ok(())));

    let callback = options.callback.clone();
    let on_error = move |_: web_sys::Event| {
        callback(MediaRenderEvent::Finished(Err(anyhow::anyhow!(
            "Could not load video"
        ))))
    };

    let video = Tag::Video
        .new()
        .style_raw("max-width: 100%; max-height: 100%; object-fit: contain;")
        .attr_toggle(Attr::Controls)
        .attr_toggle_if(options.muted, Attr::Muted)
        .attr_toggle_if(options.playing, Attr::AutoPlay)
        .on_event(Event::Ended, on_finished)
        .on_event(Event::Error, on_error)
        .and(source);

    let handle = VideoPlayerHandle {
        elem: video.elem().clone().dyn_into().unwrap(),
    };

    (video, std::rc::Rc::new(handle))
}
