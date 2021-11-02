use std::rc::Rc;

use brass::{
    component::{msg::MsgComponent, Context, Handle},
    dom::{Attr, ClickEvent, InputEvent, Render, Tag, TagBuilder},
    signal::{
        signal::{Mutable, SignalExt},
        signal_vec::MutableVec,
    },
};
use factordb::AnyError;
use futures::future::LocalBoxFuture;

use crate::{
    components::{
        loader::{spinner, Loader},
        util::{
            box_, bulma_tag, buttons, notification_default, tag_with_delete, tags, ButtonBuilder,
        },
    },
    SharedRenderer0,
};

pub struct Action<T: 'static> {
    pub label: String,
    pub on: Rc<dyn Fn(&T)>,
}

pub type ItemId = String;

pub enum MultiSelectMsg<T> {
    Select(ItemId),
    Remove(ItemId),
    Search(String),
    AsyncAddLoaded(T),
    AsyncRemoveLoaded(T),
    AsyncChangeLoaded(Vec<T>),
    Submit,
    Cancel,

    SetAvailable(Vec<T>),
}

pub struct MultiSelectRender<'a, T: Clone + 'static> {
    // TODO: Refactor once brass has a custom MutableVec
    // (Index, T) because MutableVec does not support enumerate.
    pub selected: &'a MutableVec<(ItemId, T)>,
    /// Fallback content to show if nothing is selected.
    pub selected_fallback: &'a Option<SharedRenderer0>,
    // TODO: Refactor once brass has a custom MutableVec
    // (Index, T) because MutableVec does not support enumerate.
    pub available: &'a MutableVec<(ItemId, T)>,
    /// Fallback content to show if nothing is available.
    pub available_fallback: &'a Option<SharedRenderer0>,
    pub search_term: &'a str,
    pub submit_label: Option<&'a str>,
    pub cancel_label: Option<&'a str>,
    pub status: &'a Loader<()>,

    pub handle: Handle<State<T>>,
}

pub struct MultiSelect<T: Clone + 'static> {
    pub heading: String,
    pub multi: bool,
    pub get_id: fn(&T) -> String,
    pub options: Vec<T>,
    pub initial_selection: Vec<T>,

    /// Fallback content to show if nothing is selected.
    pub selected_fallback: Option<SharedRenderer0>,
    /// Fallback content to show if nothing is available.
    pub available_fallback: Option<SharedRenderer0>,

    pub render: Box<dyn Fn(MultiSelectRender<'_, T>) -> TagBuilder>,
    pub search: Option<Box<dyn Fn(String) -> LocalBoxFuture<'static, Result<Vec<T>, AnyError>>>>,
    pub load_more: Option<Box<dyn Fn(usize)>>,
    pub on_change: Option<Rc<dyn Fn(Vec<T>)>>,
    pub on_change_async:
        Option<Rc<dyn Fn(Vec<T>) -> LocalBoxFuture<'static, Result<Vec<T>, AnyError>>>>,
    pub on_submit: Option<Action<Vec<T>>>,
    pub on_cancel: Option<Action<()>>,
    pub on_add_async: Option<Rc<dyn Fn(T) -> LocalBoxFuture<'static, Result<T, AnyError>>>>,
    pub on_remove_async: Option<Rc<dyn Fn(T) -> LocalBoxFuture<'static, Result<T, AnyError>>>>,
}

impl<T: Clone + 'static> Render for MultiSelect<T> {
    fn render(self) -> TagBuilder {
        brass::component::build_component::<State<T>>(self)
    }
}

pub struct State<T: Clone + 'static> {
    search: Option<Box<dyn Fn(String) -> LocalBoxFuture<'static, Result<Vec<T>, AnyError>>>>,
    get_id: fn(&T) -> String,
    load_more: Option<Box<dyn Fn(usize)>>,
    on_change: Option<Rc<dyn Fn(Vec<T>)>>,
    on_change_async:
        Option<Rc<dyn Fn(Vec<T>) -> LocalBoxFuture<'static, Result<Vec<T>, AnyError>>>>,
    on_submit: Option<Action<Vec<T>>>,
    on_cancel: Option<Action<()>>,
    on_add_async: Option<Rc<dyn Fn(T) -> LocalBoxFuture<'static, Result<T, AnyError>>>>,
    on_remove_async: Option<Rc<dyn Fn(T) -> LocalBoxFuture<'static, Result<T, AnyError>>>>,

    selected_fallback: Option<SharedRenderer0>,
    available_fallback: Option<SharedRenderer0>,
    render: Box<dyn Fn(MultiSelectRender<'_, T>) -> TagBuilder>,

    available: MutableVec<(ItemId, T)>,
    selected: MutableVec<(ItemId, T)>,
    search_term: Mutable<String>,

    loader: Loader<()>,
}

