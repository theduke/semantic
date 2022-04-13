use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Render, TagBuilder, View},
    signal::signal::{Mutable, SignalExt},
};
use semantic_core::base::Video;

use crate::{
    base::{build_entity_blob_path, plugin::build_entity_blob_path_video},
    components::{
        loader::Loader,
        util::{
            box_, buttons, notification_error, notification_success, subtitle_4, ButtonBuilder,
        },
    },
    context,
};

use super::video::video_tag;

pub struct FileOptimiseCompare {
    pub file: Video,
    pub on_cancel: Option<Box<dyn Fn()>>,
}

impl Render for FileOptimiseCompare {
    fn render(self) -> View {
        State::build(self)
    }
}

enum Discarded {
    Original,
    Conversion,
}

struct State {
    file: Video,
    on_cancel: Option<Box<dyn Fn()>>,
    loader: Loader<Discarded>,
}

#[derive(Debug)]
enum Msg {
    DiscardOriginal,
    DiscardConversion,
    Cancel,
}

impl MsgComponent for State {
    type Properties = FileOptimiseCompare;
    type Msg = Msg;

    fn init(props: Self::Properties, _ctx: brass::component::Context<Self>) -> Self {
        Self {
            file: props.file,
            on_cancel: props.on_cancel,
            loader: Loader::new_idle(),
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: brass::component::Context<Self>) {
        match msg {
            Msg::DiscardOriginal => {
                if self.loader.is_loading() {
                    return;
                }
                let id = self.file.file.id;
                self.loader.spawn(async move {
                    context::api()
                        .file_discard_un_optimized(id)
                        .await
                        .map(|_| Discarded::Original)
                });
            }
            Msg::DiscardConversion => {
                if self.loader.is_loading() {
                    return;
                }
                let id = self.file.file.id;
                self.loader.spawn(async move {
                    context::api()
                        .file_discard_optimized(id)
                        .await
                        .map(|_| Discarded::Conversion)
                });
            }
            Msg::Cancel => {
                if let Some(c) = &self.on_cancel {
                    c();
                }
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> brass::dom::TagBuilder {
        let video = &self.file;

        let video_old = if let Some(_url) = &video.file.blob_uri {
            video_tag(&build_entity_blob_path(video.file.id))
        } else {
            notification_error().and("Could not find old video: Video blob_uri not set!")
        };
        let old = div().and(subtitle_4().and("Old Video")).and(video_old);

        let video_new = if let Some(_url) = &video.file.blob_uri_web {
            video_tag(&build_entity_blob_path_video(video.file.id))
        } else {
            notification_error().and("Could not find new video: Video blob_uri_web not set!")
        };
        let new = div().and(subtitle_4().and("New Video")).and(video_new);

        let buttons = buttons()
            .and(
                ButtonBuilder::new()
                    .size(crate::components::util::BtnSize::Large)
                    .color(crate::components::util::Color::Warning)
                    .signal_disabled(self.loader.signal_loading())
                    .label("Discard original file")
                    .on(ctx.callback_msg(|| Msg::DiscardOriginal))
                    .build(),
            )
            .and(
                ButtonBuilder::new()
                    .size(crate::components::util::BtnSize::Large)
                    .signal_disabled(self.loader.signal_loading())
                    .label("Discard converted file")
                    .on(ctx.callback_msg(|| Msg::DiscardConversion))
                    .build(),
            )
            .and(
                ButtonBuilder::new()
                    .size(crate::components::util::BtnSize::Large)
                    .signal_disabled(self.loader.signal_loading())
                    .label("Cancel")
                    .on(ctx.callback_msg(|| Msg::Cancel))
                    .build(),
            );

        let status = self.loader.signal_render(|flag| {
            let msg = match flag {
                Discarded::Original => "Original file version was deleted",
                Discarded::Conversion => "Optimised file version was deleted",
            };
            notification_success().and(msg).into_view()
        });

        box_().and(old).and(new).and(buttons).signal(status)
    }
}

pub fn file_optimise_compare_toggle(video: Video) -> TagBuilder {
    let is_active = Mutable::new(false);

    let is_active2 = is_active.clone();
    div().signal(is_active.signal_cloned().map(move |flag| {
        if flag {
            FileOptimiseCompare {
                file: video.clone(),
                on_cancel: {
                    let flag = is_active2.clone();
                    Some(Box::new(move || {
                        flag.set(false);
                    }))
                },
            }
            .render()
        } else {
            let flag = is_active.clone();
            ButtonBuilder::new()
                .label("Compare Optimised")
                .on(move || {
                    flag.set(true);
                })
                .build()
                .into_view()
        }
    }))
}
