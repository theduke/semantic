use brass::vdom::div;
use factordb::{
    query::{expr::Expr, select::Item},
    schema::{AttrMapExt, AttributeDescriptor},
    AnyError,
};
use semantic_ui_core::{EntityRenderOpts, RenderContextExt};

use semantic_ui_core::loader::LoadState;

pub struct EntityPageProps {
    pub ident: factordb::Ident,
}

pub struct EntityPage {
    ident: factordb::Ident,
    loader: LoadState<factordb::query::select::Item>,
    deleting: bool,
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
            loader: LoadState::Loading(Some(guard)),
            deleting: false,
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

            let ty = item.data.get_type_name();
            let renderer = ty.and_then(|ty| reg.entity_page_renderer(ty));

            let content = if let Some(renderer) = renderer {
                renderer(
                    item,
                    &EntityRenderOpts {
                        editable: true,
                        preview: false,
                    },
                )
            } else {
                super::generic_entity_item(
                    item,
                    &reg,
                    &EntityRenderOpts {
                        editable: true,
                        preview: false,
                    },
                )
                .build()
            };

            content
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
