use std::collections::BTreeMap;

use semantic_data::schema::{AttributeType, ClassType};
use semantic_db_core::catalog::{CatalogStorageSnapshot, StoredCollection};

use crate::ui_catalog::{
    MediaRendererRegistration, MenuSection, RenderRegistry, defaults::register_defaults,
};

#[derive(Clone, Default)]
pub struct UiCatalogConfig {
    pub register_default_renderers: bool,
}

#[derive(Clone)]
pub struct UiCatalog {
    snapshot: CatalogStorageSnapshot,
    attributes_by_id: BTreeMap<String, AttributeType>,
    attributes_by_name: BTreeMap<String, AttributeType>,
    classes_by_id: BTreeMap<String, ClassType>,
    classes_by_name: BTreeMap<String, ClassType>,
    collections_by_name: BTreeMap<String, StoredCollection>,
    render_registry: RenderRegistry,
    media_renderers: Vec<MediaRendererRegistration>,
    menu_sections: Vec<MenuSection>,
}

impl UiCatalog {
    pub fn builder(snapshot: CatalogStorageSnapshot) -> UiCatalogBuilder {
        UiCatalogBuilder::new(snapshot)
    }

    pub fn from_snapshot(snapshot: CatalogStorageSnapshot) -> Self {
        Self::builder(snapshot).build()
    }

    pub fn snapshot(&self) -> &CatalogStorageSnapshot {
        &self.snapshot
    }

    pub fn render_registry(&self) -> &RenderRegistry {
        &self.render_registry
    }

    pub fn render_registry_mut(&mut self) -> &mut RenderRegistry {
        &mut self.render_registry
    }

    pub fn media_renderers(&self) -> &[MediaRendererRegistration] {
        &self.media_renderers
    }

    pub fn menu_sections(&self) -> &[MenuSection] {
        &self.menu_sections
    }

    pub fn register_media_renderer(&mut self, renderer: MediaRendererRegistration) {
        self.media_renderers.push(renderer);
    }

    pub fn register_menu_section(&mut self, section: MenuSection) {
        self.menu_sections.push(section);
    }

    pub(crate) fn attributes_by_id(&self) -> &BTreeMap<String, AttributeType> {
        &self.attributes_by_id
    }

    pub(crate) fn attributes_by_name(&self) -> &BTreeMap<String, AttributeType> {
        &self.attributes_by_name
    }

    pub(crate) fn classes_by_id(&self) -> &BTreeMap<String, ClassType> {
        &self.classes_by_id
    }

    pub(crate) fn classes_by_name(&self) -> &BTreeMap<String, ClassType> {
        &self.classes_by_name
    }

    pub(crate) fn collections_by_name(&self) -> &BTreeMap<String, StoredCollection> {
        &self.collections_by_name
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
                snapshot,
                attributes_by_id,
                attributes_by_name,
                classes_by_id,
                classes_by_name,
                collections_by_name,
                render_registry: RenderRegistry::default(),
                media_renderers: Vec::new(),
                menu_sections: Vec::new(),
            },
            config: UiCatalogConfig {
                register_default_renderers: true,
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
        self.catalog
    }
}
