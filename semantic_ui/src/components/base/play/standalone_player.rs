use rand::seq::SliceRandom;
use std::rc::Rc;

use brass::{
    dom::Attr,
    vdom::{self, s, Render},
    PropComponent, Shared,
};
use brass_bulma::{Help, SelectOption};
use factordb::{
    query::{expr::Expr, select::Item},
    schema::{builtin::AttrType, EntityDescriptor},
    AnyError,
};
use semantic_ui_core::loader::LoadState;

pub struct StandalonePlayer {
    pub expr: Option<Expr>,
}

enum Msg {
    FilterChanged(Expr),
    Loaded(Result<Vec<Item>, AnyError>),
    Next,
    Prev,
    TogglePlay,
    ToggleShuffle,
    ToggleLoop,
    ToggleMuted,
    ToggleSettings,
    IntervalChanged(Option<std::time::Duration>),

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
    settings_active: bool,
}

brass::enable_props!(wrapped StandalonePlayer => State);

impl State {
    fn default_expr() -> Expr {
        Expr::in_(
            Expr::attr::<AttrType>(),
            vec![
                semantics_core::base::Image::QUALIFIED_NAME,
                semantics_core::base::Video::QUALIFIED_NAME,
            ],
        )
    }

    fn load(&mut self, expr: Expr, ctx: &mut brass::Context<Msg>) {
        let guard = ctx.run_map(
            async move {
                let select = factordb::query::select::Select::new()
                    .with_filter(expr)
                    .with_limit(1000);
                let page = crate::api().select(select).await?;
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
        let mut s = Self {
            loader: LoadState::Idle,
            expr: props.expr.clone().unwrap_or_else(|| Self::default_expr()),
            index: 0,
            playing: false,
            muted: false,
            cycle: true,
            shuffle: false,
            autoplay_interval: Some(std::time::Duration::from_secs(5)),
            settings_active: false,
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
            Msg::FilterChanged(expr) => {
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
        }
    }

    fn render(
        &self,
        _props: &Self::Properties,
        mut ctx: brass::RenderContext<brass::PropWrapper<Self>>,
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
            .on_click(ctx.on_simple(|| Msg::TogglePlay));
        let btn_prev = brass_bulma::button()
            .and(brass_bulma::icon_fa("fas fa-chevron-circle-left"))
            .attr(Attr::Title, "Back")
            .on_click(ctx.on_simple(|| Msg::Prev));
        let btn_next = brass_bulma::button()
            .and(brass_bulma::icon_fa("fas fa-chevron-circle-right"))
            .attr(Attr::Title, "Forward")
            .on_click(ctx.on_simple(|| Msg::Next));

        let controls = vdom::div()
            .class(s("mr-4"))
            .and((btn_play, btn_prev, btn_next));

        let btn_shuffle = brass_bulma::button()
            .and(brass_bulma::icon_fa(s("fas fa-random")))
            .and_class_if(self.shuffle, "is-info")
            .attr(Attr::Title, s("Shuffle"))
            .on_click(ctx.on_simple(|| Msg::ToggleShuffle));
        let btn_cycle = brass_bulma::button()
            .and(brass_bulma::icon_fa(s("fas fa-undo")))
            .and_class_if(self.cycle, "is-info")
            .attr(Attr::Title, s("Loop"))
            .on_click(ctx.on_simple(|| Msg::ToggleLoop));
        let btn_mute = brass_bulma::button()
            .and(brass_bulma::icon_fa(s(if self.muted {
                "fas fa-volume-slash"
            } else {
                "fas fa-volume"
            })))
            .and_class_if(self.cycle, "is-info")
            .attr(Attr::Title, s(if self.muted { "Unmute" } else { "Mute" }))
            .on_click(ctx.on_simple(|| Msg::ToggleMuted));
        let btn_settings = brass_bulma::button()
            .and(brass_bulma::icon_fa(s("fas fa-cog")))
            .and_class_if(self.settings_active, "is-info")
            .attr(Attr::Title, s("Settings"))
            .on_click(ctx.on_simple(|| Msg::ToggleSettings));

        let bar_settings =
            vdom::div()
                .class(s("mb-4"))
                .and((btn_shuffle, btn_cycle, btn_mute, btn_settings));

        let settings = if self.settings_active {
            tracing::trace!(?self.autoplay_interval, "current autoplay");
            let interval = brass_bulma::FieldHorizontal {
                label: s("Duration"),
                help: Some(Help {
                    message: s("Time that static media (like images) is displayed").render(),
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

            brass_bulma::box_().and_class("mb-4").and(interval)
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

        vdom::div()
            .style_raw(s(
                "height: 100%; width: 100%; overflow: hidden; display: flex; flex-direction: column;",
            ))
            .and(bar)
            .and(settings)
            .and(player)
            .build()
    }
}
