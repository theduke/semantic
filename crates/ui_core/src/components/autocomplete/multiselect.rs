use crate::loader::LoadState;
use brass::{
    dom::Attr,
    vdom::{self, s, RefRenderer},
    Callback, Shared, Str, VNode,
};

#[derive(Clone)]
pub struct Action<T: 'static> {
    pub label: Str,
    pub on: Callback<T>,
}

#[derive(Clone)]
pub struct MultiSelect<T: Clone + 'static> {
    pub heading: Str,
    pub compare_identity: fn(&T, &T) -> bool,
    pub options: Shared<Vec<T>>,
    pub load_status: LoadState<()>,
    pub initial_selection: Shared<Vec<T>>,
    pub multi: bool,
    pub render: Shared<RefRenderer<T>>,
    pub on_search: Option<Callback<String>>,
    pub load_more: Option<Callback<()>>,
    pub on_select: Option<Callback<Vec<T>>>,
    pub on_submit: Option<Action<Vec<T>>>,
    pub on_cancel: Option<Action<()>>,
}

impl<T: Clone + 'static> brass::vdom::Render for MultiSelect<T> {
    fn render(self) -> VNode {
        <State<T> as brass::PropComponent>::build(self)
    }
}

type Index = usize;

enum Msg {
    Select(Index),
    Remove(Index),
    Search(String),
    Submit,
    Cancel,
}

struct State<T> {
    selected: Vec<T>,
    search: String,
}

// brass::enable_props!(wrapped TagSelector => State);
impl<T: Clone + 'static> brass::PropComponent for State<T> {
    type Properties = MultiSelect<T>;
    type Msg = Msg;

    fn init(props: &Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
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
            Msg::Select(index) => {
                if let Some(item) = props.options.get(index) {
                    self.selected.push(item.clone());
                }

                if let Some(on) = &props.on_select {
                    on.send(self.selected.clone());
                }
            }
            Msg::Remove(index) => {
                self.selected.remove(index);
                if let Some(on) = &props.on_select {
                    on.send(self.selected.clone());
                }
            }
            Msg::Search(term) => {
                if let Some(cb) = &props.on_search {
                    cb.send(term.clone());
                }
                self.search = term;
            }
            Msg::Submit => {
                if let Some(Action { on, .. }) = &props.on_submit {
                    on.send(self.selected.clone());
                }
            }
            Msg::Cancel => {
                if let Some(Action { on, .. }) = &props.on_cancel {
                    on.send(());
                }
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        let search = if props.on_search.is_some() {
            let input = vdom::input()
                .class(s("input"))
                .attr(Attr::Placeholder, s("Search..."))
                .attr(Attr::Value, &self.search)
                .on(
                    brass::dom::Event::Input,
                    ctx.on_opt(|ev: web_sys::Event| {
                        brass::util::input_event_value(ev).map(Msg::Search)
                    }),
                );
            vdom::p()
                .class(s("control has-icons-left"))
                .and(input)
                .and(brass_bulma::icon_fa_left("fas fa-search"))
        } else {
            vdom::div()
        };

        let actions = {
            let submit = if let Some(Action { label, .. }) = &props.on_submit {
                brass_bulma::button_medium()
                    .and(label)
                    .on_click(ctx.on_simple(|| Msg::Submit))
                    .build()
            } else {
                VNode::Empty
            };

            let cancel = if let Some(Action { label, .. }) = &props.on_cancel {
                brass_bulma::button_medium()
                    .and(label)
                    .on_click(ctx.on_simple(|| Msg::Cancel))
                    .build()
            } else {
                VNode::Empty
            };

            brass_bulma::buttons().and((submit, cancel))
        };

        let selected_items = self.selected.iter().enumerate().map(|(index, item)| {
            let content = props.render.call(item);
            let remover = brass_bulma::icon_fa("fas fa-xmark");
            brass_bulma::panel_block()
                .and_class("is-flex")
                .and((content, remover))
                .on_click(ctx.on_simple(move || Msg::Remove(index)))
        });
        let selected = vdom::div().and_iter(selected_items);

        let loader = if props.load_status.is_loading() {
            vdom::div().and(crate::components::DelayedSpinner {})
        } else {
            vdom::div()
        };

        let option_items = props
            .options
            .iter()
            .enumerate()
            .filter(|(_index, item)| {
                !self
                    .selected
                    .iter()
                    .any(|selected| (props.compare_identity)(selected, item))
            })
            .map(|(index, item)| {
                let content = props.render.call(item);
                let icon = brass_bulma::icon_fa("fas fa-plus");
                brass_bulma::panel_block()
                    .and_class("is-flex")
                    .and((content, icon))
                    .on_click(ctx.on_simple(move || Msg::Select(index)))
            });
        let options = vdom::div().and_iter(option_items);

        brass_bulma::panel()
            .and(brass_bulma::panel_heading(&props.heading))
            .and((search, actions, selected, vdom::hr(), loader, options))
            .build()
    }
}
