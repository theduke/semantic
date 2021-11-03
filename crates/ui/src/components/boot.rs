use std::rc::Rc;

use brass::{
    dom::{
        builder::{div, span},
        TagBuilder,
    },
    effect::{spawn_guarded, EffectGuard},
    signal::signal::Mutable,
};
use factordb::AnyError;
use semantic_core::api::SemanticSchema;
use semantic_ui_core::{
    components::loader::{error_msg, spinner},
    context,
    routing::Route,
    Registry,
};

enum BootPhase {
    Init,
    LoadingStatus(EffectGuard),
    StatusFailed(AnyError),
    SchemaFailed(AnyError),
    SchemaLoaded(SemanticSchema),
    Login,
}

#[derive(Clone)]
pub struct Boot {
    status: Rc<Mutable<BootPhase>>,
}

impl Boot {
    pub fn render() -> TagBuilder {
        let boot = Boot {
            status: Rc::new(Mutable::new(BootPhase::Init)),
        };

        let boot2 = boot.clone();
        let guard = spawn_guarded(async move {
            let api = context::api();

            let a = api.server_status();
            let res = a.await;
            let phase = match res {
                Err(err) => {
                    tracing::trace!("error");
                    BootPhase::StatusFailed(err)
                }
                Ok(status) => {
                    tracing::trace!("got status");
                    if !status.backend_initialized {
                        // Backend not already initialized, need to manually
                        // log in.
                        // tracing::trace!("showing login!");
                        BootPhase::Login
                    } else {
                        tracing::trace!("loading schema");
                        // Backend initialized, load schema.
                        match api.schema().await {
                            Ok(schema) => BootPhase::SchemaLoaded(schema),
                            Err(err) => BootPhase::SchemaFailed(err),
                        }
                    }
                }
            };
            boot2.on_phase(phase);
        });

        boot.on_phase(BootPhase::LoadingStatus(guard));

        div().child_signal(boot.status.clone().signal_ref(move |phase| {
            match phase {
                BootPhase::Init => span(),
                BootPhase::LoadingStatus(_) => {
                    // TODO: use DelayedSpinner
                    spinner()
                }
                BootPhase::StatusFailed(err) => error_msg(&err.to_string()),
                BootPhase::SchemaFailed(err) => error_msg(&err.to_string()),
                BootPhase::SchemaLoaded(_) => {
                    tracing::trace!("schema loaded");
                    super::root::root()
                }
                BootPhase::Login => {
                    let boot = boot.clone();
                    super::login::login(Box::new(move |schema| {
                        boot.on_phase(BootPhase::SchemaLoaded(schema));
                    }))
                }
            }
        }))
    }

    fn on_phase(&self, phase: BootPhase) {
        tracing::trace!("boot phase change");
        match phase {
            BootPhase::SchemaLoaded(ref schema) => {
                // Initialize registry.
                let mut reg = Registry::new(schema.clone());
                reg.register_plugin(semantic_ui_core::base::BasePlugin);
                semantic_ui_core::context::set_registry(reg);
                // Read current route.

                let current_path = web_sys::window().unwrap().location().pathname().unwrap();
                let route = Route::from_path(&&current_path).unwrap_or(Route::Browse);
                // TODO: initialze url path listener.
                context::router().goto(route);

                self.status.set(phase);
            }
            _ => {
                self.status.set(phase);
            }
        }
    }
}
