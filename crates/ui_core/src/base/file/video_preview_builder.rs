use anyhow::anyhow;
use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Render, Tag, TagBuilder, View},
    signal::signal::Mutable,
};

use semantic_core::{api, base::Video};
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, HtmlVideoElement};

use crate::{
    base::plugin::build_entity_blob_path_video,
    components::{
        loader::Loader,
        util::{
            box_, buttons, notification_error, notification_success, subtitle_4, ButtonBuilder,
        },
    },
    context,
};

use super::video::video_tag;

pub struct VideoPreviewBuilder {
    pub video: Video,
    pub on_cancel: Option<Box<dyn Fn()>>,
}

impl Render for VideoPreviewBuilder {
    fn render(self) -> brass::dom::View {
        State::build(self)
    }
}

struct State {
    video: Video,
    pub on_cancel: Option<Box<dyn Fn()>>,
    tags: Option<(HtmlVideoElement, HtmlCanvasElement)>,
    captured_data_url: Mutable<Option<Result<String, String>>>,
    loader: Loader<()>,
}

enum Msg {
    Capture,
    Apply,
    Cancel,
}

impl MsgComponent for State {
    type Properties = VideoPreviewBuilder;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        Self {
            video: props.video,
            on_cancel: props.on_cancel,
            tags: None,
            captured_data_url: Mutable::new(None),
            loader: Loader::new_idle(),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: brass::component::Context<Self>) {
        match msg {
            Msg::Capture => {
                if let Some((video, canvas)) = &self.tags {
                    let res = capture(video, canvas).map_err(|err| err.to_string());
                    self.captured_data_url.set(Some(res))
                }
            }
            Msg::Apply => {
                if self.loader.is_loading() {
                    return;
                }
                if let Some(Ok(url)) = &*self.captured_data_url.lock_ref() {
                    let id = self.video.file.id;
                    let url = url.clone();

                    self.loader.spawn(async move {
                        context::api()
                            .file_create_preview_image_blob(api::FileCreatePreviewImageBlob {
                                file_id: id,
                                data: url,
                                mime_type: "image/webp".to_string(),
                            })
                            .await
                    })
                }
            }
            Msg::Cancel => {
                if let Some(c) = &self.on_cancel {
                    c();
                }
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> brass::dom::TagBuilder {
        let _blob = if let Some(u) = &self.video.file.blob_uri {
            u
        } else {
            return notification_error().and("Video does not have a blob");
        };

        let url = build_entity_blob_path_video(self.video.file.id);

        let video = video_tag(&url);
        let video_tag = video.elem().dyn_ref::<HtmlVideoElement>().unwrap().clone();

        let canvas = Tag::Canvas.new();
        let canvas_tag = canvas
            .elem()
            .dyn_ref::<HtmlCanvasElement>()
            .unwrap()
            .clone();
        self.tags = Some((video_tag, canvas_tag));

        let handle = ctx.handle();
        let loader = self.loader.clone();
        let can_cancel = self.on_cancel.is_some();
        let control = self.captured_data_url.signal_ref(move |opt| match opt {
            Some(Ok(_)) => {
                let cancel = if can_cancel {
                    ButtonBuilder::new()
                        .label("Cancel")
                        .on(handle.callback(|| Msg::Cancel))
                        .build()
                        .into_view()
                } else {
                    View::Empty
                };
                buttons()
                    .and(
                        ButtonBuilder::new()
                            .label("Set Preview")
                            .signal_loading(loader.signal_loading())
                            .on(handle.callback(|| Msg::Apply))
                            .build(),
                    )
                    .and(cancel)
                    .into_view()
            }
            Some(Err(err)) => notification_error()
                .and(format!("Could not capture image: {err}"))
                .into_view(),
            None => View::Empty,
        });

        box_()
            .class("mb-4")
            .and(subtitle_4().and("Select Preview Image"))
            .and(div().and(video))
            .and(
                buttons().and(
                    ButtonBuilder::new()
                        .label("Capture")
                        .on(ctx.callback_msg(|| Msg::Capture))
                        .build(),
                ),
            )
            .and(div().and(canvas))
            .signal(self.loader.signal_render(|_| {
                notification_success()
                    .and("Preview image saved.")
                    .into_view()
            }))
            .signal(control)
    }
}

fn capture(video: &HtmlVideoElement, canvas: &HtmlCanvasElement) -> Result<String, anyhow::Error> {
    let width = video.video_width();
    let height = video.video_height();
    canvas.set_width(width);
    canvas.set_height(height);

    let c = canvas
        .get_context("2d")
        .ok()
        .flatten()
        .and_then(|opt| opt.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
        .ok_or_else(|| anyhow!("Could not acquire canvas 2d context"))?;

    c.draw_image_with_html_video_element_and_dw_and_dh(
        &video,
        0.0,
        0.0,
        width as f64,
        height as f64,
    )
    .map_err(|_| anyhow!("Could not draw video to canvas"))?;

    let url = canvas
        .to_data_url_with_type("image/webp")
        .map_err(|_| anyhow!("Could not capture canvas to image"))?;

    let data = url
        .split_once(',')
        .ok_or_else(|| anyhow!("Could not split data url"))?
        .1;

    Ok(data.to_string())
}

pub fn video_preview_picker_toggle(video: Video) -> TagBuilder {
    let is_active = Mutable::new(false);

    let is_active2 = is_active.clone();
    div().signal(is_active.signal_ref(move |flag| {
        if *flag {
            VideoPreviewBuilder {
                video: video.clone(),
                on_cancel: {
                    let flag = is_active2.clone();
                    Some(Box::new(move || {
                        flag.set(false);
                    }))
                },
            }
            .render()
        } else {
            let flag = is_active2.clone();
            ButtonBuilder::new()
                .label("Pick Preview Image")
                .on(move || {
                    flag.set(true);
                })
                .build()
                .into_view()
        }
    }))
}
