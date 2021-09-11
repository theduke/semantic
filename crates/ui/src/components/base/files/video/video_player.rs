use brass::{
    dom::{Attr, Event},
    vdom,
};
use semantic_ui_core::registry::{MediaRenderEvent, MediaRenderOpts};
use wasm_bindgen::JsCast;

use super::VideoInfo;

pub struct VideoPlayer {
    pub info: VideoInfo,
    pub options: MediaRenderOpts,
}

enum Msg {}

struct State {
    ref_: brass::vdom::Ref,
}

brass::enable_props!(wrapped VideoPlayer => State);

impl brass::PropComponent for State {
    type Properties = VideoPlayer;
    type Msg = Msg;

    fn init(_props: &Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            ref_: brass::vdom::Ref::new(),
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        _props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {}
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: &mut brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        let source = vdom::tag(brass::dom::Tag::Source).attr(Attr::Src, &props.info.url);

        let callback_ended = props
            .options
            .callback
            .clone()
            .map(|_| MediaRenderEvent::Finished(Ok(())));

        let callback_error = props
            .options
            .callback
            .clone()
            .map(|_| MediaRenderEvent::Finished(Err(anyhow::anyhow!("Could not load video"))));

        let video = vdom::tag(brass::dom::Tag::Video)
            .attr_toggle(Attr::Controls)
            .attr_toggle_if(props.options.muted, Attr::Muted)
            .attr_toggle_if(props.options.playing, Attr::AutoPlay)
            // FIXME: refactor to on_callback
            .on_event_opt(ctx, Event::Ended, move |_| {
                callback_ended.send(());
                None
            })
            .on_event_opt(ctx, Event::Error, move |_| {
                callback_error.send(());
                None
            })
            .and(source)
            .build_ref(&self.ref_);

        video
    }

    fn on_render(&mut self, props: &Self::Properties, first_render: bool) {
        if !first_render {
            self.ref_
                .get()
                .and_then(|elem| elem.dyn_into::<web_sys::HtmlVideoElement>().ok())
                .map(|elem| {
                    let res = if props.options.playing {
                        elem.play().map(|_| ())
                    } else {
                        elem.pause().map(|_| ())
                    };

                    if let Err(_err) = res {
                        tracing::error!("Could not control video element");
                    }
                });
        }
    }
}
