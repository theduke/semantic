use std::collections::BTreeMap;

use semantic_data::schema::TypeKind;

use crate::{
    form::{AttributeFormRenderer, ClassFormRenderer, ValueFormRenderer},
    ui_catalog::type_kind_key,
};

#[derive(Clone, Default)]
pub struct UiFormRegistry {
    class_form_renderers: BTreeMap<String, ClassFormRenderer>,
    class_field_form_renderers: BTreeMap<(String, String), AttributeFormRenderer>,
    attribute_form_renderers: BTreeMap<String, AttributeFormRenderer>,
    type_form_renderers: BTreeMap<&'static str, ValueFormRenderer>,
    fallback_form_renderer: Option<ValueFormRenderer>,
}

impl UiFormRegistry {
    pub fn register_class_form_renderer(
        &mut self,
        class_id: impl Into<String>,
        renderer: ClassFormRenderer,
    ) {
        self.class_form_renderers.insert(class_id.into(), renderer);
    }

    pub fn register_class_field_form_renderer(
        &mut self,
        class_id: impl Into<String>,
        field_name: impl Into<String>,
        renderer: AttributeFormRenderer,
    ) {
        self.class_field_form_renderers
            .insert((class_id.into(), field_name.into()), renderer);
    }

    pub fn register_attribute_form_renderer(
        &mut self,
        attribute_id: impl Into<String>,
        renderer: AttributeFormRenderer,
    ) {
        self.attribute_form_renderers
            .insert(attribute_id.into(), renderer);
    }

    pub fn register_type_form_renderer(&mut self, kind: &'static str, renderer: ValueFormRenderer) {
        self.type_form_renderers.insert(kind, renderer);
    }

    pub fn set_fallback_form_renderer(&mut self, renderer: ValueFormRenderer) {
        self.fallback_form_renderer = Some(renderer);
    }

    pub fn class_form_renderer(&self, class_id: &str) -> Option<ClassFormRenderer> {
        self.class_form_renderers.get(class_id).cloned()
    }

    pub fn class_field_form_renderer(
        &self,
        class_id: &str,
        field_name: &str,
    ) -> Option<AttributeFormRenderer> {
        self.class_field_form_renderers
            .get(&(class_id.to_string(), field_name.to_string()))
            .cloned()
    }

    pub fn attribute_form_renderer(&self, attribute_id: &str) -> Option<AttributeFormRenderer> {
        self.attribute_form_renderers.get(attribute_id).cloned()
    }

    pub fn type_form_renderer(&self, kind: &TypeKind) -> Option<ValueFormRenderer> {
        self.type_form_renderers.get(type_kind_key(kind)).cloned()
    }

    pub fn fallback_form_renderer(&self) -> Option<ValueFormRenderer> {
        self.fallback_form_renderer.clone()
    }
}
