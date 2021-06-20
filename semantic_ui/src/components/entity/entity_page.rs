use brass::{vdom::div, EffectGuard};
use factordb::{
    query::{expr::Expr, select::Item},
    schema::AttributeDescriptor,
    AnyError,
};
use semantic_ui_core::EntityRenderOpts;

use crate::components::{loader::LoadState, RenderContextExt};

pub struct EntityPageProps {
    pub ident: factordb::Ident,
}

pub struct EntityPage {
    ident: factordb::Ident,
    loader: LoadState<factordb::query::select::Item>,
    _guard: EffectGuard,
}

pub enum Msg {
    Loaded(Result<Item, AnyError>),
}

impl brass::Component for EntityPage {
    type Properties = EntityPageProps;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let query = factordb::query::select::Select::new()
            .with_limit(1)
            .with_filter(Expr::eq(
                Expr::Ident(factordb::schema::builtin::AttrId::IDENT),
                Expr::Literal(props.ident.clone().into()),
            ));

        let f = async move { crate::api().select(query).await };
        let guard = ctx.run_map(f, |res| {
            let res = res.and_then(|mut page| {
                page.items
                    .pop()
                    .ok_or_else(|| AnyError::msg("Entity not found"))
            });
            Msg::Loaded(res)
        });

        Self {
            ident: props.ident,
            loader: LoadState::Loading,
            _guard: guard,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Loaded(res) => {
                self.loader.set_result(res);
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        let item = self.loader.render(|item| {
            let reg = ctx.registry();

            super::entity_item(item, &reg, &EntityRenderOpts { editable: true }).build()
        });
        div().and(item).build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if props.ident != self.ident {
            *self = Self::init(props, ctx);
            true
        } else {
            false
        }
    }
}