// brass::enable_props!(wrapped TagSelector => State);
impl<T: Clone + 'static> MsgComponent for State<T> {
    type Properties = MultiSelect<T>;
    type Msg = MultiSelectMsg<T>;

    fn init(props: Self::Properties, _ctx: Context<Self>) -> Self {
        let get_id = props.get_id;

        let selected: Vec<_> = props
            .initial_selection
            .into_iter()
            .map(|item| ((get_id)(&item), item))
            .collect();
        let available: Vec<_> = props
            .options
            .into_iter()
            .filter_map(|item| {
                let id = (get_id)(&item);
                if !selected.iter().any(|(id2, _)| &id == id2) {
                    Some((id, item))
                } else {
                    None
                }
            })
            .collect();

        Self {
            render: props.render,

            search: props.search,
            load_more: props.load_more,
            on_change: props.on_change,
            on_change_async: props.on_change_async,
            on_submit: props.on_submit,
            on_cancel: props.on_cancel,
            on_add_async: props.on_add_async,
            on_remove_async: props.on_remove_async,
            available: MutableVec::new_with_values(available),
            selected: MutableVec::new_with_values(selected),
            search_term: Mutable::new(String::new()),
            loader: Loader::new_idle(),
            selected_fallback: props.selected_fallback,
            available_fallback: props.available_fallback,
            get_id,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: Context<Self>) {
        match msg {
            MultiSelectMsg::Select(id) => {
                let mut available = self.available.lock_mut();

                tracing::trace!(?id, available_len=?available.len(), "Msg::Select");

                let (_, value) = if let Some(index) = available.iter().position(|(id2, _)| {
                    tracing::trace!(?id, ?id2, "compare");
                    &id == id2
                }) {
                    available.remove(index)
                } else {
                    return;
                };

                if let Some(on) = &self.on_change_async {
                    if self.loader.is_loading() {
                        return;
                    }

                    let mut values = self
                        .selected
                        .lock_ref()
                        .as_slice()
                        .iter()
                        .map(|(_idx, v)| v.clone())
                        .collect::<Vec<_>>();
                    values.push(value);

                    let handle = ctx.handle();
                    let on = on.clone();
                    self.loader.spawn(async move {
                        let values = on(values).await?;
                        handle.send(MultiSelectMsg::AsyncChangeLoaded(values));
                        Ok(())
                    });
                } else if let Some(on) = &self.on_add_async {
                    if self.loader.is_loading() {
                        return;
                    }

                    let handle = ctx.handle();
                    let on = on.clone();
                    self.loader.spawn(async move {
                        let value = on(value).await?;
                        handle.send(MultiSelectMsg::AsyncAddLoaded(value));
                        Ok(())
                    });
                } else {
                    self.selected.lock_mut().push_cloned((id, value));
                }

                if let Some(on) = &self.on_change {
                    on(self
                        .selected
                        .lock_ref()
                        .as_slice()
                        .iter()
                        .map(|(_, v)| v.clone())
                        .collect());
                }
            }
            MultiSelectMsg::Remove(id) => {
                let mut selected = self.selected.lock_mut();
                let (index, value) =
                    if let Some(index) = selected.iter().position(|(id2, _)| &id == id2) {
                        selected.remove(index)
                    } else {
                        return;
                    };

                if let Some(on) = &self.on_change_async {
                    if self.loader.is_loading() {
                        return;
                    }

                    let values = selected
                        .as_slice()
                        .iter()
                        .map(|(_idx, v)| v.clone())
                        .collect::<Vec<_>>();

                    let handle = ctx.handle();
                    let on = on.clone();
                    self.loader.spawn(async move {
                        let values = on(values).await?;
                        handle.send(MultiSelectMsg::AsyncChangeLoaded(values));
                        Ok(())
                    });
                } else if let Some(on) = &self.on_remove_async {
                    if self.loader.is_loading() {
                        return;
                    }

                    let handle = ctx.handle();
                    let on = on.clone();
                    self.loader.spawn(async move {
                        let value = on(value).await?;
                        handle.send(MultiSelectMsg::AsyncRemoveLoaded(value));
                        Ok(())
                    });
                } else {
                    self.available.lock_mut().push_cloned((index, value));
                }

                if let Some(on) = &self.on_change {
                    on(selected.as_slice().iter().map(|(_, v)| v.clone()).collect());
                }
            }
            MultiSelectMsg::Search(term) => {
                if let Some(cb) = self.search.as_ref() {
                    let f = cb(term.clone());

                    let handle = ctx.handle();
                    self.loader.spawn(async move {
                        let options = f.await?;
                        handle.send(MultiSelectMsg::SetAvailable(options));
                        Ok(())
                    });
                }
                self.search_term.set(term);
            }
            MultiSelectMsg::Submit => {
                if let Some(Action { on, .. }) = &self.on_submit {
                    // TODO: redundant clone...
                    on(&self
                        .selected
                        .lock_ref()
                        .as_slice()
                        .iter()
                        .map(|(_, v)| v.clone())
                        .collect());
                }
            }
            MultiSelectMsg::Cancel => {
                if let Some(Action { on, .. }) = &self.on_cancel {
                    on(&());
                }
            }
            MultiSelectMsg::SetAvailable(items) => {
                let selected = self.selected.lock_ref();

                let items = items
                    .into_iter()
                    .filter_map(|item| {
                        let id = (self.get_id)(&item);
                        if !selected.iter().any(|(id2, _)| &id == id2) {
                            Some((id, item))
                        } else {
                            None
                        }
                    })
                    .collect();

                self.available.lock_mut().replace_cloned(items);
            }
            MultiSelectMsg::AsyncAddLoaded(value) => {
                let mut selected = self.selected.lock_mut();
                selected.push_cloned(((self.get_id)(&value), value));
            }
            MultiSelectMsg::AsyncRemoveLoaded(value) => {
                let mut available = self.available.lock_mut();
                available.push_cloned(((self.get_id)(&value), value));
            }
            MultiSelectMsg::AsyncChangeLoaded(values) => {
                self.selected.lock_mut().replace_cloned(
                    values
                        .into_iter()
                        .map(|value| ((self.get_id)(&value), value))
                        .collect(),
                );
            }
        }
    }

    fn render(&mut self, ctx: Context<Self>) -> TagBuilder {
        (self.render)(MultiSelectRender {
            status: &self.loader,
            selected: &self.selected,
            available: &self.available,
            search_term: self.search_term.lock_ref().as_str(),
            submit_label: self.on_submit.as_ref().map(|x| x.label.as_str()),
            cancel_label: self.on_cancel.as_ref().map(|x| x.label.as_str()),

            handle: ctx.handle(),
            selected_fallback: &self.selected_fallback,
            available_fallback: &self.available_fallback,
        })
    }
}

