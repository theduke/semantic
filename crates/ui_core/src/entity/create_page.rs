use brass::Callback;
use brass_bulma::box_;
use factordb::{query::select::Item, schema::AttrMapExt};

use crate::ContextExt;

use super::persister::{DynEntityFormRenderer, EntityPersister};

pub struct CreatePage {
    pub render: DynEntityFormRenderer,
}

struct CreatePageComp {
    render: DynEntityFormRenderer,
    on_complete: Callback<Item>,
    on_cancel: Callback<()>,
}

brass::enable_props!(CreatePage => CreatePageComp);

enum Msg {
    Created(Item),
    Canceled,
}

impl brass::Component for CreatePageComp {
    type Properties = CreatePage;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self {
            render: props.render,
            on_complete: ctx.callback_map(|item: Item| Msg::Created(item)),
            on_cancel: ctx.callback_map(|_| Msg::Canceled),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Created(item) => {
                if let Some(ident) = item.data.get_ident() {
                    ctx.router().goto(crate::routing::Route::Entity(ident));
                }
            }
            Msg::Canceled => {
                ctx.router().goto(crate::routing::Route::EntityCreateSelect);
            }
        }
    }

    fn render(&self, _ctx: brass::RenderContext<Self>) -> brass::VNode {
        let persister = EntityPersister {
            item: None,
            renderer: self.render.clone(),
            on_complete: self.on_complete.clone(),
            on_cancel: Some(self.on_cancel.clone()),
        };

        box_()
            .and(brass_bulma::h2_with("Create"))
            .and(persister)
            .build()
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        true
    }
}
