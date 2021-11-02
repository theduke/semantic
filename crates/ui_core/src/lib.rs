pub mod api;
pub mod context;
pub mod plugin;
pub mod registry;
pub mod routing;

pub mod base;

pub mod validate;

pub mod components;

pub type SharedRenderer0 = std::rc::Rc<dyn Fn() -> TagBuilder>;


use brass::dom::TagBuilder;

pub use self::{
    plugin::{BrowserPlugin, BrowserPluginSpec},
    registry::{
        DynEntityRenderer, EntityInfo, EntityRenderMode, EntityRenderOpts, EntityRendererSpec,
        Registry, SharedRegistry,
    },
};
