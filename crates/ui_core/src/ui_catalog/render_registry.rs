use std::collections::BTreeMap;

use semantic_data::schema::TypeKind;

use crate::ui_catalog::{AttributeRenderer, ClassRenderer, ValueRenderer};

#[derive(Clone, Default)]
pub struct RenderRegistry {
    class_renderers: BTreeMap<String, ClassRenderer>,
    attribute_renderers: BTreeMap<String, AttributeRenderer>,
    type_renderers: BTreeMap<&'static str, ValueRenderer>,
    fallback_renderer: Option<ValueRenderer>,
}

impl RenderRegistry {
    pub fn register_class_renderer(
        &mut self,
        class_id: impl Into<String>,
        renderer: ClassRenderer,
    ) {
        self.class_renderers.insert(class_id.into(), renderer);
    }

    pub fn register_attribute_renderer(
        &mut self,
        attribute_id: impl Into<String>,
        renderer: AttributeRenderer,
    ) {
        self.attribute_renderers
            .insert(attribute_id.into(), renderer);
    }

    pub fn register_type_renderer(&mut self, kind: &'static str, renderer: ValueRenderer) {
        self.type_renderers.insert(kind, renderer);
    }

    pub fn set_fallback_renderer(&mut self, renderer: ValueRenderer) {
        self.fallback_renderer = Some(renderer);
    }

    pub fn class_renderer(&self, class_id: &str) -> Option<ClassRenderer> {
        self.class_renderers.get(class_id).cloned()
    }

    pub fn attribute_renderer(&self, attribute_id: &str) -> Option<AttributeRenderer> {
        self.attribute_renderers.get(attribute_id).cloned()
    }

    pub fn type_renderer(&self, kind: &TypeKind) -> Option<ValueRenderer> {
        self.type_renderers.get(type_kind_key(kind)).cloned()
    }

    pub fn fallback_renderer(&self) -> Option<ValueRenderer> {
        self.fallback_renderer.clone()
    }
}

pub fn type_kind_key(kind: &TypeKind) -> &'static str {
    match kind {
        TypeKind::Any(_) => "any",
        TypeKind::Never(_) => "never",
        TypeKind::Unknown(_) => "unknown",
        TypeKind::Null(_) => "null",
        TypeKind::Bool(_) => "bool",
        TypeKind::Char(_) => "char",
        TypeKind::Number(_) => "number",
        TypeKind::String(_) => "string",
        TypeKind::Bytes(_) => "bytes",
        TypeKind::Temporal(_) => "temporal",
        TypeKind::Uuid => "uuid",
        TypeKind::IpAddr(_) => "ip_addr",
        TypeKind::Json => "json",
        TypeKind::Optional(_) => "optional",
        TypeKind::Array(_) => "array",
        TypeKind::List(_) => "list",
        TypeKind::Tuple(_) => "tuple",
        TypeKind::Map(_) => "map",
        TypeKind::Set(_) => "set",
        TypeKind::Record(_) => "record",
        TypeKind::Attribute(_) => "attribute",
        TypeKind::Class(_) => "class",
        TypeKind::Union(_) => "union",
        TypeKind::Intersection(_) => "intersection",
        TypeKind::Variant(_) => "variant",
        TypeKind::Enum(_) => "enum",
        TypeKind::Result(_) => "result",
        TypeKind::Function(_) => "function",
        TypeKind::Interface(_) => "interface",
        TypeKind::Handle(_) => "handle",
        TypeKind::Stream(_) => "stream",
        TypeKind::Opaque(_) => "opaque",
        TypeKind::Extension(_) => "extension",
        TypeKind::Ref(_) => "ref",
    }
}
