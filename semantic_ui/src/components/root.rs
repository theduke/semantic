use brass::VNode;
use factordb::{schema::AttributeDescriptor, AnyError};

use super::{loader::LoadState, router::Route};

pub struct Root {
    _guard: brass::EffectGuard,
    status: LoadState<()>,
    route: Route,
}

pub enum Msg {
    SchemaLoaded(Result<semantics_core::api::SemanticSchema, AnyError>),
    RouteChange(Route),
}

impl brass::Component for Root {
    type Properties = ();
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let guard = ctx.run_map(
            async move { crate::api().schema().await },
            Msg::SchemaLoaded,
        );

        // Build router.
        let callback = ctx.callback_map(Msg::RouteChange);
        let router = super::router::Router::new(callback);
        ctx.provide(router);

        Self {
            status: LoadState::Loading,
            _guard: guard,
            route: Route::Browse,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::SchemaLoaded(res) => match res {
                Ok(schema) => {
                    let mut registry = semantic_ui_core::Registry::new(schema);

                    registry.register_plugin(crate::base_plugin::BasePlugin);
                    registry.register_plugin(semantic_contrib::ContribPlugin);

                    registry
                        .attr_renderer(&semantics_core::base::AttrPreviewImageUrl::QUALIFIED_NAME)
                        .expect("no custom renderer for preview image");

                    ctx.provide(registry.into_shared());
                    self.status.set_loaded(());
                }
                Err(err) => {
                    self.status.set_failed(err);
                }
            },
            Msg::RouteChange(route) => {
                self.route = route;
            }
        }
    }

    fn render(&self, _ctx: brass::RenderContext<Self>) -> brass::VNode {
        let content = if self.status.is_success() {
            super::router::router(&self.route)
        } else {
            self.status.render(|_| VNode::Empty)
        };
        content
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        false
    }
}
