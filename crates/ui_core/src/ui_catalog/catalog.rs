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
    /// Catalog without any schema or default renderers, for unit tests.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self::builder(CatalogStorageSnapshot {
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
        })
        .with_config(UiCatalogConfig {
            register_default_renderers: false,
            register_default_form_renderers: false,
        })
        .build()
    }

    pub fn builder(snapshot: CatalogStorageSnapshot) -> UiCatalogBuilder {
        UiCatalogBuilder::new(snapshot)
    }

    pub fn from_snapshot(snapshot: CatalogStorageSnapshot) -> Self {
        Self::builder(snapshot).build()
    }

    pub fn snapshot(&self) -> &CatalogStorageSnapshot {
        &self.inner.snapshot
    }

    /// Build a read-only rendering catalog without changing the local catalog.
    pub fn with_virtual_schema(
        &self,
        collection: &str,
        schema: &semantic_data::vdb::DatabaseSchema,
    ) -> Result<Self, String> {
        use semantic_db_core::catalog::Catalog;
        let local = Catalog::from_storage_snapshot(self.snapshot().clone())
            .map_err(|error| error.to_string())?;
        let ddl = schema.to_ddl_batch();
        let overlay = semantic_db_core::virtual_overlay(&local, &[(collection, &ddl)])?;
        let rebuilt = Self::from_snapshot(overlay.to_storage_snapshot());
        let mut catalog = self.clone();
        let inner = Rc::make_mut(&mut catalog.inner);
        inner.snapshot = rebuilt.inner.snapshot.clone();
        inner.attributes_by_id = rebuilt.inner.attributes_by_id.clone();
        inner.attributes_by_name = rebuilt.inner.attributes_by_name.clone();
        inner.classes_by_id = rebuilt.inner.classes_by_id.clone();
        inner.classes_by_name = rebuilt.inner.classes_by_name.clone();
        inner.collections_by_name = rebuilt.inner.collections_by_name.clone();
        inner.entity_actions.clear();
        inner.entity_navigation = EntityNavigation::default();
        inner.render_settings.enable_label_editor = false;
        Ok(catalog)
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

    #[test]
    fn creators_exclude_only_explicitly_disabled_classes() {
        let mut catalog = UiCatalog::from_snapshot(empty_snapshot());
        for (id, flag) in [
            ("default", None),
            ("allowed", Some(true)),
            ("hidden", Some(false)),
        ] {
            let mut class = semantic_data::filestore::file_class();
            class.id = id.to_string();
            class.creatable_in_ui = flag;
            Rc::make_mut(&mut catalog.inner)
                .classes_by_id
                .insert(class.id.clone(), class);
        }
        assert_eq!(
            catalog
                .creatable_classes()
                .map(|class| class.id.as_str())
                .collect::<Vec<_>>(),
            vec!["allowed", "default"]
        );
        assert_eq!(catalog.classes().count(), 3);
    }

    #[test]
    fn listings_exclude_only_explicitly_disabled_classes() {
        let mut catalog = UiCatalog::from_snapshot(empty_snapshot());
        for (id, flag) in [
            ("default", None),
            ("allowed", Some(true)),
            ("hidden", Some(false)),
        ] {
            let mut class = semantic_data::filestore::file_class();
            class.id = id.to_string();
            class.include_in_ui_listings = flag;
            Rc::make_mut(&mut catalog.inner)
                .classes_by_id
                .insert(class.id.clone(), class);
        }
        assert_eq!(
            catalog
                .unlisted_classes()
                .map(|class| class.id.as_str())
                .collect::<Vec<_>>(),
            vec!["hidden"]
        );
    }

    #[test]
    fn listing_exclusions_include_transitive_relation_subclasses() {
        let mut catalog = UiCatalog::from_snapshot(empty_snapshot());
        for (id, parent, flag) in [
            (semantic_data::attr::RELATION_CLASS_ID, None, None),
            (
                "example:link",
                Some(semantic_data::attr::RELATION_CLASS_ID),
                None,
            ),
            ("example:nested_link", Some("example:link"), Some(true)),
            ("example:article", None, None),
            ("example:internal", None, Some(false)),
        ] {
            let mut class = semantic_data::filestore::file_class();
            class.id = id.to_string();
            class.name = id.rsplit(':').next().unwrap().to_string();
            class.inherits =
                parent.map(|id| semantic_data::schema::ClassRef { id: id.to_string() });
            class.include_in_ui_listings = flag;
            Rc::make_mut(&mut catalog.inner)
                .classes_by_name
                .insert(class.name.clone(), class.clone());
            Rc::make_mut(&mut catalog.inner)
                .classes_by_id
                .insert(class.id.clone(), class);
        }
        let excluded = catalog.listing_excluded_type_values(true);
        for value in [
            semantic_data::attr::RELATION_CLASS_ID,
            "example:link",
            "example:nested_link",
            "link",
            "nested_link",
            "example:internal",
        ] {
            assert!(excluded.iter().any(|id| id == value), "missing {value}");
        }
        assert!(!excluded.iter().any(|id| id.contains("article")));
        assert_eq!(
            catalog.listing_excluded_type_values(false),
            vec!["example:internal", "internal"]
        );
    }

    #[test]
    fn virtual_rendering_schema_is_isolated_and_read_only() {
        use semantic_data::schema::{
            AttributeRef, AttributeType, ClassAttribute, Meta, StringType, Type, TypeDef, TypeKind,
        };
        let local = semantic_db_core::catalog::Catalog::new();
        let mut catalog = UiCatalog::from_snapshot(local.to_storage_snapshot());
        catalog.render_settings_mut().file_api_prefix = "/custom/files".into();
        let mut class = semantic_data::filestore::file_class();
        class.id = "virtual:Item".into();
        class.name = "VirtualItem".into();
        class.inherits = None;
        class.extends.clear();
        class.constraints.clear();
        class.attributes = std::collections::BTreeMap::from([(
            "virtual:title".into(),
            ClassAttribute {
                attribute: AttributeRef {
                    id: "virtual:title".into(),
                },
                required: true,
                ui_order: None,
                computed: None,
                default: None,
                constraints: vec![],
                meta: Meta::default(),
            },
        )]);
        class.meta.title = Some("Virtual item".into());
        let string = Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }));
        let schema = semantic_data::vdb::DatabaseSchema {
            types: vec![TypeDef {
                name: "virtual:Text".into(),
                module: None,
                params: vec![],
                visibility: semantic_data::schema::Visibility::Public,
                ty: string.clone(),
                meta: Meta::default(),
            }],
            attributes: vec![AttributeType {
                id: "virtual:title".into(),
                name: "VirtualTitle".into(),
                ty: string,
                constraints: vec![],
                meta: Meta {
                    title: Some("Virtual title".into()),
                    ..Meta::default()
                },
            }],
            classes: vec![class],
            relationships: vec![],
        };
        let before = catalog.snapshot().clone();
        let virtual_catalog = catalog.with_virtual_schema("fx", &schema).unwrap();
        assert_eq!(catalog.snapshot(), &before);
        assert!(catalog.attribute_by_id("virtual:title").is_none());
        assert_eq!(
            virtual_catalog.attribute_title("virtual:title"),
            "Virtual title"
        );
        assert_eq!(
            virtual_catalog
                .class_by_id("virtual:Item")
                .unwrap()
                .meta
                .title
                .as_deref(),
            Some("Virtual item")
        );
        assert!(
            virtual_catalog
                .snapshot()
                .type_defs
                .iter()
                .any(|ty| ty.type_def.name == "virtual:Text")
        );
        assert_eq!(
            virtual_catalog.render_settings().file_api_prefix,
            "/custom/files"
        );
        assert!(catalog.render_settings().enable_label_editor);
        assert!(!virtual_catalog.render_settings().enable_label_editor);
        assert!(virtual_catalog.entity_actions().is_empty());
        assert!(virtual_catalog.entity_navigation().href.is_none());
        assert!(
            virtual_catalog
                .with_virtual_schema("other", &schema)
                .is_err()
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
