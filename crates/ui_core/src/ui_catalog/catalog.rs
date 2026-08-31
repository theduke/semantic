use std::{collections::BTreeMap, rc::Rc};

use semantic_data::schema::{AttributeType, ClassType};
use semantic_db_core::catalog::{CatalogStorageSnapshot, StoredCollection};

use crate::{
    form::{UiFormRegistry, register_default_form_renderers},
    ui_catalog::{
        EntityActionRegistration, EntityHrefBuilder, EntityLinkRenderer, EntityNavigation,
        EntityOpenHandler, MediaPlaybackRendererRegistration, MediaRendererRegistration,
        MenuSection, RenderRegistry, RenderSettings, defaults::register_defaults,
    },
};

#[derive(Clone, Default)]
pub struct UiCatalogConfig {
    pub register_default_renderers: bool,
    pub register_default_form_renderers: bool,
}

#[derive(Clone)]
pub struct UiCatalog {
    inner: Rc<UiCatalogInner>,
}

#[derive(Clone)]
struct UiCatalogInner {
    snapshot: CatalogStorageSnapshot,
    attributes_by_id: BTreeMap<String, AttributeType>,
    attributes_by_name: BTreeMap<String, AttributeType>,
    classes_by_id: BTreeMap<String, ClassType>,
    classes_by_name: BTreeMap<String, ClassType>,
    collections_by_name: BTreeMap<String, StoredCollection>,
    render_registry: RenderRegistry,
    render_settings: RenderSettings,
    form_registry: UiFormRegistry,
    media_renderers: Vec<MediaRendererRegistration>,
    media_playback_renderers: Vec<MediaPlaybackRendererRegistration>,
    menu_sections: Vec<MenuSection>,
    entity_navigation: EntityNavigation,
    entity_actions: Vec<EntityActionRegistration>,
}

impl UiCatalog {
    pub fn builder(snapshot: CatalogStorageSnapshot) -> UiCatalogBuilder {
        UiCatalogBuilder::new(snapshot)
    }

    pub fn from_snapshot(snapshot: CatalogStorageSnapshot) -> Self {
        Self::builder(snapshot).build()
    }

    pub fn snapshot(&self) -> &CatalogStorageSnapshot {
        &self.inner.snapshot
    }

    pub fn render_registry(&self) -> &RenderRegistry {
        &self.inner.render_registry
    }

    pub fn render_registry_mut(&mut self) -> &mut RenderRegistry {
        &mut Rc::make_mut(&mut self.inner).render_registry
    }

    pub fn render_settings(&self) -> &RenderSettings {
        &self.inner.render_settings
    }

    pub fn render_settings_mut(&mut self) -> &mut RenderSettings {
        &mut Rc::make_mut(&mut self.inner).render_settings
    }

    pub fn form_registry(&self) -> &UiFormRegistry {
        &self.inner.form_registry
    }

    pub fn form_registry_mut(&mut self) -> &mut UiFormRegistry {
        &mut Rc::make_mut(&mut self.inner).form_registry
    }

    pub fn media_renderers(&self) -> &[MediaRendererRegistration] {
        &self.inner.media_renderers
    }

    pub fn media_playback_renderers(&self) -> &[MediaPlaybackRendererRegistration] {
        &self.inner.media_playback_renderers
    }

    pub fn menu_sections(&self) -> &[MenuSection] {
        &self.inner.menu_sections
    }

    pub fn entity_navigation(&self) -> &EntityNavigation {
        &self.inner.entity_navigation
    }

    pub fn entity_navigation_mut(&mut self) -> &mut EntityNavigation {
        &mut Rc::make_mut(&mut self.inner).entity_navigation
    }

    pub fn entity_actions(&self) -> &[EntityActionRegistration] {
        &self.inner.entity_actions
    }

    pub(crate) fn entity_actions_mut(&mut self) -> &mut Vec<EntityActionRegistration> {
        &mut Rc::make_mut(&mut self.inner).entity_actions
    }

    pub fn set_entity_href_builder(&mut self, builder: EntityHrefBuilder) {
        self.entity_navigation_mut().href = Some(builder);
    }

    pub fn set_entity_open_handler(&mut self, handler: EntityOpenHandler) {
        self.entity_navigation_mut().open = Some(handler);
    }

    pub fn set_entity_link_renderer(&mut self, renderer: EntityLinkRenderer) {
        self.entity_navigation_mut().link_renderer = Some(renderer);
    }

    pub fn register_media_renderer(&mut self, renderer: MediaRendererRegistration) {
        Rc::make_mut(&mut self.inner).media_renderers.push(renderer);
    }

    pub fn register_media_playback_renderer(
        &mut self,
        renderer: MediaPlaybackRendererRegistration,
    ) {
        Rc::make_mut(&mut self.inner)
            .media_playback_renderers
            .push(renderer);
    }

