use std::{cell::RefCell, ops::Not, rc::Rc, time::Duration};

use brass::{
    component::{build_component, msg::MsgComponent, Context, Handle},
    dom::{builder::div, Render, TagBuilder, View},
    effect::{set_timeout, TimeoutGuard},
    signal::signal::{Mutable, Signal, SignalExt},
};
use factordb::{query::select::Item, schema::AttrMapExt};
use rand::prelude::SliceRandom;
use semantic_core::base::entity_title;
use semantic_ui_core::{
    components::{entity::entity_view::EntityView, util::notification_warning},
    context,
    registry::{DynMediaHandle, MediaRenderEvent, MediaRenderOpts},
    EntityRenderOpts, SharedRegistry,
};

struct Props {
    shared: SharedState,

    handle: Rc<RefCell<Option<Handle<State>>>>,
}

#[derive(Clone)]
pub struct ActiveItem {
    pub item: Item,
    pub index: Index,
    pub total_count: usize,
    pub title: String,
}

// TODO: remove allow
#[allow(dead_code)]
enum Msg {
    Next,
    Prev,
    Play,
    Pause,
    TogglePaused,
    AutoplayTimeout,
    Shuffle,
    Mute(bool),
    ToggleMute,
    ToggleCycle,
    ReplaceItems(Vec<Item>),
    AppendItems(Vec<Item>),
    ItemEvent {
        index: usize,
        event: MediaRenderEvent,
    },
}

type Index = usize;

#[derive(Clone, Copy)]
pub struct Position {
    pub index: usize,
    pub total: usize,
}

struct SharedStateData {
    items: RefCell<Vec<Item>>,
    playing: Mutable<bool>,
    cycle: Mutable<bool>,
    muted: Mutable<bool>,
    autoplay_interval: Mutable<Option<std::time::Duration>>,
    active_item: Mutable<Option<ActiveItem>>,
    position: Mutable<Position>,
}

type SharedState = Rc<SharedStateData>;

struct State {
    shared: SharedState,
    registry: SharedRegistry,

    timeout_guard: Option<TimeoutGuard>,
    current_handle: Option<DynMediaHandle>,
    current_is_finished: bool,

    // TODO: Remove hack
    // The RefCell<Option<>> is a stupid workaround because
    // TagBuilder can't be cloned, and a Mutable<> signal with .signal
    // requires cloning. Refactor once brass is improved.
    dom_item: Mutable<RefCell<View>>,
}

impl State {
    fn get_index(&self) -> Index {
        self.shared
            .active_item
            .get_cloned()
            .map(|item| item.index)
            .unwrap_or(0)
    }

    fn next(&mut self, ctx: &Context<Self>) {
        let index = self.get_index();

        if index + 1 >= self.shared.items.borrow().len() {
            if self.shared.cycle.get() {
                self.goto(0, ctx)
            } else {
                self.shared.playing.set(false);
            }
        } else {
            self.goto(index + 1, ctx)
        }
    }

    fn prev(&mut self, ctx: &Context<Self>) {
        let index = self.get_index();
        if index == 0 {
            if self.shared.cycle.get() {
                let len = self.shared.items.borrow().len();
                self.goto(len - 1, ctx);
            } else {
                self.shared.playing.set(false);
            }
        } else {
            self.goto(index - 1, ctx);
        }
    }

    fn start(&mut self, new_index: Option<Index>, ctx: &Context<Self>) {
        self.shared.playing.set(true);
        if let Some(new) = new_index {
            self.goto(new, ctx);
        } else {
            if let Some(handle) = &self.current_handle {
                if !self.current_is_finished {
                    handle.play();
                    self.shared.playing.set(true);
                } else {
                    self.next(ctx);
                }
            } else {
                self.next(ctx);
            }
        }
    }

    fn stop(&mut self) {
        // Pause media playback.
        if let Some(handle) = &self.current_handle {
            handle.pause();
        }
        // Remove active timeout guard to prevent continuing.
        self.timeout_guard.take();
        self.shared.playing.set(false);
    }

