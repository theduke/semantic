use std::{rc::Rc, sync::RwLock};

pub struct Context {
    registry: RwLock<crate::SharedRegistry>,
    api: crate::api::BrowserApiClient,
    router: crate::routing::Router,
}

thread_local!(
static CONTEXT: Rc<Context> = Rc::new(Context {
    registry: RwLock::new(crate::Registry::new(Default::default()).into_shared()),
    api: crate::api::new_api(None),
    router: crate::routing::Router::new(),
}));

pub fn api() -> crate::api::BrowserApiClient {
    CONTEXT.with(|c| c.api.clone())
}

pub fn registry() -> crate::SharedRegistry {
    // FIXME: drop unsafe
    CONTEXT.with(|c| c.registry.read().unwrap().clone())
}

pub fn set_registry(registry: crate::Registry) {
    CONTEXT.with(|c| {
        *c.registry.write().unwrap() = registry.into_shared();
    })
}

pub fn router() -> crate::routing::Router {
    CONTEXT.with(|c| c.router.clone())
}
