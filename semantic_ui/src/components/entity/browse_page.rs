use brass::{
    dom::Event::Click,
    vdom::{div, EventCallback},
    Callback, EffectGuard, VNode,
};
use factordb::{
    query::{
        expr::Expr,
        select::{ItemPage, Select},
    },
    AnyError,
};
use semantic_ui_core::{EntityRenderOpts, Registry, RenderContextExt};

use semantic_ui_core::loader::LoadState;

pub struct BrowsePage {
    loader: LoadState<ItemPage>,
    query: Select,
    guard: Option<EffectGuard>,
    filter_callback: Callback<Expr>,
}

pub struct BrowsePageProps {}

pub enum Msg {
    Loaded(Result<ItemPage, AnyError>),
    FilterUpdated(Expr),
    Next,
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
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        let registry = ctx.registry().clone();

        let filter = super::entity_filter::EntityFilterForm {
            on_submit: self.filter_callback.clone(),
        };
        let loader = self.loader.render(move |page| {
            render_page(page, &registry, ctx.callback_ignore_event(|| Msg::Next))
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

fn render_page(page: &ItemPage, registry: &Registry, on_next: EventCallback) -> brass::VNode {
    if page.items.is_empty() {
        return div()
            .and(brass_bulma::notification_warning("Nothing found"))
            .build();
    }

    let items = super::entity_page(
        page,
        registry,
        &EntityRenderOpts {
            editable: false,
            preview: true,
        },
    );

    let next = if page.next_cursor.is_some() {
        let btn = brass_bulma::button_medium().and("More").on(Click, on_next);
        div().and(btn).build()
    } else {
        VNode::Empty
    };

    div().and(items).and(next).build()
}