pub fn multiselect_render_tags<'a, T: Clone>(
    args: MultiSelectRender<'a, T>,
    get_name: fn(&T) -> &str,
) -> TagBuilder {
    let handle = args.handle.clone();
    let search = {
        let input = Tag::Input
            .new()
            .class("input")
            .attr(Attr::Placeholder, "Search...")
            .attr(Attr::Value, args.search_term)
            .on(handle.on_opt(|ev: InputEvent| ev.value().map(MultiSelectMsg::Search)));
        Tag::P
            .new()
            .class("control")
            .class("has-icons-left")
            .and(input)
            .and(
                Tag::Span
                    .new()
                    .class("icon")
                    .class("is-small")
                    .class("is-left")
                    .and(Tag::I.new().class("fas").class("fa-search")),
            )
    };

    let actions = {
        let submit = if let Some(label) = args.submit_label {
            Some(
                ButtonBuilder::new()
                    .size_medium()
                    .label(label)
                    .on(handle.callback(|| MultiSelectMsg::Submit))
                    .build(),
            )
        } else {
            None
        };

        let cancel = if let Some(label) = args.cancel_label {
            Some(
                ButtonBuilder::new()
                    .size_medium()
                    .label(label)
                    .on(handle.callback(|| MultiSelectMsg::Cancel))
                    .build(),
            )
        } else {
            None
        };

        buttons().and((submit, cancel))
    };

    let handle = args.handle.clone();

    let fallback = args
        .selected_fallback
        .as_ref()
        .map(|x| x())
        .unwrap_or_else(|| notification_default().and("Nothing selected yet"));

    let selected = tags().children_signal_with_fallback(
        args.selected.signal_vec_cloned(),
        move |(id, item)| {
            let id = id.clone();
            tag_with_delete(
                get_name(item),
                handle.callback(move || MultiSelectMsg::Remove(id.clone())),
            )
            .build()
        },
        fallback,
    );

    let loader_signal = args.status.signal_loading().map(|loading| {
        if loading {
            // TODO: delayed spinner
            Some(spinner())
        } else {
            None
        }
    });

    let handle = args.handle.clone();

    let fallback = args
        .available_fallback
        .as_ref()
        .map(|x| x())
        .unwrap_or_else(|| notification_default().and("Nothing found. Try searching."));
    let available = tags().children_signal_with_fallback(
        args.available.signal_vec_cloned(),
        move |(id, item)| {
            let id = id.clone();
            bulma_tag()
                .and(get_name(item))
                .on(handle.on(move |_: ClickEvent| MultiSelectMsg::Select(id.clone())))
                .build()
        },
        fallback,
    );

    box_()
        .and(search)
        .and(actions)
        .and(selected)
        .and(Tag::Hr.new())
        .child_signal_opt(loader_signal)
        .and(available)
}
