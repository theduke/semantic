use std::collections::HashSet;

use brass::{
    vdom::{self, RefRenderer},
    Callback, PropComponent, Str,
};
use factordb::{
    query::{
        expr::Expr,
        select::{Item, Page, Select},
    },
    schema::{AttrMapExt, AttributeDescriptor},
    AnyError,
};
use semantic_ui_core::loader::LoadState;
use semantics_core::base::AttrTitle;

use super::entity_title;

pub struct EntitySearchAutocomplete {
    pub placeholder: Option<Str>,
    pub filter: Option<Expr>,
    pub attribute: Option<String>,
    pub renderer: Option<RefRenderer<Item>>,
    pub on_select: Callback<Item>,
    pub ignored_ids: Option<HashSet<factordb::Id>>,
}

enum Msg {
    Term(String),
    Loaded(Result<Page<Item>, AnyError>),
    Select(usize),
}

struct State {
    term: Str,
    loader: LoadState<Page<Item>>,
}

brass::enable_props!(wrapped EntitySearchAutocomplete => State);

impl PropComponent for State {
    type Properties = EntitySearchAutocomplete;
    type Msg = Msg;

    fn init(_props: &Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            term: Str::new(),
            loader: LoadState::Idle,
        }
    }

    fn update(
        &mut self,
        msg: Self::Msg,
        props: &Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) {
        match msg {
            Msg::Term(value) => {
                let attr = props
                    .attribute
                    .clone()
                    .unwrap_or_else(|| AttrTitle::QUALIFIED_NAME.to_string());
                let expr = Expr::contains(Expr::Attr(attr.into()), value.trim());
                let expr = if let Some(filter) = &props.filter {
                    expr.and_with(filter.clone())
                } else {
                    expr
                };

                let query = Select::new().with_filter(expr).with_limit(10);

                let guard =
                    ctx.run_map(async move { crate::api().select(query).await }, Msg::Loaded);
                self.loader.set_loading_guarded(guard);
            }
            Msg::Loaded(res) => {
                // Filter out ignored.
                let res = res.map(|mut page| {
                    if let Some(ignored) = &props.ignored_ids {
                        page.items.retain(|item| {
                            item.data
                                .get_id()
                                .map(|id| !ignored.contains(&id))
                                .unwrap_or(true)
                        });
                    }

                    page
                });

                self.loader.set_result(res);
            }
            Msg::Select(index) => {
                if let Some(item) = self
                    .loader
                    .as_success()
                    .and_then(|page| page.items.get(index))
                {
                    props.on_select.send(item.clone());
                    self.term = Str::new();
                    self.loader = LoadState::Idle;
                }
            }
        }
    }

    fn render(
        &self,
        props: &Self::Properties,
        ctx: brass::RenderContext<brass::PropWrapper<Self>>,
    ) -> brass::VNode {
        let input = brass_bulma::Input {
            _type: "text".into(),
            color: brass_bulma::Color::Default,
            placeholder: props.placeholder.clone().into(),
            value: self.term.clone(),
            on_input: ctx
                .on_opt(|ev: web_sys::Event| brass::util::input_event_value(ev).map(Msg::Term)),
        };

        let input_field = vdom::div().and(input).class("mb-4");

        let items = self.loader.render(|page| {
            if page.items.is_empty() {
                brass_bulma::notification_warning("Nothing found...").build()
            } else {
                let items = page.items.iter().enumerate().map(|(index, item)| {
                    let content = props
                        .renderer
                        .as_ref()
                        .map(|r| r.render(&item))
                        .unwrap_or_else(|| vdom::text(entity_title(&item.data)));

                    brass_bulma::button()
                        .and(content)
                        .on_click(ctx.on_simple(move || Msg::Select(index)))
                });

                vdom::div().and_iter(items).build()
            }
        });

        vdom::div().and((input_field, items)).build()
    }
}
