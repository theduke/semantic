mod catalog;
pub(crate) mod defaults;
mod entity_actions;
mod entity_navigation;
mod error;
mod lookup;
mod media;
mod menu;
mod notes;
mod provider;
mod render_registry;
mod renderer;

pub use catalog::{UiCatalog, UiCatalogBuilder, UiCatalogConfig};
pub use entity_actions::{EntityActionContext, EntityActionPlacement, EntityActionRegistration};
pub use entity_navigation::{
    EntityHrefBuilder, EntityLinkRenderer, EntityNavigation, EntityOpenHandler, EntityTarget,
};
pub use error::UiCatalogError;
pub use media::{
    MediaHandle, MediaKind, MediaRenderEvent, MediaRenderOptions, MediaRendererRegistration,
};
pub use menu::{ActionPlacement, MenuSection, UiAction};
pub use provider::{
    CatalogLoadStatus, UiCatalogContext, UiCatalogProvider, UiCatalogReload, load_catalog,
    use_ui_catalog, use_ui_catalog_context, use_ui_catalog_reload,
};
pub use render_registry::{RenderRegistry, type_kind_key};
pub use renderer::{
    AttributeRenderer, ClassRenderContext, ClassRenderer, RenderCtx, RenderMode, RenderSettings,
    ValueRenderContext, ValueRenderer,
};
