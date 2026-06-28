use std::collections::BTreeMap;

use semantic_data::schema::{AttributeType, ClassType};
use semantic_db_core::catalog::{CatalogStorageSnapshot, StoredCollection};

use crate::{
    form::{UiFormRegistry, register_default_form_renderers},
    ui_catalog::{
        EntityActionRegistration, EntityHrefBuilder, EntityLinkRenderer, EntityNavigation,
        EntityOpenHandler, MediaRendererRegistration, MenuSection, RenderRegistry, RenderSettings,
        defaults::register_defaults,
    },
};

#[derive(Clone, Default)]
pub struct UiCatalogConfig {
    pub register_default_renderers: bool,
    pub register_default_form_renderers: bool,
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
    render_settings: RenderSettings,
    form_registry: UiFormRegistry,
    media_renderers: Vec<MediaRendererRegistration>,
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
        &self.snapshot
    }

    pub fn render_registry(&self) -> &RenderRegistry {
        &self.render_registry
    }

    pub fn render_registry_mut(&mut self) -> &mut RenderRegistry {
        &mut self.render_registry
    }

    pub fn render_settings(&self) -> &RenderSettings {
        &self.render_settings
    }

    pub fn render_settings_mut(&mut self) -> &mut RenderSettings {
        &mut self.render_settings
    }

    pub fn form_registry(&self) -> &UiFormRegistry {
        &self.form_registry
    }

    pub fn form_registry_mut(&mut self) -> &mut UiFormRegistry {
        &mut self.form_registry
    }

    pub fn media_renderers(&self) -> &[MediaRendererRegistration] {
        &self.media_renderers
    }

    pub fn menu_sections(&self) -> &[MenuSection] {
        &self.menu_sections
    }

    pub fn entity_navigation(&self) -> &EntityNavigation {
        &self.entity_navigation
    }

    pub fn entity_navigation_mut(&mut self) -> &mut EntityNavigation {
        &mut self.entity_navigation
    }

    pub fn entity_actions(&self) -> &[EntityActionRegistration] {
        &self.entity_actions
    }

    pub(crate) fn entity_actions_mut(&mut self) -> &mut Vec<EntityActionRegistration> {
        &mut self.entity_actions
    }

    pub fn set_entity_href_builder(&mut self, builder: EntityHrefBuilder) {
        self.entity_navigation.href = Some(builder);
    }

    pub fn set_entity_open_handler(&mut self, handler: EntityOpenHandler) {
        self.entity_navigation.open = Some(handler);
    }

    pub fn set_entity_link_renderer(&mut self, renderer: EntityLinkRenderer) {
        self.entity_navigation.link_renderer = Some(renderer);
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
                render_settings: RenderSettings::default(),
                form_registry: UiFormRegistry::default(),
                media_renderers: Vec::new(),
                menu_sections: Vec::new(),
                entity_navigation: EntityNavigation::default(),
                entity_actions: Vec::new(),
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
