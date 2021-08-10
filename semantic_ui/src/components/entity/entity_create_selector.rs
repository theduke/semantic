use brass::vdom;
use semantic_ui_core::{routing::Route, ContextExt};

pub enum Msg {
    Create(String),
}

pub struct EntityCreateSelectorPage {
    entities: Vec<semantic_ui_core::EntityInfo>,
}

impl brass::Component for EntityCreateSelectorPage {
    type Properties = ();
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let entities = ctx
            .registry()
            .creatable_entities()
            .into_iter()
            .map(|x| x.clone())
            .collect();
        Self { entities }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::Create(entity_type) => {
                ctx.router().goto(Route::EntityCreate { entity_type });
            }
        }
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> brass::VNode {
        let content = if self.entities.is_empty() {
            brass_bulma::notification_warning("No creatable entity types found.").build()
        } else {
            let items = self.entities.iter().map(|entity| {
                let title = entity.schema.pretty_name();
                let type_ident = entity.schema.ident.clone();

                let btn = brass_bulma::button_medium()
                    .on(
                        brass::dom::Event::Click,
                        ctx.on_simple(move || Msg::Create(type_ident.clone())),
                    )
                    .and(title);
                vdom::div().and(btn)
            });
            vdom::div().and_iter(items).build()
        };

        vdom::div()
            .and(brass_bulma::h2_with("Create"))
            .and(content)
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
