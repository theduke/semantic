use std::{collections::HashMap, rc::Rc};

use factordb::{
    prelude::IdOrIdent,
    schema::{AttrMapExt, AttributeSchema, EntityAttribute, EntityDescriptor, EntitySchema},
    AnyError,
};
use fnv::{FnvHashMap, FnvHashSet};
use futures::future::LocalBoxFuture;

use crate::{
    plugin::{DynBrowserPlugin, PluginMainRoute},
    BrowserPlugin,
};

pub struct Registry {
    schema: semantic_core::api::SemanticSchema,

    attributes: FnvHashMap<String, AttributeSchema>,
    entities: FnvHashMap<String, EntityInfo>,

    plugins: HashMap<String, DynBrowserPlugin>,

    entity_renderers: Vec<EntityRendererSpec>,

    attribute_renderers: FnvHashMap<String, DynAttrRenderer>,
    entity_content_renderers: FnvHashMap<String, DynEntityRenderer>,
    entity_renderers_list: FnvHashMap<String, DynEntityRenderer>,
    entity_renderers_page: FnvHashMap<String, DynEntityRenderer>,
    entity_renderers_create: FnvHashMap<String, DynEntityRenderer>,
    entity_renderers_create_page: FnvHashMap<String, DynEntityRenderer>,
    entity_renderer_media: FnvHashMap<String, RegisteredMediaRenderer>,

    ignored_entity_types: FnvHashSet<String>,
}

impl Registry {
    pub fn new(schema: semantic_core::api::SemanticSchema) -> Self {
        let entities = schema
            .db
            .entities
            .clone()
            .into_iter()
            .map(|entity| {
                let fields = entity
                    .attributes
                    .clone()
                    .into_iter()
                    .filter_map(|field| {
                        let attr = schema.db.resolve_attr(&field.attribute)?.clone();
                        Some((attr.ident.clone(), EntityFieldAtrr { field, attr }))
                    })
                    .collect();

                (
                    entity.ident.clone(),
                    EntityInfo {
                        fields,
                        schema: entity,
                    },
                )
            })
            .collect();

        let attributes = schema
            .db
            .attributes
            .clone()
            .into_iter()
            .map(|attr| (attr.ident.clone(), attr))
            .collect();

        Self {
            schema,
            entities,
            attributes,
            plugins: HashMap::new(),
            entity_renderers: Vec::new(),

            entity_content_renderers: FnvHashMap::default(),
            entity_renderers_list: FnvHashMap::default(),
            entity_renderers_page: FnvHashMap::default(),
            entity_renderers_create: FnvHashMap::default(),
            entity_renderers_create_page: FnvHashMap::default(),
            entity_renderer_media: FnvHashMap::default(),
            attribute_renderers: FnvHashMap::default(),

            ignored_entity_types: FnvHashSet::from_iter(vec![
                // Ignore the factor builtin base types.
                factordb::schema::builtin::EntitySchemaType::QUALIFIED_NAME.to_string(),
                factordb::schema::builtin::AttributeSchemaType::QUALIFIED_NAME.to_string(),
                factordb::schema::builtin::IndexSchemaType::QUALIFIED_NAME.to_string(),
            ]),
        }
    }

    pub fn into_shared(self) -> SharedRegistry {
        SharedRegistry(Rc::new(self))
    }

    pub fn schema(&self) -> &semantic_core::api::SemanticSchema {
        &self.schema
    }

    pub fn register_plugin(&mut self, plugin: DynBrowserPlugin) {
        plugin.register(self);
        self.plugins.insert(plugin.spec().name, plugin);
    }

    pub fn plugin_main_routes(&self) -> Vec<PluginMainRoute> {
        self.plugins
            .values()
            .filter_map(|p| p.spec().main_route)
            .collect()
    }

    pub fn register_attr_renderer(&mut self, ty: String, renderer: DynAttrRenderer) {
        self.attribute_renderers.insert(ty, renderer);
    }

    pub fn register_entity_renderer(&mut self, spec: EntityRendererSpec) {
        match spec.mode {
            EntityRenderMode::Content => {
                self.entity_content_renderers
                    .insert(spec.entity_type.clone(), spec.renderer.clone());
            }
            EntityRenderMode::View => {
                self.entity_renderers_list
                    .insert(spec.entity_type.clone(), spec.renderer.clone());
            }
            EntityRenderMode::ViewPage => {
                self.entity_renderers_page
                    .insert(spec.entity_type.clone(), spec.renderer.clone());
            }
            EntityRenderMode::Create => {
                self.entity_renderers_create
                    .insert(spec.entity_type.clone(), spec.renderer.clone());
            }
            EntityRenderMode::CreatePage => {
                self.entity_renderers_create_page
                    .insert(spec.entity_type.clone(), spec.renderer.clone());
            }
        }

        self.entity_renderers.push(spec);
    }

    pub fn register_media_renderer(&mut self, render: RegisteredMediaRenderer) {
        self.entity_renderer_media
            .insert(render.entity_type.clone(), render);
    }

    pub fn get_media_renderer(&self, entity_type: &str) -> Option<&RegisteredMediaRenderer> {
        self.entity_renderer_media.get(entity_type)
    }

    pub fn attr(&self, ty: &str) -> Option<&AttributeSchema> {
        self.attributes.get(ty)
    }

    /// Get a reference to the registries entities.
    pub fn entities(&self) -> &FnvHashMap<String, EntityInfo> {
        &self.entities
    }

    pub fn entities_without_ignored(&self) -> impl Iterator<Item = &EntityInfo> {
        self.entities
            .values()
            .filter(|e| !self.ignored_entity_types.contains(&e.schema.ident))
    }

