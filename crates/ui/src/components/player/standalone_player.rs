use brass::{
    component::{msg::MsgComponent, Context},
    dom::{
        builder::{div, span},
        Attr, ClickEvent, Render, Tag, TagBuilder,
    },
    effect::EventSubscription,
    signal::signal::{Mutable, SignalExt},
};
use semantic_ui_core::{
    components::{
        entity::entity_filter::{entity_filter, EntityFilter},
        loader::Loader,
        util::{box_, button, icon_fas, Cls},
    },
    context,
};
use web_sys::Element;

use factordb::{
    query::{expr::Expr, select::Item},
    schema::{builtin::AttrType, EntityDescriptor},
    AnyError,
};

use super::player_viewer::PlayerHandle;

pub struct StandalonePlayer {
    pub filter: Option<Expr>,
    pub keyboard_controls: bool,
}

impl Render for StandalonePlayer {
    fn render(self) -> TagBuilder {
        brass::component::build_component::<State>(self)
    }
}

enum Msg {
    FilterChanged(EntityFilter),
    Loaded(Result<Vec<Item>, AnyError>),
    ToggleSettings,
    ToggleFullscreen,
    KeyPress(String),
}

struct State {
    loader: Loader<()>,

    base_filter: Option<Expr>,
    expr: Expr,

    fullscreen: bool,
    settings_active: Mutable<bool>,

    // Never read, but must be kept alive.
    _keydown_subscription: Option<EventSubscription>,
    player: PlayerHandle,

    dom_player: Option<Element>,
    rendered_player: Option<TagBuilder>,
}

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

    fn load(&mut self, expr: Expr, ctx: Context<Self>) {
        let api = context::api().clone();
        let guard = ctx.spawn_map(
            async move {
                let select = factordb::query::select::Select::new()
                    .with_filter(expr)
                    .with_limit(10_000);
                let page = api.select(select).await?;
                Ok(page.items)
            },
            Msg::Loaded,
        );
        self.loader.set_loading(guard);
    }
}

