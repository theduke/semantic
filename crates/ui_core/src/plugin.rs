use factordb::AnyError;
use semantic_core::plugin::{ImportOutput, ImportSupport};

use crate::Registry;

pub struct BrowserPluginSpec {
    pub name: String,
}

pub trait BrowserPlugin: Sync + Send {
    /// Allows initializing the context of the UI.
    /// The primary use case here is registering global context for the UI.
    // Silence warning for unused `ctx` arg because it would mess up IDE
    // code generation.
    #[allow(unused_variables)]
    fn init_ui_context(&self, ctx: &brass::Context<()>) {}

    fn spec(&self) -> BrowserPluginSpec;

    fn register(&self, registry: &mut Registry);

    #[allow(unused_variables)]
    fn import_match(&self, url: &url::Url) -> Option<ImportSupport> {
        None
    }

    #[allow(unused_variables)]
    fn import(
        &self,
        url: url::Url,
        api: &crate::api::BrowserApiClient,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<ImportOutput>, AnyError>> + 'static>,
    > {
        Box::pin(futures::future::ready(Ok(None)))
    }
}