    fn goto(&mut self, index: Index, ctx: &Context<Self>) {
        let items = self.shared.items.borrow();

        let item = if let Some(item) = items.get(index) {
            item
        } else {
            tracing::error!(
                index,
                "Internal media player error: tried to show index that does not exist"
            );
            return;
        };

        let ty = if let Some(ty) = item.data.get_type_name() {
            ty
        } else {
            // TODO: implement
            std::mem::drop(items);
            self.next(ctx);
            return;
        };

        // ctx.handle().get

        let is_playing = self.shared.playing.get();
        let is_muted = self.shared.muted.get();

        let (content, media_handle) = if let Some(media_render) =
            self.registry.get_media_renderer(ty)
        {
            let handle = ctx.handle();
            let index = index;

            let (tag, x) = (media_render.render)(
                item,
                &MediaRenderOpts {
                    playing: is_playing,
                    muted: is_muted,
                    callback: Rc::new(move |event| handle.send(Msg::ItemEvent { index, event })),
                },
            );
            (tag.into_view(), x)
        } else if let Some(regular_renderer) = self.registry.entity_content_renderer(ty) {
            let content = regular_renderer(
                item,
                &EntityRenderOpts {
                    editable: false,
                    preview: false,
                },
            )
            .into_view();
            (content, None)
        } else {
            let content = EntityView::from_item(
                item,
                &self.registry,
                &EntityRenderOpts {
                    editable: false,
                    preview: false,
                },
            )
            .render();
            (content, None)
        };

        if let Some(old_media) = self.current_handle.take() {
            old_media.on_remove();
        }

        self.current_handle = media_handle.clone();

        self.timeout_guard.take();
        if is_playing && media_handle.is_none() {
            if let Some(duration) = self.shared.autoplay_interval.get() {
                let callback = ctx.callback_msg(|| Msg::AutoplayTimeout);
                let guard = set_timeout(duration, callback);
                self.timeout_guard = Some(guard);
            }
        }

        self.shared.position.set(Position {
            index,
            total: self.shared.items.borrow().len(),
        });
        self.shared.active_item.set(Some(ActiveItem {
            item: item.clone(),
            index,
            total_count: items.len(),
            title: entity_title(&item.data),
        }));

        self.dom_item.set(RefCell::new(content));
    }

    fn set_muted(&mut self, muted: bool) {
        self.shared.muted.set(muted);
        if let Some(handle) = &self.current_handle {
            handle.set_muted(muted);
        }
    }
}

impl MsgComponent for State {
    type Properties = Props;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: Context<Self>) -> Self {
        let mut s = Self {
            shared: props.shared,
            registry: context::registry(),
            timeout_guard: None,
            current_handle: None,
            current_is_finished: false,
            dom_item: Mutable::new(RefCell::new(View::Empty)),
        };

        s.goto(0, &ctx);

        *props.handle.borrow_mut() = Some(ctx.handle());

        s
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            Msg::AutoplayTimeout => {
                self.next(&ctx);
            }
            Msg::ItemEvent { index: _, event } => match event {
                MediaRenderEvent::Finished(_) => {
                    self.current_is_finished = true;
                    if self.shared.playing.get() {
                        self.next(&ctx);
                    }
                }
                MediaRenderEvent::Paused => {
                    self.shared.playing.set(false);
                }
                MediaRenderEvent::Resumed => {
                    self.shared.playing.set(true);
                }
            },
            Msg::Next => {
                self.next(&ctx);
            }
            Msg::Prev => {
                self.prev(&ctx);
            }
            Msg::Play => {
                self.start(None, &ctx);
            }
            Msg::Pause => {
                self.stop();
            }
            Msg::TogglePaused => {
                if self.shared.playing.get() {
                    self.stop();
                } else {
                    self.start(None, &ctx);
                }
            }
            Msg::Mute(flag) => {
                self.set_muted(flag);
            }
            Msg::ToggleMute => {
                self.set_muted(self.shared.muted.get().not());
            }
            Msg::ReplaceItems(items) => {
                let len = items.len();
                *self.shared.items.borrow_mut() = items;
                if len > 0 {
                    self.goto(0, &ctx);
                } else {
                    self.dom_item.set(RefCell::new(
                        notification_warning().and("Nothing found").into_view(),
                    ));
                }
            }
            Msg::AppendItems(items) => {
                let mut current_items = self.shared.items.borrow_mut();
                let was_empty = items.is_empty();
                current_items.extend(items);
                if was_empty {
                    std::mem::drop(current_items);
                    self.goto(0, &ctx);
                }
            }
            Msg::Shuffle => {
                self.shared
                    .items
                    .borrow_mut()
                    .shuffle(&mut rand::thread_rng());
                self.goto(0, &ctx);
            }
            Msg::ToggleCycle => {
                self.shared.cycle.replace_with(|f| !*f);
            }
        }
    }

    fn render(&mut self, _ctx: Context<Self>) -> TagBuilder {
        div()
            .style_raw("height: 100%; width: 100%; overflow: hidden; display: flex; justify-content: center; align-items: center; flex-grow: 1;")
            .signal(self.dom_item.signal_ref(|cell| {
                let content = std::mem::take(&mut *cell.borrow_mut());
                content
            }))
    }
}

