use brass::{vdom::div, VNode};
use factordb::{query::select::Item, schema::AttrMapExt};
use semantic_ui_core::{
    ContextExt, DynEntityRenderer, EntityInfo, EntityRenderOpts, RenderContextExt,
};

use super::entity_title;

pub struct EntityView {
    pub item: Item,
    pub options: EntityRenderOpts,
}

enum Msg {
    ToggleActions,
    Open,
}

struct EntityViewComponent {
    item: Item,
    options: EntityRenderOpts,
    actions_active: bool,

    title: String,
    type_name: Option<String>,
    content_renderer: Option<DynEntityRenderer>,
    info: Option<EntityInfo>,
}

brass::enable_props!(EntityView => EntityViewComponent);

impl brass::Component for EntityViewComponent {
    type Properties = EntityView;
    type Msg = Msg;

    fn init(props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let item = props.item;
        let options = props.options;

        let registry = ctx.registry();
        let title = entity_title(&item.data);

        let info = item
            .data
            .get_type_name()
            .and_then(|name| registry.entity(name));
        let type_name = super::entity_type_name(&item.data, info);
        let content_renderer = info
            .and_then(|info| registry.entity_content_renderer(&info.schema.ident))
            .cloned();

        Self {
            item,
            options,
            title,
            type_name,
            actions_active: false,
            content_renderer,
            info: info.cloned(),
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::ToggleActions => {
                self.actions_active = !self.actions_active;
            }
            Msg::Open => {
                if let Some(id) = self.item.data.get_id() {
                    ctx.router()
                        .goto(semantic_ui_core::router::Route::Entity(id.into()));
                }
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        // Header.
        let title = brass_bulma::card_header_title(&self.title)
            .style_raw("flex-grow: 0; cursor: pointer;")
            .on_click(ctx.callback_ignore_event(|| Msg::Open));
        let ty = self
            .type_name
            .as_ref()
            .map(|name| {
                div()
                    .and(name)
                    .class("is-flex is-align-items-center mr-3")
                    .build()
            })
            .unwrap_or(VNode::Empty);

        let actions_dropdown = brass_bulma::Dropdown {
            trigger: brass_bulma::icon_fa("fas fa-cog"),
            content: "hello",
            is_hoverable: true,
            is_active: self.actions_active,
            on_toggle: ctx.callback_ignore_event(|| Msg::ToggleActions),
        };
        let actions = div().and(actions_dropdown).style_raw("margin-left: auto;");

        let header = brass_bulma::card_header().and((title, ty, actions)).build();

        let content = if let Some(renderer) = &self.content_renderer {
            renderer(&self.item, &self.options)
        } else {
            super::entity_fields_table(&self.item.data, self.info.as_ref(), ctx.registry()).build()
        };

        let card_content = brass_bulma::card_content().and(content);
        brass_bulma::card()
            .and_class("mb-4")
            .and((header, card_content))
            .build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        // TODO: can we avoid this?
        self.item = props.item;
        true
    }
}
