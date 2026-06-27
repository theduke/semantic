use semantic_data::schema::{AttributeType, ClassType};
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::StoredCollection;

use crate::ui_catalog::UiCatalog;

impl UiCatalog {
    pub fn attribute_by_id(&self, id: &str) -> Option<&AttributeType> {
        self.attributes_by_id().get(id)
    }

    pub fn attribute_by_name(&self, name: &str) -> Option<&AttributeType> {
        self.attributes_by_name().get(name)
    }

    pub fn class_by_id(&self, id: &str) -> Option<&ClassType> {
        self.classes_by_id().get(id)
    }

    pub fn class_by_name(&self, name: &str) -> Option<&ClassType> {
        self.classes_by_name().get(name)
    }

    pub fn collection_by_name(&self, name: &str) -> Option<&StoredCollection> {
        self.collections_by_name().get(name)
    }

    pub fn classes(&self) -> impl Iterator<Item = &ClassType> {
        self.classes_by_id().values()
    }

    pub fn attributes(&self) -> impl Iterator<Item = &AttributeType> {
        self.attributes_by_id().values()
    }

    pub fn collections(&self) -> impl Iterator<Item = &StoredCollection> {
        self.collections_by_name().values()
    }

    pub fn class_inherits(&self, class_id: &str, target_class_id: &str) -> bool {
        let Some(class) = self.class_by_id(class_id) else {
            return false;
        };
        if class.id == target_class_id {
            return true;
        }
        if class
            .inherits
            .as_ref()
            .is_some_and(|parent| parent.id == target_class_id)
        {
            return true;
        }
        class
            .extends
            .iter()
            .any(|parent| parent.id == target_class_id)
    }

    pub fn object_class<'a>(&'a self, object: &'a Object) -> Option<&'a ClassType> {
        let Some(Value::String(class_id)) = object.get("type") else {
            return None;
        };
        self.class_by_id(class_id)
            .or_else(|| self.class_by_name(class_id))
    }
}
