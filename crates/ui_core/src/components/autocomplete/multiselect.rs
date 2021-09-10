use std::rc::Rc;

use crate::loader::LoadState;
use brass::{
    dom::Attr,
    vdom::{
        self,
        event::{ClickEvent, InputEvent},
        s,
    },
    Callback, Shared, Str, VNode,
};

#[derive(Clone)]
pub struct Action<T: 'static> {
    pub label: Str,
    pub on: Callback<T>,
}

pub type Index = usize;

pub enum MultiSelectMsg {
    Select(Index),
    Remove(Index),
    Search(String),
    Submit,
    Cancel,
}

pub struct MultiSelectRender<'a, T> {
    pub selected: &'a [T],
    pub available: &'a [T],
    pub search_term: &'a str,
    pub callback: Callback<MultiSelectMsg>,
    pub submit_label: Option<&'a Str>,
    pub cancel_label: Option<&'a Str>,
    pub status: &'a LoadState<()>,
}

#[derive(Clone)]
pub struct MultiSelect<T: Clone + 'static> {
    pub heading: Str,
    pub compare_identity: fn(&T, &T) -> bool,
    pub options: Shared<Vec<T>>,
    pub load_status: LoadState<()>,
    pub initial_selection: Shared<Vec<T>>,
    pub multi: bool,
    pub render: Rc<dyn Fn(MultiSelectRender<'_, T>) -> VNode>,
    pub on_search: Option<Callback<String>>,
    pub load_more: Option<Callback<()>>,
    pub on_select: Option<Callback<Vec<T>>>,
    pub on_submit: Option<Action<Vec<T>>>,
    pub on_cancel: Option<Action<()>>,
}

struct State<T> {
    available: Vec<T>,
    selected: Vec<T>,
    search: String,
}

impl<T: Clone + 'static> brass::vdom::Render for MultiSelect<T> {
    fn render(self) -> VNode {
        <State<T> as brass::PropComponent>::build(self)
    }
}

// brass::enable_props!(wrapped TagSelector => State);
impl<T: Clone + 'static> brass::PropComponent for State<T> {
    type Properties = MultiSelect<T>;
    type Msg = MultiSelectMsg;

    fn init(props: &Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        let available = props
            .options
            .iter()
            .filter(|item| {
                !props
                    .initial_selection
                    .iter()
                    .any(|selected| (props.compare_identity)(item, selected))
            })
            .cloned()
            .collect();
        Self {
            available,
            selected: props.initial_selection.as_ref().clone(),
            search: String::new(),
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            MultiSelectMsg::Select(index) => {
                self.selected.push(self.available.remove(index));

                if let Some(on) = &props.on_select {
                    on.send(self.selected.clone());
                }
            }
            MultiSelectMsg::Remove(index) => {
                self.available.push(self.selected.remove(index));
                if let Some(on) = &props.on_select {
                    on.send(self.selected.clone());
                }
            }
            MultiSelectMsg::Search(term) => {
                if let Some(cb) = &props.on_search {
                    cb.send(term.clone());
                }
                self.search = term;
            }
            MultiSelectMsg::Submit => {
                if let Some(Action { on, .. }) = &props.on_submit {
                    on.send(self.selected.clone());
                }
            }
            MultiSelectMsg::Cancel => {
                if let Some(Action { on, .. }) = &props.on_cancel {
                    on.send(());
                }
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: &mut brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        (props.render)(MultiSelectRender {
            status: &props.load_status,
            selected: &self.selected,
            available: &self.available,
            search_term: &self.search,
            submit_label: props.on_submit.as_ref().map(|x| &x.label),
            cancel_label: props.on_cancel.as_ref().map(|x| &x.label),
            callback: ctx.callback(),
        })
    }

    fn on_property_change(
        &mut self,
        old_props: &Self::Properties,
        new_props: &Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if old_props.options != new_props.options {
            self.available = new_props
                .options
                .iter()
                .filter(|item| {
                    !self
                        .selected
                        .iter()
                        .any(|selected| (new_props.compare_identity)(item, selected))
                })
                .cloned()
                .collect();
        }
        true
    }
}

pub fn multiselect_render_tags<'a, T>(
    args: MultiSelectRender<'a, T>,
    get_name: fn(&T) -> &str,
) -> VNode {
    tracing::trace!(count = ?args.available.len(), "option count");
    let search = {
        let input = vdom::input()
            .class(s("input"))
            .attr(Attr::Placeholder, s("Search..."))
            .attr(Attr::Value, args.search_term)
            .on_callback(
                |ev: InputEvent| ev.value().map(MultiSelectMsg::Search),
                &args.callback,
            );
        vdom::p()
            .class(s("control has-icons-left"))
            .and(input)
            .and(brass_bulma::icon_fa_left("fas fa-search"))
    };

    let actions = {
        let submit = if let Some(label) = args.submit_label {
            brass_bulma::button_medium()
                .and(label)
                .on_callback(|_: ClickEvent| MultiSelectMsg::Submit, &args.callback)
                .build()
        } else {
            VNode::Empty
        };

        let cancel = if let Some(label) = args.cancel_label {
            brass_bulma::button_medium()
                .and(label)
                .on_callback(|_: ClickEvent| MultiSelectMsg::Cancel, &args.callback)
                .build()
        } else {
            VNode::Empty
        };

        brass_bulma::buttons().and((submit, cancel))
    };

    let items = args.selected.iter().enumerate().map(|(index, item)| {
        brass_bulma::tag_with_delete(
            get_name(item).into(),
            &args
                .callback
                .clone()
                .map(move |_| MultiSelectMsg::Remove(index)),
        )
        .style_raw(s("cursor:pointer;"))
    });
    let selected = brass_bulma::tags().and_iter(items);

    let loader = if args.status.is_loading() {
        vdom::div().and(crate::components::DelayedSpinner {})
    } else {
        vdom::div()
    };

    let option_values = args.available.iter().enumerate().map(|(index, item)| {
        brass_bulma::bulma_tag(get_name(item).into())
            .on_callback(
                move |_: ClickEvent| MultiSelectMsg::Select(index),
                &args.callback,
            )
            .style_raw(s("cursor:pointer;"))
    });
    let options = brass_bulma::tags().and_iter(option_values);

    brass_bulma::box_()
        .and((search, actions, selected, vdom::hr(), loader, options))
        .build()
}
