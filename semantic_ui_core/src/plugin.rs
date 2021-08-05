use std::pin::Pin;

use factordb::{query::select::ItemPage, AnyError};

use crate::Registry;

pub struct BrowserPluginSpec {
    pub name: String,
}

pub trait BrowserPlugin {
    fn spec(&self) -> BrowserPluginSpec;

    fn register(&self, registry: &mut Registry);

    fn can_import_url(&self, url: &str) -> bool;

    fn import(
        &self,
        url: url::Url,
        api: &crate::api::BrowserApiClient,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<ItemPage, AnyError>> + 'static>>;
}
