use brass::{
    dom::Event::Click,
    vdom::{div, EventCallback},
    EffectGuard, VNode,
};
use factordb::{
    query::select::{ItemPage, Select},
    AnyError,
};
use semantic_ui_core::{EntityRenderOpts, Registry};

use crate::components::{loader::LoadState, RenderContextExt};

pub struct BrowsePage {
    loader: LoadState<ItemPage>,
    query: Select,
    guard: Option<EffectGuard>,
}

pub struct BrowsePageProps {}

pub enum Msg {
    Loaded(Result<ItemPage, AnyError>),
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
        };
        s.load(s.query.clone(), ctx);
        s
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
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
        div()
            .and(self.loader.render(move |page| {
                render_page(page, &registry, ctx.callback(|_: web_sys::Event| Msg::Next))
            }))
            .build()
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
    let items = super::entity_page(page, registry, &EntityRenderOpts { editable: false });

    let next = if page.next_cursor.is_some() {
        let btn = brass_bulma::button_medium().and("More").on(Click, on_next);
        div().and(btn).build()
    } else {
        VNode::Empty
    };

    div().and(items).and(next).build()
}
