use wasm_bindgen::JsCast;

use brass::dom::{builder::div, Attr, Ev, Tag, TagBuilder, View};
use factdb::{AttrMapExt, ClassContainer, DataMap, Id};
use semantic_core::base::{
    AttrBlobUri, AttrBlobUriWeb, AttrPreviewImageBlobUri, AttrPreviewImageUrl, Video,
};

use crate::{
    base::{
        file::video_preview_builder::video_preview_picker_toggle,
        plugin::build_entity_blob_preview_image_uri,
    },
    components::util::notification_warning,
    registry::{DynMediaHandle, MediaHandle, MediaRenderEvent, MediaRenderOpts},
    EntityRenderOpts,
};

pub struct VideoInfo {
    pub id: Id,
    pub url: String,
    pub mime_type: Option<String>,
    pub preview_image_url: Option<url::Url>,
}

impl VideoInfo {
    pub fn from_map(item: &DataMap) -> Option<Self> {
        let id = item.get_id()?;
        let url = semantic_core::base::Video::video_uri_from_map(&item)?;

        // let url = item
        //     .data
        //     .get_attr::<AttrBlobUri>()
        //     .map(|uri| format!("/blob/video/{}", uri))
        //     .or_else(|| {
        //         item.data
        //             .get_attr::<AttrDownloadUrl>()
        //             .map(|x| x.to_string())
        //     })?;

        let mime_type = item.get_attr::<semantic_core::base::AttrMimeType>();

        let preview_image_url = item.get_attr::<AttrPreviewImageUrl>();

        Some(VideoInfo {
            id,
            url,
            mime_type,
            preview_image_url,
        })
    }
}

/// Get the video URL from an item.
pub fn video_content(item: &DataMap, opts: &EntityRenderOpts) -> TagBuilder {
    tracing::info!(?opts, "video render opts");
    let web_uri = item.get_attr::<AttrBlobUriWeb>();
    let blob_uri = item.get_attr::<AttrBlobUri>();
    let id = item.get_id();

    let video = Video::try_from_map(item.clone()).ok();

    // FIXME: use custom EntityBox and add optimise / preview picker as action buttons

    let optimiser = match id {
        Some(id) if !web_uri.is_some() && opts.editable => {
            let opt = super::optimiser::VideoOptimiser { video_id: id };
            Some(div().class("mb-4").and(opt))
        }
        Some(_id) if web_uri.is_some() && web_uri != blob_uri && opts.editable => {
            if let Some(video) = &video {
                Some(div().class("mb-4").and(
                    super::optimise_compare::file_optimise_compare_toggle(video.clone()),
                ))
            } else {
                None
            }
        }
        _ => None,
    };

    let preview_picker = if opts.editable && item.get_attr::<AttrPreviewImageBlobUri>().is_none() {
        if let Some(video) = &video {
            div()
                .class("mb-4")
                .and(video_preview_picker_toggle(video.clone()))
                .into_view()
        } else {
            View::Empty
        }
    } else {
        View::Empty
    };

    let info = if let Some(info) = VideoInfo::from_map(item) {
        info
    } else {
        return notification_warning().and("Video can't be played.");
    };

    let poster = item
        .get_attr::<AttrPreviewImageBlobUri>()
        .map(|_path| build_entity_blob_preview_image_uri(info.id))
        .or_else(|| info.preview_image_url.map(|x| x.to_string()));

    let mut video = video_tag(&info.url);

    if let Some(poster) = poster {
        video = video.attr(Attr::Poster, poster);
    }

    let main = if opts.preview {
        video
            .style_raw("max-width: 200px; max-height: 200px; object-fit: contain;")
            // Disable preload in previews since some browsers trigger
            // huge downloads (Firefox).
            .attr(Attr::Preload, "none")
    } else {
        video
    };

    div().and(optimiser).and(preview_picker).and(main)
}

pub fn video_tag(url: &str) -> TagBuilder {
    let source = Tag::Source.new().attr(Attr::Src, url);
    let video = Tag::Video.new().attr_toggle(Attr::Controls).and(source);
    video
}

pub fn video_media(item: &DataMap, opts: &MediaRenderOpts) -> (TagBuilder, Option<DynMediaHandle>) {
    if let Some(info) = VideoInfo::from_map(item) {
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

    fn on_remove(&self) {
        // Reset the video source to stop the request.
        self.elem.pause().ok();
        self.elem.set_src("");
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
        .on_event(Ev::Ended, on_finished)
        .on_event(Ev::Error, on_error)
        .and(source);

    let handle = VideoPlayerHandle {
        elem: video.elem().clone().dyn_into().unwrap(),
    };

    (video, std::rc::Rc::new(handle))
}
