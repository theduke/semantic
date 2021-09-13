use rand::seq::SliceRandom;
use std::rc::Rc;
use wasm_bindgen::JsCast;

use brass::{
    dom::Attr,
    vdom::{self, s, Render},
    Callback, PropComponent, Shared,
};
use brass_bulma::{Help, SelectOption};
use factordb::{
    query::{expr::Expr, select::Item},
    schema::{builtin::AttrType, EntityDescriptor},
    AnyError,
};
use semantic_ui_core::{loader::LoadState, ContextExt};

use crate::components::entity::entity_filter::EntityFilter;

pub struct StandalonePlayer {
    pub expr: Option<Expr>,
    pub keyboard_controls: bool,
}

enum Msg {
    FilterChanged(EntityFilter),
    Loaded(Result<Vec<Item>, AnyError>),
    Next,
    Prev,
    TogglePlay,
    ToggleShuffle,
    ToggleLoop,
    ToggleMuted,
    ToggleSettings,
    ToggleFullscreen,
    IntervalChanged(Option<std::time::Duration>),
    KeyPress(String),

    OnIndexChanged(usize),
}

struct State {
    loader: LoadState<Shared<Vec<Item>>>,
    expr: Expr,

    playing: bool,
    muted: bool,
    cycle: bool,
    shuffle: bool,
    index: usize,
    autoplay_interval: Option<std::time::Duration>,
    fullscreen: bool,
    settings_active: bool,

    keydown_callback: Callback<web_sys::KeyboardEvent>,
    keydown_subscription: Option<brass::util::EventSubscription>,

    /// Dom reference to the player div.
    /// Used for fullscreen support.
    player_ref: brass::vdom::Ref,
}

brass::enable_props!(wrapped StandalonePlayer => State);

impl State {
    fn default_expr() -> Expr {
        Expr::in_(
            Expr::attr::<AttrType>(),
            vec![
                semantic_core::base::Image::QUALIFIED_NAME,
                semantic_core::base::Video::QUALIFIED_NAME,
            ],
        )
    }

    fn load(&mut self, expr: Expr, ctx: &mut brass::Context<Msg>) {
        let api = ctx.api().clone();
        let guard = ctx.run_map(
            async move {
                let select = factordb::query::select::Select::new()
                    .with_filter(expr)
                    .with_limit(1000);
                let page = api.select(select).await?;
                Ok(page.items)
            },
            Msg::Loaded,
        );
        self.loader.set_loading_guarded(guard);
    }
}

impl PropComponent for State {
    type Properties = StandalonePlayer;
    type Msg = Msg;

