use crate::routing;

pub trait ContextExt {
    fn router(&self) -> &routing::Router;
    fn registry(&self) -> &crate::SharedRegistry;
    fn api(&self) -> &crate::api::BrowserApiClient;
}

impl<'a, M> ContextExt for brass::Context<'a, M> {
    fn router(&self) -> &routing::Router {
        self.get().expect("Router not in global context")
    }

    fn registry(&self) -> &crate::SharedRegistry {
        self.get().expect("Registry not in global context")
    }

    fn api(&self) -> &crate::api::BrowserApiClient {
        self.get().expect("API not in global context")
    }
}

pub trait RenderContextExt {
    fn router(&self) -> &routing::Router;
    fn registry(&self) -> &crate::SharedRegistry;
}

impl<'a, C: brass::Component> RenderContextExt for brass::RenderContext<'a, C> {
    fn router(&self) -> &routing::Router {
        self.get().expect("Router not in global context")
    }

    fn registry(&self) -> &crate::SharedRegistry {
        self.get().expect("Registry not in global context")
    }
}
