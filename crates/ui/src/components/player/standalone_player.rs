use brass::{
    component::{msg::MsgComponent, Context},
    dom::{builder::div, Attr, ClickEvent, Event, Render, Tag, TagBuilder, View},
    effect::EventSubscription,
    signal::signal::{Mutable, SignalExt},
};
use semantic_core::base::{AttrBlobUri, Video};
use semantic_ui_core::{
    components::{
        entity::entity_filter,
        loader::Loader,
        util::{box_, button, icon_fas, Cls},
    },
    context,
};
use wasm_bindgen::JsCast;
use web_sys::Element;

use factordb::{
    prelude::Select,
    query::{expr::Expr, select::Item},
    schema::{builtin::AttrType, AttrMapExt, EntityDescriptor},
    AnyError,
};

use super::player_viewer::PlayerHandle;

pub struct StandalonePlayer {
    pub filter: Option<Expr>,
    pub keyboard_controls: bool,
}

impl Render for StandalonePlayer {
    fn render(self) -> View {
        brass::component::build_component::<State>(self)
    }
}

enum Msg {
    FilterChanged(entity_filter::EntityFilter),
    Loaded(Result<Vec<Item>, AnyError>),
    ToggleSettings,
    ToggleFullscreen,
    KeyPress(String),
    OnFullscreenChange { is_fullscreen: bool },
    DownloadPlaylist,
}

struct State {
    loader: Loader<()>,

    base_filter: Expr,
    select: Select,

    fullscreen: bool,
    settings_active: Mutable<bool>,

    // Never read, but must be kept alive.
    _keydown_subscription: Option<EventSubscription>,
    player: PlayerHandle,

    dom_player: Option<Element>,
    rendered_player: View,
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

    fn load(&mut self, select: Select, ctx: Context<Self>) {
        let api = context::api().clone();
        self.select = select.clone();
        let guard = ctx.spawn_map(
            async move {
                let select = select.with_limit(100_000);
                let page = api.select(select).await?;
                Ok(page.items)
            },
            Msg::Loaded,
        );
        self.loader.set_loading(guard);
    }

    fn toggle_fullscreen(&mut self) {
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

        let base_filter = props.filter.clone().unwrap_or_else(|| Self::default_expr());
        let select = Select::new().with_filter(base_filter.clone());

        let mut s = Self {
            loader: Loader::new_idle(),
            base_filter,
            select,
            fullscreen: false,
            settings_active: Mutable::new(false),
            _keydown_subscription,
            dom_player: None,
            player,
            rendered_player,
        };

        s.load(s.select.clone(), ctx);

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
                let mut select = filter.build_select();

                select.filter = Some(
                    select
                        .filter
                        .map(|e| e.and_with(self.base_filter.clone()))
                        .unwrap_or_else(|| self.base_filter.clone()),
                );

                self.load(select, ctx);
            }
            Msg::ToggleSettings => {
                self.settings_active.replace_with(|old| !*old);
            }
            Msg::ToggleFullscreen => {
                self.toggle_fullscreen();
            }