#[derive(Clone)]
pub struct PlayerHandle(Rc<PlayerHandleInner>);

struct PlayerHandleInner {
    handle: Handle<State>,
    shared: SharedState,
}

impl PlayerHandle {
    pub fn active_item(&self) -> Option<Item> {
        if let Some(active) = &*self.0.shared.active_item.lock_ref() {
            Some(active.item.clone())
        } else {
            None
        }
    }

    pub fn replace_items(&self, items: Vec<Item>) {
        self.0.handle.send(Msg::ReplaceItems(items));
    }

    pub fn with_items<O, F: FnOnce(&[Item]) -> O>(&self, f: F) -> O {
        f(&*self.0.shared.items.borrow())
    }

    // pub fn append_items(&self, items: Vec<Item>) {
    //     self.0.handle.send(Msg::AppendItems(items));
    // }

    pub fn shuffle(&self) {
        self.0.handle.send(Msg::Shuffle);
    }

    pub fn start(&self) {
        self.0.handle.send(Msg::Play);
    }

    pub fn pause(&self) {
        self.0.handle.send(Msg::Pause);
    }

    pub fn toggle_paused(&self) {
        self.0.handle.send(Msg::TogglePaused);
    }

    pub fn next(&self) {
        self.0.handle.send(Msg::Next);
    }

    pub fn prev(&self) {
        self.0.handle.send(Msg::Prev);
    }

    // pub fn set_muted(&self, flag: bool) {
    //     self.0.handle.send(Msg::Mute(flag));
    // }

    pub fn toggle_muted(&self) {
        self.0.handle.send(Msg::ToggleMute);
    }

    pub fn toggle_cycle(&self) {
        self.0.handle.send(Msg::ToggleCycle);
    }

    pub fn signal_playing(&self) -> impl Signal<Item = bool> {
        self.0.shared.playing.signal()
    }

    pub fn is_playing(&self) -> bool {
        self.0.shared.playing.get()
    }

    pub fn signal_muted(&self) -> impl Signal<Item = bool> {
        self.0.shared.muted.signal()
    }

    pub fn signal_cycle(&self) -> impl Signal<Item = bool> {
        self.0.shared.cycle.signal()
    }

    // pub fn signal_interval(&self) -> impl Signal<Item = Option<Duration>> {
    //     self.0.shared.autoplay_interval.signal()
    // }

    pub fn signal_item(&self) -> impl Signal<Item = Option<ActiveItem>> {
        self.0.shared.active_item.signal_cloned()
    }

    // pub fn signal_position(&self) -> impl Signal<Item = Position> {
    //     self.0.shared.position.signal()
    // }

    // pub fn signal_has_previous(&self) -> impl Signal<Item = bool> {
    //     self.0.shared.position.signal().map(|x| x.index > 0)
    // }

    pub fn signal_no_previous(&self) -> impl Signal<Item = bool> {
        self.0.shared.position.signal().map(|x| x.index < 1)
    }

    // pub fn signal_has_next(&self) -> impl Signal<Item = bool> {
    //     self.0.shared.position.signal().map(|x| x.index < x.total)
    // }

    pub fn signal_no_next(&self) -> impl Signal<Item = bool> {
        self.0.shared.position.signal().map(|x| x.index >= x.total)
    }
}

pub struct PlayerViewer {
    /// Initial items.
    pub items: Vec<Item>,
}

impl PlayerViewer {
    pub fn build(self) -> (View, PlayerHandle) {
        let handle = Rc::new(RefCell::new(None));
        let shared = Rc::new(SharedStateData {
            items: RefCell::new(self.items),
            playing: Mutable::new(true),
            cycle: Mutable::new(true),
            muted: Mutable::new(false),
            autoplay_interval: Mutable::new(Some(Duration::from_secs(5))),
            active_item: Mutable::new(None),
            position: Mutable::new(Position { index: 0, total: 0 }),
        });
        let content = build_component::<State>(Props {
            shared: shared.clone(),
            handle: handle.clone(),
        });

        let handle = PlayerHandle(Rc::new(PlayerHandleInner {
            handle: handle.borrow_mut().take().unwrap(),
            shared,
        }));

        (content, handle)
    }
}
