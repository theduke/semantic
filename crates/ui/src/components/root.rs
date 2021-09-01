use brass::vdom::Render;
use factordb::{schema::AttributeDescriptor, AnyError};
use semantic_ui_core::{
    loader::LoadState,
    routing::{Route, Router},
    ContextExt,
};

use super::router;

#[derive(Clone, Copy, Debug)]
enum Phase {
    CheckingBackend,
    BackendSetup,
    Active,
    LoggingOut,
}

pub enum Msg {
    StatusLoaded(Result<semantic_core::api::ServerStatus, AnyError>),
    SchemaLoaded(Result<semantic_core::api::SemanticSchema, AnyError>),
    Initialize(semantic_core::api::BackendConfig),
    InitializeLoaded(Result<semantic_core::api::SemanticSchema, AnyError>),
    LogoutLoaded(Result<(), AnyError>),
    RouteChange(Route),
}

pub struct Root {
    status: LoadState<()>,
    phase: Phase,
    route: Route,
}

impl brass::Component for Root {
    type Properties = ();
    type Msg = Msg;

    fn init(_props: Self::Properties, ctx: &mut brass::Context<Self::Msg>) -> Self {
        let api = semantic_ui_core::api::new_api(None);
        ctx.provide(api);

        // Read route from url.
        let current_path = brass::util::url_path();
        let route = Route::from_path(&&current_path).unwrap_or(Route::Browse);

        let api = ctx.api().clone();

        let guard = ctx.run(async move {
            let status = match api.server_status().await {
                Ok(s) => s,
                Err(err) => {
                    if err.to_string().contains("ExpiredSignature") {}
                    return Msg::StatusLoaded(Err(err));
                }
            };

            if !status.backend_initialized {
                return Msg::StatusLoaded(Ok(status));
            }

            let schema_res = api.schema().await;
            Msg::SchemaLoaded(schema_res)
        });

        // Build router.
        let callback = ctx.callback_map(Msg::RouteChange);
        let router = Router::new(callback);
        ctx.provide(router);

        Self {
            status: LoadState::Loading(Some(guard)),
            phase: Phase::CheckingBackend,
            route,
        }
    }

    fn update(&mut self, msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        match msg {
            Msg::StatusLoaded(res) => match res {
                Ok(status) => {
                    assert!(
                        status.backend_initialized == false,
                        "internal error: Msg::StatusLoaded must only be sent if backend is not initialized",
                    );
                    self.phase = Phase::BackendSetup;
                    self.status.set_success(());
                }
                Err(err) => {
                    self.status.set_failed(err);
                }
            },
            Msg::Initialize(config) => {
                let api = ctx.api().clone();
                let f = async move {
                    api.initialize(config).await?;
                    api.schema().await
                };
                let guard = ctx.run_map(f, Msg::InitializeLoaded);
                self.status.set_loading_guarded(guard);
            }
            Msg::InitializeLoaded(res) => match res {
                Ok(schema) => self.update(Msg::SchemaLoaded(Ok(schema)), ctx),
                Err(err) => {
                    self.status.set_failed(err);
                }
            },
            Msg::SchemaLoaded(res) => match res {
                Ok(schema) => {
                    let mut registry = semantic_ui_core::Registry::new(schema);

                    registry.register_plugin(crate::components::base::BasePlugin);
                    registry.register_plugin(semantic_contrib::ContribPlugin);

                    registry
                        .attr_renderer(&semantic_core::base::AttrPreviewImageUrl::QUALIFIED_NAME)
                        .expect("no custom renderer for preview image");

                    ctx.provide(registry.into_shared());
                    self.status.set_success(());
                    self.phase = Phase::Active;
                }
                Err(err) => {
                    self.status.set_failed(err);
                }
            },
            Msg::RouteChange(route) => {
                if matches!(route, Route::Logout) {
                    self.phase = Phase::LoggingOut;

                    let api = ctx.api().clone();
                    let guard =
                        ctx.run_map(async move { api.close_backend().await }, Msg::LogoutLoaded);
                    self.status.set_loading_guarded(guard);
                } else {
                    router::history_push_route(&route);
                    self.route = route;
                }
            }
            Msg::LogoutLoaded(res) => match res {
                Ok(_) => {
                    ctx.remove::<semantic_ui_core::Registry>();
                    self.phase = Phase::BackendSetup;
                }
                Err(err) => {
                    self.status.set_failed(err);
                }
            },
        }
    }

    fn render(&self, mut _ctx: brass::RenderContext<Self>) -> brass::VNode {
        match self.phase {
            Phase::CheckingBackend | Phase::LoggingOut => {
                self.status.render(|_| brass::VNode::Empty)
            }
            Phase::BackendSetup => self.status.render(move |_| {
                let title = brass_bulma::h2_with("Login");

                let form = super::backend_setup::BackendSetupForm {
                    on_submit: _ctx.callback_map(Msg::Initialize),
                }
                .render();

                brass_bulma::box_().and((title, form)).build()
            }),
            Phase::Active => super::router::router(&self.route),
        }
    }

    fn on_property_change(
        &mut self,
        _props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        true
    }
}