    fn init(props: &Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let keydown_callback =
            ctx.callback_map(|ev: web_sys::KeyboardEvent| Msg::KeyPress(ev.key()));

        let mut s = Self {
            loader: LoadState::Idle,
            expr: props.expr.clone().unwrap_or_else(|| Self::default_expr()),
            index: 0,
            playing: false,
            muted: false,
            cycle: true,
            shuffle: false,
            autoplay_interval: Some(std::time::Duration::from_secs(5)),
            fullscreen: false,
            settings_active: false,
            keydown_callback,
            keydown_subscription: None,
            player_ref: brass::vdom::Ref::new(),
        };

        s.load(s.expr.clone(), ctx);

        s
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Loaded(res) => {
                let res = res.map(|mut items| {
                    if self.shuffle {
                        items.shuffle(&mut rand::thread_rng());
                    }
                    Shared::new(items)
                });
                self.loader.set_result(res);
            }
            Msg::FilterChanged(filter) => {
                let expr = filter.build_expr();

                self.expr = if let Some(base) = &props.expr {
                    base.clone().and_with(expr)
                } else {
                    expr
                };
                self.load(self.expr.clone(), ctx);
            }
            Msg::Next => {
                if let Some(items) = self.loader.as_success() {
                    let new_index = if self.index + 1 > items.len() {
                        if self.cycle {
                            Some(0)
                        } else {
                            None
                        }
                    } else {
                        Some(self.index + 1)
                    };

                    if let Some(idx) = new_index {
                        self.index = idx;
                    }
                }
            }
            Msg::Prev => {
                if let Some(items) = self.loader.as_success() {
                    let new_index = if self.index == 0 {
                        if self.cycle {
                            items.len().checked_sub(1)
                        } else {
                            None
                        }
                    } else {
                        Some(self.index - 1)
                    };

                    if let Some(idx) = new_index {
                        self.index = idx;
                    }
                }
            }
            Msg::TogglePlay => {
                self.playing = !self.playing;
            }
            Msg::ToggleShuffle => {
                self.shuffle = !self.shuffle;

                if self.shuffle {
                    // Randomize current items.
                    if let Some(items) = self.loader.as_success_mut() {
                        let mut new_items = items.as_ref().clone();
                        new_items.shuffle(&mut rand::thread_rng());
                        *items = Shared::new(new_items);
                    }
                }
            }
            Msg::ToggleLoop => {
                self.cycle = !self.cycle;
            }
            Msg::ToggleSettings => {
                self.settings_active = !self.settings_active;
            }
            Msg::IntervalChanged(value) => {
                self.autoplay_interval = value;
            }
            Msg::OnIndexChanged(index) => {
                self.index = index;
                ctx.skip_render();
            }
            Msg::ToggleMuted => {
                self.muted = !self.muted;
            }
            Msg::ToggleFullscreen => {
                if !self.fullscreen {
                    if let Some(elem) = self.player_ref.get() {
                        if let Err(_error) = elem.request_fullscreen() {
                            tracing::error!("Could not launch fullscreen mode");
                        } else {
                            self.fullscreen = true;
                        }
                    }
                } else {
                    self.fullscreen = false;
                    brass::util::document().exit_fullscreen();
                }
            }

            Msg::KeyPress(key) => match key.as_str() {
                "ArrowLeft" => {
                    self.update(Msg::Prev, props, ctx);
                }
                "ArrowRight" => {
                    self.update(Msg::Next, props, ctx);
                }
                " " => {
                    self.update(Msg::TogglePlay, props, ctx);
                }
                "m" => {
                    self.update(Msg::ToggleMuted, props, ctx);
                }
                _other => {
                    ctx.skip_render();
                }
            },
        }
    }

    fn render(
        &self,
        _props: &Self::Properties,
        ctx: &mut brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        let btn_play = brass_bulma::button()
            .attr(
                Attr::Title,
                if self.playing { s("Pause") } else { s("Play") },
            )
            .and(brass_bulma::icon_fa(if self.playing {
                s("fas fa-pause")
            } else {
                s("fas fa-play")
            }))
            .on_click(ctx, || Msg::TogglePlay);
        let btn_prev = brass_bulma::button()
            .and(brass_bulma::icon_fa("fas fa-chevron-circle-left"))
            .attr(Attr::Title, "Back")
            .on_click(ctx, || Msg::Prev);
        let btn_next = brass_bulma::button()
            .and(brass_bulma::icon_fa("fas fa-chevron-circle-right"))
            .attr(Attr::Title, "Forward")
            .on_click(ctx, || Msg::Next);

        let controls = vdom::div()
            .class(s("mr-4"))
            .and((btn_play, btn_prev, btn_next));

        let btn_shuffle = brass_bulma::button()
            .and(brass_bulma::icon_fa(s("fas fa-random")))
            .and_class_if(self.shuffle, "is-info")
            .attr(Attr::Title, s("Shuffle"))
            .on_click(ctx, || Msg::ToggleShuffle);
        let btn_cycle = brass_bulma::button()
            .and(brass_bulma::icon_fa(s("fas fa-undo")))
            .and_class_if(self.cycle, "is-info")
            .attr(Attr::Title, s("Loop"))
            .on_click(ctx, || Msg::ToggleLoop);
        let btn_mute = brass_bulma::button()
            .and(brass_bulma::icon_fa(s(if self.muted {
                "fas fa-volume-slash"
            } else {
                "fas fa-volume"
            })))
            .attr(Attr::Title, s(if self.muted { "Unmute" } else { "Mute" }))
            .on_click(ctx, || Msg::ToggleMuted);

        let btn_fullscreen = brass_bulma::button()
            .and(brass_bulma::icon_fa(s("fas fa-expand-arrows-alt")))
            .attr(Attr::Title, s("Fullscreen"))
            .on_click(ctx, || Msg::ToggleFullscreen);

        let btn_settings = brass_bulma::button()
            .and(brass_bulma::icon_fa(s("fas fa-cog")))
            .and_class_if(self.settings_active, "is-info")
            .attr(Attr::Title, s("Settings"))
            .on_click(ctx, || Msg::ToggleSettings);

        let bar_settings = vdom::div().class(s("mb-4")).and((
            btn_shuffle,
            btn_cycle,
            btn_mute,
            btn_fullscreen,
            btn_settings,
        ));

        let settings = if self.settings_active {
            let interval = brass_bulma::FieldHorizontal {
                label: s("Duration"),
                help: Some(Help {
                    message: s("Time that static media (like images) is displayed. Dynamic content like video or audio will play until finished.").render(),
                    color: brass_bulma::Color::Default,
                }),
                control: brass_bulma::Select {
                    value: self.autoplay_interval.clone(),
                    empty_option_label: Some(s("No Autoplay")),
                    options: Rc::new(vec![
                        SelectOption {
                            value: std::time::Duration::from_secs(5),
                            label: s("5 seconds"),
                        },
                        SelectOption {
                            value: std::time::Duration::from_secs(10),
                            label: s("10 seconds"),
                        },
                        SelectOption {
                            value: std::time::Duration::from_secs(30),
                            label: s("30 seconds"),
                        },
                        SelectOption {
                            value: std::time::Duration::from_secs(60),
                            label: s("1 minute"),
                        },
                    ]),
                    on_select: ctx.callback_map(Msg::IntervalChanged),
                },
            };

            let filter = crate::components::entity::entity_filter::EntityFilterForm {
                on_submit: ctx.callback_map(Msg::FilterChanged),
            };

            brass_bulma::box_()
                .and_class("mb-4")
                .and((interval, vdom::hr(), filter))
        } else {
            vdom::div()
        };

        let bar = vdom::div()
            .style_raw(s("display: flex; margin-bottom: 1rem;"))
            .and((controls, bar_settings));

        let player = self.loader.render(|items| {
            super::PlayerViewer {
                items: items.clone(),
                playing: self.playing,
                cycle: self.cycle,
                muted: self.muted,
                shuffle: self.shuffle,
                autoplay_interval: self.autoplay_interval.clone(),
                active_index: Some(self.index),
                on_index_changed: Some(ctx.callback_map(Msg::OnIndexChanged)),
            }
            .render()
        });
        let player_wrap = vdom::div()
            .style_raw(s("flex-grow: 1; height: 100%; widht: 100%;"))
            .and(player)
            .build_ref(&self.player_ref);

        vdom::div()
            .style_raw(s(
                "height: 100%; width: 100%; overflow: hidden; display: flex; flex-direction: column;",
            ))
            .and(bar)
            .and(settings)
            .and(player_wrap)
            .build()
    }

    fn on_render(&mut self, props: &Self::Properties, first_render: bool) {
        if first_render && props.keyboard_controls {
            let target = brass::util::document();
            self.keydown_subscription = Some(brass::util::EventSubscription::subscribe(
                target.dyn_into().unwrap(),
                brass::dom::Event::KeyDown,
                self.keydown_callback.clone(),
            ));
        }
    }
}