    pub fn register_menu_section(&mut self, section: MenuSection) {
        Rc::make_mut(&mut self.inner).menu_sections.push(section);
    }

    pub(crate) fn attributes_by_id(&self) -> &BTreeMap<String, AttributeType> {
        &self.inner.attributes_by_id
    }

    pub(crate) fn attributes_by_name(&self) -> &BTreeMap<String, AttributeType> {
        &self.inner.attributes_by_name
    }

    pub(crate) fn classes_by_id(&self) -> &BTreeMap<String, ClassType> {
        &self.inner.classes_by_id
    }

    pub(crate) fn classes_by_name(&self) -> &BTreeMap<String, ClassType> {
        &self.inner.classes_by_name
    }

    pub(crate) fn collections_by_name(&self) -> &BTreeMap<String, StoredCollection> {
        &self.inner.collections_by_name
    }
}

pub struct UiCatalogBuilder {
    catalog: UiCatalog,
    config: UiCatalogConfig,
}

impl UiCatalogBuilder {
    pub fn new(snapshot: CatalogStorageSnapshot) -> Self {
        let attributes_by_id = snapshot
            .attributes
            .iter()
            .map(|stored| (stored.attribute.id.clone(), stored.attribute.clone()))
            .collect();
        let attributes_by_name = snapshot
            .attributes
            .iter()
            .map(|stored| (stored.attribute.name.clone(), stored.attribute.clone()))
            .collect();
        let classes_by_id = snapshot
            .classes
            .iter()
            .map(|stored| (stored.class.id.clone(), stored.class.clone()))
            .collect();
        let classes_by_name = snapshot
            .classes
            .iter()
            .map(|stored| (stored.class.name.clone(), stored.class.clone()))
            .collect();
        let collections_by_name = snapshot
            .collections
            .iter()
            .map(|stored| (stored.name.clone(), stored.clone()))
            .collect();
        Self {
            catalog: UiCatalog {
                inner: Rc::new(UiCatalogInner {
                    snapshot,
                    attributes_by_id,
                    attributes_by_name,
                    classes_by_id,
                    classes_by_name,
                    collections_by_name,
                    render_registry: RenderRegistry::default(),
                    render_settings: RenderSettings::default(),
                    form_registry: UiFormRegistry::default(),
                    media_renderers: Vec::new(),
                    media_playback_renderers: Vec::new(),
                    menu_sections: Vec::new(),
                    entity_navigation: EntityNavigation::default(),
                    entity_actions: Vec::new(),
                }),
            },
            config: UiCatalogConfig {
                register_default_renderers: true,
                register_default_form_renderers: true,
            },
        }
    }

    pub fn with_config(mut self, config: UiCatalogConfig) -> Self {
        self.config = config;
        self
    }

    pub fn build(mut self) -> UiCatalog {
        if self.config.register_default_renderers {
            register_defaults(&mut self.catalog);
        }
        if self.config.register_default_form_renderers {
            register_default_form_renderers(&mut self.catalog);
        }
        self.catalog
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use semantic_db_core::catalog::CatalogStorageSnapshot;

    use super::UiCatalog;

    #[test]
    fn clones_share_storage_until_mutated() {
        let catalog = UiCatalog::from_snapshot(empty_snapshot());
        let mut configured = catalog.clone();

        assert!(Rc::ptr_eq(&catalog.inner, &configured.inner));

        configured.render_settings_mut().show_media = false;

        assert!(!Rc::ptr_eq(&catalog.inner, &configured.inner));
        assert!(catalog.render_settings().show_media);
        assert!(!configured.render_settings().show_media);
    }

    #[test]
    fn defaults_register_the_note_class_form_renderer() {
        let catalog = UiCatalog::from_snapshot(empty_snapshot());

        assert!(
            catalog
                .form_registry()
                .class_form_renderer("semantic:base:note")
                .is_some()
        );
    }

    #[test]
    fn defaults_register_the_file_class_renderer() {
        let catalog = UiCatalog::from_snapshot(empty_snapshot());

        assert!(
            catalog
                .render_registry()
                .class_renderer(semantic_data::filestore::FILE_CLASS_ID)
                .is_some()
        );
    }

    fn empty_snapshot() -> CatalogStorageSnapshot {
        CatalogStorageSnapshot {
            attributes: Vec::new(),
            type_defs: Vec::new(),
            record_types: Vec::new(),
            classes: Vec::new(),
            collections: Vec::new(),
            indexes: Vec::new(),
            relationships: Vec::new(),
            packages: Vec::new(),
            applied_migrations: Vec::new(),
            next_field_id: 0,
            auto_index_enabled: false,
        }
    }
}
