use std::sync::Arc;

use crate::{
    routing::{DynPluginRouter, PluginRoute},
    Registry,
};

pub struct PluginMainRoute {
    pub name: String,
    pub route: PluginRoute,
}

pub struct BrowserPluginSpec {
    pub name: String,
    pub main_route: Option<PluginMainRoute>,
}

pub type DynBrowserPlugin = Arc<dyn BrowserPlugin>;

impl BrowserPlugin for DynBrowserPlugin {
    fn spec(&self) -> BrowserPluginSpec {
        self.as_ref().spec()
    }

    fn register(&self, registry: &mut Registry) {
        self.as_ref().register(registry)
    }
}

pub trait BrowserPlugin: Sync + Send {
    // /// Allows initializing the context of the UI.
    // /// The primary use case here is registering global context for the UI.
    // // Silence warning for unused `ctx` arg because it would mess up IDE
    // // code generation.
    // #[allow(unused_variables)]
    // fn init_ui_context(&self, ctx: &brass::Context<()>) {}

    fn spec(&self) -> BrowserPluginSpec;

    fn register(&self, registry: &mut Registry);

    fn router(&self) -> Option<DynPluginRouter> {
        None
    }

    // #[allow(unused_variables)]
    // fn import_match(&self, url: &url::Url) -> Option<UrlSupport> {
    //     None
    // }

    // #[allow(unused_variables)]
    // fn import(
    //     &self,
    //     url: url::Url,
    //     api: &crate::api::BrowserApiClient,
    // ) -> std::pin::Pin<
    //     Box<dyn std::future::Future<Output = Result<Option<FetchUrlOutput>, anyhow::Error>> + 'static>,
    // > {
    //     Box::pin(futures::future::ready(Ok(None)))
    // }
}
