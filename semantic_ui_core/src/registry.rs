use std::{collections::HashMap, rc::Rc};

use factordb::schema::{AttributeSchema, EntityAttribute, EntitySchema};

use crate::BrowserPlugin;

pub struct Registry {
    schema: semantics_core::api::SemanticSchema,

    attributes: HashMap<String, AttributeSchema>,
    entities: HashMap<String, EntityInfo>,

    plugins: Vec<Box<dyn BrowserPlugin>>,

    entity_renderers: Vec<EntityRendererSpec>,

    attribute_renderers: HashMap<String, DynAttrRenderer>,
    entity_content_renderers: HashMap<String, DynEntityRenderer>,
    entity_renderers_list: HashMap<String, DynEntityRenderer>,
    entity_renderers_page: HashMap<String, DynEntityRenderer>,
    entity_renderers_create: HashMap<String, DynEntityRenderer>,
    entity_renderers_create_page: HashMap<String, DynEntityRenderer>,
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

#[derive(Clone)]
pub struct EntityRenderOpts {
    pub editable: bool,
    pub preview: bool,
}
pub type DynEntityRenderer =
    Rc<dyn Fn(&factordb::query::select::Item, &EntityRenderOpts) -> brass::VNode>;

pub type DynAttrRenderer =
    Rc<dyn Fn(&factordb::data::Value, Option<&factordb::data::DataMap>) -> brass::VNode>;

#[derive(Clone, Debug)]
pub struct EntityInfo {
    pub schema: EntitySchema,
    pub fields: HashMap<String, EntityFieldAtrr>,
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

impl Registry {
    pub fn new(schema: semantics_core::api::SemanticSchema) -> Self {
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
            plugins: Vec::new(),
            entity_renderers: Vec::new(),

            entity_content_renderers: HashMap::new(),
            entity_renderers_list: HashMap::new(),
            entity_renderers_page: HashMap::new(),
            entity_renderers_create: HashMap::new(),
            entity_renderers_create_page: HashMap::new(),
            attribute_renderers: HashMap::new(),
        }
    }

    pub fn into_shared(self) -> SharedRegistry {
        SharedRegistry(Rc::new(self))
    }

    pub fn schema(&self) -> &semantics_core::api::SemanticSchema {
        &self.schema
    }

    pub fn register_plugin(&mut self, plugin: impl BrowserPlugin + 'static) {
        plugin.register(self);
        self.plugins.push(Box::new(plugin));
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

    pub fn attr(&self, ty: &str) -> Option<&AttributeSchema> {
        self.attributes.get(ty)
    }

    pub fn entity(&self, ty: &str) -> Option<&EntityInfo> {
        self.entities.get(ty)
    }

    pub fn entity_by_ident(&self, ident: &factordb::Ident) -> Option<&EntityInfo> {
        match ident {
            factordb::Ident::Id(_id) => todo!(),
            factordb::Ident::Name(name) => self.entity(name),
        }
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

    pub fn find_importer(&self, url: &str) -> Option<&dyn BrowserPlugin> {
        self.plugins
            .iter()
            .find(|p| p.can_import_url(url))
            .map(|p| p.as_ref())
    }
}
