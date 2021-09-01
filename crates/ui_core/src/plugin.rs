use std::pin::Pin;

use factordb::{query::select::ItemPage, AnyError};

use crate::Registry;

pub struct BrowserPluginSpec {
    pub name: String,
}

pub trait BrowserPlugin {
    /// Allows initializing the context of the UI.
    /// The primary use case here is registering global context for the UI.
    // Silence warning for unused `ctx` arg because it would mess up IDE
    // code generation.
    #[allow(unused_variables)]
    fn init_ui_context(&self, ctx: &brass::Context<()>) {}

    fn spec(&self) -> BrowserPluginSpec;

    fn register(&self, registry: &mut Registry);

    fn can_import_url(&self, url: &str) -> bool;

    fn import(
        &self,
        url: url::Url,
        api: &crate::api::BrowserApiClient,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ItemPage, AnyError>> + 'static>>;
}
