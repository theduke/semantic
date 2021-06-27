use factordb::query::select::Item;
use semantic_ui_core::{ContextExt, DynEntityRenderer};

pub struct EntityCreatePageProps {
    pub entity_type: String,
}

pub enum Msg {}

pub struct EntityCreatePage {
    entity_type: String,
    renderer: Option<DynEntityRenderer>,
}

impl brass::Component for EntityCreatePage {
    type Properties = EntityCreatePageProps;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let renderer = ctx
            .registry()
            .entity_create_page_renderer(&props.entity_type)
            .cloned();
        Self {
            renderer,
            entity_type: props.entity_type,
        }
    }

    fn update(&mut self, msg: Self::Msg, _ctx: &mut brass::Context<Self::Msg>) {
        match msg {}
    }

    fn render(&self, _ctx: brass::RenderContext<Self>) -> brass::VNode {
        if let Some(renderer) = &self.renderer {
            let map = Item::default();
            renderer(&map, &semantic_ui_core::EntityRenderOpts { editable: true })
        } else {
            brass_bulma::notification_error(format!("Unknown entity type: '{}'", &self.entity_type))
                .build()
        }
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if props.entity_type != self.entity_type {
            *self = Self::init(props, ctx);
            true
        } else {
            false
        }
    }
}
