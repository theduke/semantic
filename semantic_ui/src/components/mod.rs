pub mod entity;
mod loader;

pub mod root;
mod router;

mod import;

pub trait ContextExt {
    fn router(&self) -> &router::Router;
    fn registry(&self) -> &semantic_ui_core::SharedRegistry;
}

impl<'a, M> ContextExt for brass::Context<'a, M> {
    fn router(&self) -> &router::Router {
        self.get().expect("Router not in global context")
    }

    fn registry(&self) -> &semantic_ui_core::SharedRegistry {
        self.get().expect("Registry not in global context")
    }
}

pub trait RenderContextExt {
    fn router(&self) -> &router::Router;
    fn registry(&self) -> &semantic_ui_core::SharedRegistry;
}

impl<'a, C: brass::Component> RenderContextExt for brass::RenderContext<'a, C> {
    fn router(&self) -> &router::Router {
        self.get().expect("Router not in global context")
    }

    fn registry(&self) -> &semantic_ui_core::SharedRegistry {
        self.get().expect("Registry not in global context")
    }
}
