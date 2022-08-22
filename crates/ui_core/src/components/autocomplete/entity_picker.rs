use std::{pin::Pin, rc::Rc};

use brass::{
    component::{msg::MsgComponent, Component},
    dom::{builder::div, Attr, InputEvent, Tag, TagBuilder, View},
    effect::EffectGuard,
    signal::signal::{Mutable, Signal, SignalExt},
};
use factdb::{DataMap, Expr, Select};
use js_sys::Function;
use semantic_core::base::entity_title;
use wasm_bindgen::JsCast;

use crate::{
    components::{
        entity::build_search_term_expr,
        loader::Loader,
        util::{ButtonBuilder, Cls},
    },
    context,
};

async fn sleep(duration: std::time::Duration) {
    let mut closure: Box<dyn FnMut(Function, Function)> =
        Box::new(move |resolve: js_sys::Function, _reject: Function| {
            brass::web::window()
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    &resolve,
                    duration.as_millis().try_into().unwrap(),
                )
                .unwrap();
        });

    let promise = js_sys::Promise::new(closure.as_mut());
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();

    // NOTE: this is redundant. Just here to make it clear that the clousre needs
    // to be alive until the future finishes.
    std::mem::drop(closure);
}

struct EntityPicker {
    base_filter: Pin<Box<dyn Signal<Item = Expr> + Send + 'static>>,
    filter_builder: Box<dyn Fn(&str) -> Expr>,
    on_select: Rc<dyn Fn(DataMap)>,
}

enum Msg {
    Nop,
    ItemSelected(DataMap),
    Value(String),
    FilterChanged(Expr),
}

struct State {
    base_filter: Option<Expr>,
    filter_builder: Box<dyn Fn(&str) -> Expr>,
    on_select: Rc<dyn Fn(DataMap)>,
    value: Mutable<String>,
    loader: Loader<Vec<DataMap>>,
    _filter_future: EffectGuard,
    input: Option<web_sys::HtmlInputElement>,
}

impl State {
    fn load(&mut self) {
        let val = self.value.lock_ref();
        let trimmed = val.trim();
        if trimmed.is_empty() {
            return;
        }

        let filter = (self.filter_builder)(trimmed);
        let filter = if let Some(base) = &self.base_filter {
            base.clone().and_with(filter)
        } else {
            filter
        };
        let f = async move {
            sleep(std::time::Duration::from_millis(500)).await;
            context::api()
                .select(Select::new().with_limit(10).with_filter(filter))
                .await
        };
        self.loader.spawn(f);
    }
}

impl MsgComponent for State {
    type Properties = EntityPicker;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: brass::component::Context<Self>) -> Self {
        let handle = ctx.handle();
        let _filter_future = ctx.spawn_map(
            props.base_filter.for_each(move |filter| {
                handle.send(Msg::FilterChanged(filter));
                async {}
            }),
            |_| Msg::Nop,
        );

        Self {
            base_filter: None,
            filter_builder: props.filter_builder,
            on_select: props.on_select,
            value: Mutable::new(String::new()),
            loader: Loader::new_idle(),
            _filter_future,
            input: None,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: brass::component::Context<Self>) {
        match msg {
            Msg::ItemSelected(item) => {
                (self.on_select)(item);
                self.value.set(String::new());
                self.loader.set_success(Vec::new());
                if let Some(inp) = &self.input {
                    inp.set_value("");
                    inp.focus().ok();
                }
            }
            Msg::Value(value) => {
                self.value.set(value);
                self.load();
            }
            Msg::Nop => {}
            Msg::FilterChanged(filter) => {
                self.base_filter = Some(filter);
                self.load();
            }
        }
    }

    fn render(&mut self, ctx: brass::component::Context<Self>) -> TagBuilder {
        let input = Tag::Input
            .new()
            .class(Cls::Input)
            .attr(Attr::Placeholder, "Search...")
            .attr_signal(Attr::Value, self.value.signal_cloned())
            .on(ctx.on(|ev: InputEvent| Msg::Value(ev.value().unwrap_or_default())));
        self.input = Some(input.elem().clone().dyn_into().unwrap());

        let handle = ctx.handle();
        let items = self.loader.signal_render(move |items| -> View {
            let handle = handle.clone();
            let options = items.iter().map(move |item| {
                let name = entity_title(&item);
                let handle = handle.clone();

                let item = item.clone();
                ButtonBuilder::new()
                    .label(name)
                    .on(move || {
                        handle.send(Msg::ItemSelected(item.clone()));
                    })
                    .build()
            });
            Tag::Ul.new().and_iter(options).into()
        });

        div().and(div().and(input)).signal(items)
    }
}

pub fn entity_picker(
    base_filter: impl Signal<Item = Expr> + Send + 'static,
    on_select: impl Fn(DataMap) + 'static,
    filter_builder: Option<Box<dyn Fn(&str) -> Expr>>,
) -> View {
    let filter_builder = filter_builder.unwrap_or_else(|| Box::new(build_search_term_expr));

    State::build(EntityPicker {
        base_filter: base_filter.boxed(),
        filter_builder,
        on_select: Rc::new(on_select),
    })
}

pub fn entity_picker_with_filter(
    base_filter: impl Signal<Item = Expr> + Send + 'static,
    filter_builder: impl Fn(&str) -> Expr + 'static,
    on_select: impl Fn(DataMap) + 'static,
) -> View {
    State::build(EntityPicker {
        base_filter: base_filter.boxed(),
        filter_builder: Box::new(filter_builder),
        on_select: Rc::new(on_select),
    })
}