impl MsgComponent for State {
    type Properties = StandalonePlayer;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: Context<Self>) -> Self {
        let _keydown_subscription = if props.keyboard_controls {
            let handle = ctx.handle();

            Some(brass::effect::EventSubscription::subscribe(
                web_sys::window().unwrap().into(),
                brass::dom::Event::KeyDown,
                move |e: web_sys::KeyboardEvent| handle.send(Msg::KeyPress(e.code())),
            ))
        } else {
            None
        };

        let (rendered_player, player) =
            super::player_viewer::PlayerViewer { items: Vec::new() }.build();

        let mut s = Self {
            loader: Loader::new_idle(),
            expr: props.filter.clone().unwrap_or_else(|| Self::default_expr()),
            base_filter: props.filter,
            fullscreen: false,
            settings_active: Mutable::new(false),
            _keydown_subscription,
            dom_player: None,
            player,
            rendered_player: Some(rendered_player),
        };

        s.load(s.expr.clone(), ctx);

        s
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::Loaded(res) => match res {
                Ok(items) => {
                    self.player.replace_items(items);
                    self.loader.set_result(Ok(()));
                }
                Err(err) => {
                    self.loader.set_result(Err(err));
                }
            },
            Msg::FilterChanged(filter) => {
                let expr = filter.build_expr();

                self.expr = if let Some(base) = &self.base_filter {
                    base.clone().and_with(expr)
                } else {
                    expr
                };
                self.load(self.expr.clone(), ctx);
            }
            Msg::ToggleSettings => {
                self.settings_active.replace_with(|old| !*old);
            }
            Msg::ToggleFullscreen => {
                if !self.fullscreen {
                    if let Some(elem) = &self.dom_player {
                        if let Err(_error) = elem.request_fullscreen() {
                            tracing::error!("Could not launch fullscreen mode");
                        } else {
                            self.fullscreen = true;
                        }
                    }
                } else {
                    self.fullscreen = false;
                    web_sys::window()
                        .unwrap()
                        .document()
                        .unwrap()
                        .exit_fullscreen();
                }
            }

            Msg::KeyPress(key) => match key.as_str() {
                "ArrowLeft" => {
                    self.player.prev();
                }
                "ArrowRight" => {
                    self.player.next();
                }
                " " => {
                    self.player.toggle_paused();
                }
                "m" => {
                    self.player.toggle_muted();
                }
                _other => {}
            },
        }
    }

    fn render(&mut self, ctx: Context<Self>) -> TagBuilder {
        let player = self.player.clone();

        let btn_play =
            button()
                .attr_signal(
                    Attr::Title,
                    player
                        .signal_playing()
                        .map(|flag| if flag { "Pause" } else { "Play" }),
                )
                .and(
                    Tag::I.new().class(Cls::Fas).class_signal(
                        player
                            .signal_playing()
                            .map(|flag| if flag { "fa-pause" } else { "fa-play" }),
                    ),
                )
                .on(move |_: ClickEvent| {
                    player.toggle_paused();
                });

        let player = self.player.clone();
        let btn_prev = button()
            .and(icon_fas("fa-chevron-circle-left"))
            .attr(Attr::Title, "Back")
            .on(move |_: ClickEvent| player.prev());

        let player = self.player.clone();
        let btn_next = button()
            .and(icon_fas("fa-chevron-circle-right"))
            .attr(Attr::Title, "Forward")
            .on(move |_: ClickEvent| player.next());

        let controls = div().class("mr-4").and((btn_play, btn_prev, btn_next));

        let player = self.player.clone();
        let btn_shuffle = button()
            .and(icon_fas("fa-random"))
            .attr(Attr::Title, "Shuffle")
            .on(move |_: ClickEvent| player.shuffle());

        let player = self.player.clone();
        let btn_cycle = button()
            .and(icon_fas("fa-undo"))
            .class_signal_toggle("is-info", player.signal_cycle())
            .attr(Attr::Title, "Loop")
            .on(move |_: ClickEvent| player.toggle_cycle());

        let player = self.player.clone();
        let btn_mute = button()
            .and(
                Tag::I
                    .new()
                    .class(Cls::Fas)
                    .class_signal(player.signal_muted().map(|flag| {
                        if flag {
                            "fa-volume-slash"
                        } else {
                            "va-volume"
                        }
                    })),
            )
            // TODO re-enable
            // .attr(Attr::Title, s(if self.muted { "Unmute" } else { "Mute" }))
            .on(move |_: ClickEvent| player.toggle_muted());

        let btn_fullscreen = button()
            .and(icon_fas("fa-expand-arrows-alt"))
            .attr(Attr::Title, "Fullscreen")
            .on(ctx.on(|_: ClickEvent| Msg::ToggleFullscreen));

        let btn_settings = button()
            .and(icon_fas("fa-cog"))
            .class_signal_toggle("is-info", self.settings_active.signal())
            .attr(Attr::Title, "Settings")
            .on(ctx.on(|_: ClickEvent| Msg::ToggleSettings));

        let bar_settings = div().class("mb-4").and((
            btn_shuffle,
            btn_cycle,
            btn_mute,
            btn_fullscreen,
            btn_settings,
        ));

        let handle = ctx.handle();
        let settings = div().child_signal_opt(self.settings_active.signal().map(move |flag| {
            if !flag {
                None
            } else {
                let filter = entity_filter(handle.on(Msg::FilterChanged));
                let content = box_().class("mb-4").and((Tag::Hr.new(), filter));
                Some(content)
            }
        }));

        let bar = div()
            .style_raw("display: flex; margin-bottom: 1rem;")
            .and((controls, bar_settings));

        let player_wrap = div()
            .style_raw("flex-grow: 1; height: 100%; width: 100%;")
            .class_signal_toggle(Cls::IsHidden, self.loader.signal_loading())
            .and(self.rendered_player.take());

        let loader = self.loader.signal_render(|_| span());

        self.dom_player = Some(player_wrap.elem().clone());

        div()
            .style_raw(
                "height: 100%; width: 100%; overflow: hidden; display: flex; flex-direction: column;",
            )
            .and(bar)
            .and(settings)
            .child_signal(loader)
            .and(player_wrap)
    }
}
