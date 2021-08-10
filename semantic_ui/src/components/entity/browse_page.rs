use brass::{
    dom::Event::Click,
    vdom::{div, EventCallback},
    Callback, EffectGuard, VNode,
};
use factordb::{
    query::{
        expr::Expr,
        select::{Item, ItemPage, Select},
    },
    schema::AttrMapExt,
    AnyError,
};
use semantic_ui_core::EntityRenderOpts;

use semantic_ui_core::loader::LoadState;

pub struct BrowsePage {
    loader: LoadState<ItemPage>,
    query: Select,
    guard: Option<EffectGuard>,
    filter_callback: Callback<Expr>,

    on_delete_callback: Callback<Item>,
}

pub struct BrowsePageProps {}

pub enum Msg {
    Loaded(Result<ItemPage, AnyError>),
    FilterUpdated(Expr),
    Next,
    ItemDeleted(Item),
}

impl BrowsePage {
    fn load(&mut self, query: Select, ctx: &mut brass::Context<Msg>) {
        if self.loader.is_loading() {
            // TODO: queue? abort old?
            return;
        }
        let query2 = query.clone();
        let f = async move { crate::api().select(query2).await };

        self.guard = Some(ctx.run_map(f, Msg::Loaded));
        self.loader.set_loading();
        self.query = query;
    }
}

impl brass::Component for BrowsePage {
    type Properties = BrowsePageProps;
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let mut s = Self {
            loader: LoadState::Idle,
            query: Select::new(),
            guard: None,
            filter_callback: ctx.callback_map(Msg::FilterUpdated),
            on_delete_callback: ctx.callback_map(Msg::ItemDeleted),
        };
        s.load(s.query.clone(), ctx);
        s
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::FilterUpdated(expr) => {
                let query = Select::new().with_filter(expr);
                self.load(query, ctx);
            }
            Msg::Loaded(res) => {
                self.loader.set_result(res);
            }
            Msg::Next => {
                let cursor = self
                    .loader
                    .as_success()
                    .and_then(|page| page.next_cursor.as_ref())
                    .cloned();

                if let Some(cursor) = cursor {
                    let q = Select {
                        cursor: Some(cursor),
                        ..self.query.clone()
                    };
                    self.load(q, ctx);
                }
            }
            Msg::ItemDeleted(deleted_item) => {
                if let LoadState::Success(page) = &mut self.loader {
                    page.items
                        .retain(|item| item.data.get_id() != deleted_item.data.get_id());
                }
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        let filter = super::entity_filter::EntityFilterForm {
            on_submit: self.filter_callback.clone(),
        };
        let loader = self.loader.render(move |page| {
            render_page(
                page,
                ctx.on_simple(|| Msg::Next),
                self.on_delete_callback.clone(),
            )
        });

        div().and((filter, loader)).build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        false
    }
}

fn render_page(page: &ItemPage, on_next: EventCallback, on_delete: Callback<Item>) -> brass::VNode {
    if page.items.is_empty() {
        return div()
            .and(brass_bulma::notification_warning("Nothing found"))
            .build();
    }

    let opts = EntityRenderOpts {
        editable: false,
        preview: true,
    };
    let items = page
        .items
        .iter()
        .map(|item| super::entity_view::EntityView {
            item: item.clone(),
            options: opts.clone(),
            on_delete: Some(on_delete.clone()),
        });

    let next = if page.next_cursor.is_some() {
        let btn = brass_bulma::button_medium().and("More").on(Click, on_next);
        div().and(btn).build()
    } else {
        VNode::Empty
    };

    div().and_iter(items).and(next).build()
}