    pub fn entity(&self, ty: &str) -> Option<&EntityInfo> {
        self.entities.get(ty)
    }

    pub fn entity_by_ident(&self, ident: &IdOrIdent) -> Option<&EntityInfo> {
        match ident {
            IdOrIdent::Id(_id) => todo!(),
            IdOrIdent::Name(name) => self.entity(name),
        }
    }

    /// Convenience helper to get the `[EntityInfo]` for some data.
    /// Returns [`None`] if the data does not have a "factor/type" attribute or
    /// if no info is registered.
    pub fn entity_by_data(&self, data: &factordb::data::DataMap) -> Option<&EntityInfo> {
        data.get_type().and_then(|ty| self.entity_by_ident(&ty))
    }

    /// Convenience helper to get the `[EntityInfo]` for an [`Item`].
    /// Returns [`None`] if the data does not have a "factor/type" attribute or
    /// if no info is registered.
    pub fn entity_by_item(&self, item: &factordb::query::select::Item) -> Option<&EntityInfo> {
        item.data
            .get_type()
            .and_then(|ty| self.entity_by_ident(&ty))
    }

    pub fn attr_renderer(&self, ty: &str) -> Option<&DynAttrRenderer> {
        self.attribute_renderers.get(ty)
    }

    pub fn entity_content_renderer(&self, ty: &str) -> Option<&DynEntityRenderer> {
        self.entity_content_renderers.get(ty)
    }

    pub fn entity_item_renderer(&self, ty: &str) -> Option<&DynEntityRenderer> {
        self.entity_renderers_list.get(ty)
    }

    pub fn entity_page_renderer(&self, ty: &str) -> Option<&DynEntityRenderer> {
        self.entity_renderers_page.get(ty)
    }

    pub fn entity_create_renderer(&self, ty: &str) -> Option<&DynEntityRenderer> {
        self.entity_renderers_create.get(ty)
    }

    pub fn entity_create_page_renderer(&self, ty: &str) -> Option<&DynEntityRenderer> {
        self.entity_renderers_create_page.get(ty)
    }

    pub fn creatable_entities(&self) -> Vec<&EntityInfo> {
        self.entity_renderers_create_page
            .keys()
            .filter_map(|key| self.entities.get(key))
            .collect()
    }

    /// Set the given entity type as ignored.
    ///
    /// This will exclude the entity type from list queries, type filters, ...
    pub fn ignore_entity_type(&mut self, ty: String) {
        self.ignored_entity_types.insert(ty);
    }

    pub fn ignored_entity_types(&self) -> &FnvHashSet<String> {
        &self.ignored_entity_types
    }
}

#[derive(Clone)]
pub struct SharedRegistry(Rc<Registry>);

impl std::ops::Deref for SharedRegistry {
    type Target = Registry;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct EntityFieldAtrr {
    pub field: EntityAttribute,
    pub attr: AttributeSchema,
}

// pub type DynRenderer<T> = Box<dyn Fn(&T) -> brass::VNode>;

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct EntityRenderOpts {
    pub editable: bool,
    pub preview: bool,
}

pub type DynEntityRenderer =
    Rc<dyn Fn(&factordb::query::select::Item, &EntityRenderOpts) -> brass::dom::TagBuilder>;

pub enum MediaRenderEvent {
    Finished(Result<(), AnyError>),
    Paused,
    Resumed,
}

pub struct MediaRenderOpts {
    // Settings.
    /// If true, the media should be playing.
    /// This also means it should auto-play on first render.
    /// If false, playback should be paused.
    pub playing: bool,
    /// If true, all audio output should be muted.
    pub muted: bool,
    // Callbacks.
    /// Callback that is to be invoked when the media item has stopped playing.
    /// An `Ok(())` is expected if the playback finished correctly.
    /// An `Err(_)` is expected if the playback failed, for example if a video
    /// could not be loaded.
    pub callback: Rc<dyn Fn(MediaRenderEvent)>,
}

pub type DynMediaRenderer = Rc<
    dyn Fn(
        &factordb::query::select::Item,
        &MediaRenderOpts,
    ) -> (brass::dom::TagBuilder, Option<DynMediaHandle>),
>;

#[derive(Clone)]
pub struct RegisteredMediaRenderer {
    pub entity_type: String,
    pub render: DynMediaRenderer,
    /// If true, the given media item can be played, like video or audio.
    /// If false, it is static, like an image.
    pub supports_playback: bool,
}

pub type DynAttrRenderer =
    Rc<dyn Fn(&factordb::data::Value, Option<&factordb::data::DataMap>) -> brass::dom::TagBuilder>;

#[derive(Clone, Debug)]
pub struct EntityInfo {
    pub schema: EntitySchema,
    pub fields: FnvHashMap<String, EntityFieldAtrr>,
}

#[derive(Clone, Copy, Debug)]
pub enum EntityRenderMode {
    Content,
    View,
    ViewPage,
    Create,
    CreatePage,
}

pub struct EntityRendererSpec {
    pub name: String,
    pub entity_type: String,
    pub mode: EntityRenderMode,
    pub renderer: DynEntityRenderer,
    pub is_default: bool,
}

pub type MediaFuture<T> = LocalBoxFuture<'static, Result<T, AnyError>>;

pub trait MediaHandle {
    fn play(&self);
    fn pause(&self);
    fn set_muted(&self, muted: bool);

    fn on_remove(&self);
}

pub type DynMediaHandle = Rc<dyn MediaHandle>;
