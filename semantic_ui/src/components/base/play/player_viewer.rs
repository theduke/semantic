use std::rc::Rc;

use brass::{
    vdom::{self, s},
    Callback, EffectGuard, Shared,
};

use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_ui_core::{
    registry::{MediaRenderEvent, MediaRenderOpts, RegisteredMediaRenderer},
    ContextExt, DynEntityRenderer, EntityRenderOpts, SharedRegistry,
};

use crate::components::entity::generic_entity_view;

pub struct PlayerViewer {
    pub items: Shared<Vec<Item>>,
    pub playing: bool,
    // Settings.
    pub cycle: bool,
    pub muted: bool,
    pub shuffle: bool,
    pub autoplay_interval: Option<std::time::Duration>,
    pub active_index: Option<usize>,
    pub on_index_changed: Option<Callback<usize>>,
}

enum Msg {
    AutoplayTimeout,
    ItemEvent {
        index: usize,
        event: MediaRenderEvent,
    },
}

type Index = usize;

struct State {
    index: Index,
    registry: SharedRegistry,
    is_playing: bool,

    timeout_guard: Option<EffectGuard>,

    callback: Callback<Msg>,

    active_item_renderer: Option<ItemRenderer>,
}

brass::enable_props!(wrapped PlayerViewer => State);

enum ItemRenderer {
    Generic(DynEntityRenderer),
    Media(RegisteredMediaRenderer),
}

impl ItemRenderer {
    /// Returns `true` if the item_renderer is [`Media`].
    fn supports_playback(&self) -> bool {
        match self {
            Self::Media(RegisteredMediaRenderer {
                supports_playback, ..
            }) => *supports_playback,
            Self::Generic(_) => false,
        }
    }
}

impl State {
    fn next(&mut self, props: &PlayerViewer, ctx: &mut brass::Context<Msg>) {
        let index = self.index + 1;
        if index + 1 > props.items.len() {
            if props.cycle {
                self.goto(0, props, ctx)
            } else {
                self.is_playing = false;
            }
        } else {
            self.goto(index, props, ctx)
        }
    }

    fn prev(&mut self, props: &PlayerViewer, ctx: &mut brass::Context<Msg>) {
        if self.index == 0 {
            if props.cycle {
                self.goto(props.items.len() - 1, props, ctx);
            }
        } else {
            self.goto(self.index - 1, props, ctx);
        }
    }

    fn start(
        &mut self,
        new_index: Option<Index>,
        props: &PlayerViewer,
        ctx: &mut brass::Context<Msg>,
    ) {
        self.is_playing = true;
        if let Some(new) = new_index {
            self.goto(new, props, ctx);
        } else {
            let active_supports_playback = self
                .active_item_renderer
                .as_ref()
                .map(|x| x.supports_playback())
                .unwrap_or_default();
            if !active_supports_playback {
                self.next(props, ctx);
            }
        }
    }

    fn stop(&mut self) {
        self.is_playing = false;
        // Remove active timeout guard to prevent continuing.
        self.timeout_guard.take();
    }

    fn goto(&mut self, index: Index, props: &PlayerViewer, ctx: &mut brass::Context<Msg>) {
        self.index = index;
        tracing::trace!(?index, "go to index");

        let item = if let Some(item) = props.items.get(index) {
            item
        } else {
            tracing::error!("Internal media player error: tried to show index that does not exist");
            return;
        };

        let ty = if let Some(ty) = item.data.get_type_name() {
            ty
        } else {
            // Item has no type, so skip it.
            self.next(props, ctx);
            return;
        };

        let mut supports_playback = false;

        let render: ItemRenderer = if let Some(media_render) = self.registry.get_media_renderer(ty)
        {
            supports_playback = media_render.supports_playback;

            ItemRenderer::Media(media_render.clone())
        } else if let Some(regular_renderer) = self.registry.entity_content_renderer(ty) {
            ItemRenderer::Generic(regular_renderer.clone())
        } else {
            let registry = self.registry.clone();
            ItemRenderer::Generic(Rc::new(move |item, _opts| {
                generic_entity_view(
                    &item,
                    &registry,
                    &EntityRenderOpts {
                        editable: false,
                        preview: false,
                    },
                )
                .build()
            }))
        };

        self.timeout_guard.take();
        if !supports_playback && self.is_playing {
            if let Some(duration) = &props.autoplay_interval {
                self.timeout_guard = Some(ctx.timeout(Msg::AutoplayTimeout, *duration));
            }
        }

        self.active_item_renderer = Some(render);

        if let Some(cb) = props.on_index_changed.as_ref() {
            cb.send(index);
        }
    }
}

