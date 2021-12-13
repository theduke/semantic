use std::{rc::Rc, sync::Arc};

use brass::{
    dom::{
        builder::{div, span},
        View,
    },
    effect::{spawn_guarded, EffectGuard},
    signal::signal::Mutable,
};
use factordb::AnyError;
use semantic_core::api::SemanticSchema;
use semantic_ui_core::{
    components::loader::{error_msg, spinner},
    context,
    plugin::DynBrowserPlugin,
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
    pub fn render() -> View {
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

        div()
            .style_raw("height: 100%;")
            .signal(boot.status.clone().signal_ref(move |phase| {
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
            .into()
    }

    fn on_phase(&self, phase: BootPhase) {
        tracing::trace!("boot phase change");
        match phase {
            BootPhase::SchemaLoaded(ref schema) => {
                let router = context::router();

                // Initialize registry.
                let mut reg = Registry::new(schema.clone());

                // TODO: move this code somewhere more sensible. (registry?)
                let plugins: Vec<DynBrowserPlugin> = vec![
                    Arc::new(semantic_ui_core::base::BasePlugin),
                    Arc::new(semantic_extra::health::HealthPlugin),
                    Arc::new(semantic_extra::habits::HabitsPlugin),
                ];

                for plugin in plugins {
                    reg.register_plugin(plugin.clone());

                    if let Some(plugin_router) = plugin.router() {
                        router.register_router(plugin_router);
                    }
                }

                semantic_ui_core::context::set_registry(reg);
                // Read current route.

                router.subscribe_to_history();
                router.on_location_changed();

                self.status.set(phase);
            }
            _ => {
                self.status.set(phase);
            }
        }
    }
}