            Msg::KeyPress(key) => {
                if self.settings_active.get() {
                    if key == "Escape" {
                        self.settings_active.set(false);
                    }
                    return;
                }
                match key.as_str() {
                    "ArrowLeft" | "KeyH" | "KeyK" => {
                        self.player.prev();
                    }
                    "ArrowRight" | "KeyL" | "KeyJ" => {
                        self.player.next();
                    }
                    "Space" => {
                        self.player.toggle_paused();
                    }
                    "KeyM" => {
                        self.player.toggle_muted();
                    }
                    "KeyF" => {
                        self.toggle_fullscreen();
                    }
                    "KeyS" => {
                        self.player.shuffle();
                    }
                    "KeyC" => {
                        self.settings_active.replace_with(|old| !*old);
                    }
                    _other => {
                        tracing::trace!(?_other, "unhandled keypress");
                    }
                }
            }
            Msg::OnFullscreenChange { is_fullscreen } => {
                self.fullscreen = is_fullscreen;
            }
            Msg::DownloadPlaylist => {
                let content = self.player.with_items(|items| {
                    let hostname = brass::web::window().location().origin().unwrap();

                    items
                        .iter()
                        .filter_map(|item| {
                            item.data
                                .get_type_name()
                                .filter(|t| t == &Video::QUALIFIED_NAME)
                                .and_then(|_| item.data.get_attr::<AttrBlobUri>())
                                .map(|uri| {
                                    format!(
                                        "{}{}",
                                        hostname,
                                        semantic_ui_core::base::build_blob_url(&uri)
                                    )
                                })
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                });

                let data = js_sys::Array::new();
                data.push(&js_sys::JsString::from(content));
                let blob = web_sys::Blob::new_with_str_sequence(&data).unwrap();
                let blob_url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();

                let a = brass::web::window()
                    .document()
                    .unwrap()
                    .create_element("a")
                    .unwrap()
                    .dyn_into::<web_sys::HtmlAnchorElement>()
                    .unwrap();
                a.set_href(&blob_url);
                a.set_download("playlist.m3u");

                a.click();

                web_sys::Url::revoke_object_url(&blob_url).unwrap();
            }
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
            .attr_signal_toggle(Attr::Disabled, player.signal_no_previous())
            .on(move |_: ClickEvent| player.prev());

        let player = self.player.clone();
        let btn_next = button()
            .and(icon_fas("fa-chevron-circle-right"))
            .attr_signal_toggle(Attr::Disabled, player.signal_no_next())
            .attr(Attr::Title, "Forward")
            .on(move |_: ClickEvent| player.next());

        let position_info = self.player.signal_item().map(|item| -> View {
            item.map(|item| {
                let pos_text = format!("{} / {}", item.index + 1, item.total_count);
                let pos = div()
                    .class("mr-4")
                    .and(div().class(Cls::Button).class(Cls::IsStatic).and(pos_text));

                pos.into_view()
            })
            .unwrap_or(View::Empty)
        });

        let controls = div()
            .class("ml-4")
            .class("mr-4")
            .class(Cls::Buttons)
            .and((btn_play, btn_prev, btn_next))
            .signal(position_info);

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
                    .attr_signal(
                        Attr::Title,
                        player
                            .signal_muted()
                            .map(|flag| if flag { "Unmute" } else { "Mute" }),
                    )
                    .class_signal(player.signal_muted().map(|flag| {
                        if flag {
                            "fa-volume-off"
                        } else {
                            "fa-volume-up"
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
            .and(icon_fas("fa-search"))
            .class_signal_toggle("is-info", self.settings_active.signal())
            .attr(Attr::Title, "Filter")
            .on(ctx.on(|_: ClickEvent| Msg::ToggleSettings));

        let btn_download_playlist = button()
            .and(icon_fas("fa-download"))
            .attr(Attr::Title, "Download playlist")
            .on(ctx.on(|_: ClickEvent| Msg::DownloadPlaylist));

        let bar_settings = div().class("mr-4").class(Cls::Buttons).and((
            btn_shuffle,
            btn_cycle,
            btn_mute,
            btn_fullscreen,
            btn_download_playlist,
            btn_settings,
        ));

        let item_info = self.player.signal_item().map(|item| -> View {
            item.map(|item| {
                let title = div()
                    .class(Cls::Button)
                    .class(Cls::IsStatic)
                    .and(item.title);

                div()
                    .style_raw("flex-shrink: 1; margin: 0 2rem;")
                    .and(title)
                    .into_view()
            })
            .unwrap_or(View::Empty)
        });

        let handle = ctx.handle();
        let settings = div().signal(self.settings_active.signal().map(move |flag| -> View {
            if !flag {
                View::Empty
            } else {
                let filter = entity_filter::entity_filter(handle.on(Msg::FilterChanged));
                let content = box_().class("mb-4").and((Tag::Hr.new(), filter));
                content.into()
            }
        }));

        let bar = div()
            .style_raw("display: flex; margin-bottom: 1rem; align-items: flex-start; flex-grow: 0; justify-content: space-between;")
            .and(controls)
            .signal(item_info)
            .and(bar_settings);

        let player_wrap = div()
            .style_raw("flex-grow: 1; overflow: hidden;")
            .class_signal_toggle(Cls::IsHidden, self.loader.signal_loading())
            .on_event(
                Event::FullScreenChange,
                ctx.on_opt(|_ev: web_sys::Event| {
                    let is_fullscreen = brass::web::document_fullscreen_element().is_some();
                    Some(Msg::OnFullscreenChange { is_fullscreen })
                }),
            )
            .and(std::mem::take(&mut self.rendered_player));

        let loader = self.loader.signal_render(|_| View::Empty);

        self.dom_player = Some(player_wrap.elem().clone());

        div()
            .style_raw(
                "height: 100%; width: 100%; overflow: hidden; display: flex; flex-direction: column;",
            )
            .and(bar)
            .and(settings)
            .signal(loader)
            .and(player_wrap)
    }
}