impl brass::PropComponent for State {
    type Properties = PlayerViewer;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let mut s = Self {
            is_playing: false,
            registry: ctx.registry().clone(),
            index: props.active_index.unwrap_or_default(),
            timeout_guard: None,
            callback: ctx.callback(),
            active_item_renderer: None,
        };

        s.goto(s.index, props, ctx);

        s
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::AutoplayTimeout => {
                self.next(props, ctx);
            }
            Msg::ItemEvent { index: _, event } => match event {
                MediaRenderEvent::Finished(_) => {
                    self.next(props, ctx);
                }
                MediaRenderEvent::Paused => {
                    self.is_playing = false;
                }
                MediaRenderEvent::Resumed => {
                    self.is_playing = true;
                }
            },
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        _ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        // let btn_play = brass_bulma::button()
        //     .and(brass_bulma::icon_fa("fas fa-play"))
        //     .attr(Attr::Title, s("Play"))
        //     .on_click(ctx.on_simple(|| Msg::TogglePlay));
        // let btn_next = brass_bulma::button()
        //     .and(brass_bulma::icon_fa("fas fa-next"))
        //     .attr(Attr::Title, s("Next"))
        //     .on_click(ctx.on_simple(|| Msg::Next));
        // let btn_prev = brass_bulma::button()
        //     .and(brass_bulma::icon_fa("fas fa-prev"))
        //     .attr(Attr::Title, s("Back"))
        //     .on_click(ctx.on_simple(|| Msg::Prev));
        // let bar = brass_bulma::box_();

        let active = match (&self.active_item_renderer, props.items.get(self.index)) {
            (Some(ItemRenderer::Generic(f)), Some(item)) => f(
                item,
                &EntityRenderOpts {
                    editable: false,
                    preview: false,
                },
            ),
            (Some(ItemRenderer::Media(renderer)), Some(item)) => {
                let index = self.index;
                (renderer.render)(
                    item,
                    &MediaRenderOpts {
                        playing: self.is_playing,
                        muted: props.muted,
                        callback: self
                            .callback
                            .clone()
                            .map(move |event| Msg::ItemEvent { index, event }),
                    },
                )
            }
            _ => brass_bulma::notification_warning(s("Nothing found")).build(),
        };

        vdom::div()
            .style_raw(s("height: 100%; width: 100%; overflow: hidden; display: flex; justify-content: center; align-items: center; flex-grow: 1;"))
            .and(active)
            .build()
    }

    fn on_property_change(
        &mut self,
        old_props: &Self::Properties,
        new_props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if new_props.autoplay_interval.is_none() {
            self.timeout_guard.take();
        }

        if new_props.items != old_props.items {
            self.goto(0, new_props, ctx);
        }

        if old_props.playing != new_props.playing {
            if new_props.playing {
                let new_index = if old_props.active_index != new_props.active_index {
                    new_props.active_index.clone()
                } else {
                    None
                };
                self.start(new_index, new_props, ctx);
            } else {
                self.stop();
            }
        } else {
            if new_props.active_index != old_props.active_index
                && new_props.active_index.clone().unwrap() != self.index
            {
                self.goto(new_props.active_index.clone().unwrap(), new_props, ctx);
            }
        }

        true
    }
}
