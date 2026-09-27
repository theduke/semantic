use semantic_data::schema::{AttributeType, ClassType};
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::StoredCollection;

use crate::{
    form::{ClassFormField, class_form_fields},
    ui_catalog::UiCatalog,
};

impl UiCatalog {
    pub fn attribute_by_id(&self, id: &str) -> Option<&AttributeType> {
        self.attributes_by_id().get(id)
    }

    pub fn attribute_by_name(&self, name: &str) -> Option<&AttributeType> {
        self.attributes_by_name().get(name)
    }

    /// Resolves a stored field identifier to its human-readable schema title.
    pub fn attribute_title(&self, field: &str) -> String {
        let attribute = self
            .attribute_by_id(field)
            .or_else(|| self.attribute_by_name(field));
        attribute_display_title(attribute, field)
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

    /// Classes offered by entity creators. Only an explicit false opts out.
    pub fn creatable_classes(&self) -> impl Iterator<Item = &ClassType> {
        self.classes()
            .filter(|class| class.creatable_in_ui != Some(false))
    }

    pub fn attributes(&self) -> impl Iterator<Item = &AttributeType> {
        self.attributes_by_id().values()
    }

    pub fn collections(&self) -> impl Iterator<Item = &StoredCollection> {
        self.collections_by_name()
            .values()
            .filter(|collection| !collection.internal)
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

    pub fn class_form_fields(&self, class: &ClassType) -> Vec<ClassFormField> {
        class_form_fields(self, class)
    }
}

fn attribute_display_title(attribute: Option<&AttributeType>, fallback: &str) -> String {
    attribute
        .and_then(|attribute| {
            attribute
                .meta
                .title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .or_else(|| (!attribute.name.trim().is_empty()).then_some(attribute.name.as_str()))
        })
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(test)]
mod tests {
    use semantic_data::filestore::{ATTR_FILE_FILENAME, file_attributes};

    use super::attribute_display_title;

    #[test]
    fn attribute_title_prefers_metadata_and_falls_back_cleanly() {
        let mut attribute = file_attributes()
            .into_iter()
            .find(|attribute| attribute.id == ATTR_FILE_FILENAME)
            .expect("filename attribute");

        assert_eq!(
            attribute_display_title(Some(&attribute), ATTR_FILE_FILENAME),
            "Filename"
        );

        attribute.meta.title = None;
        assert_eq!(
            attribute_display_title(Some(&attribute), ATTR_FILE_FILENAME),
            "filename"
        );

        attribute.meta.title = Some("File name".to_string());
        assert_eq!(
            attribute_display_title(Some(&attribute), ATTR_FILE_FILENAME),
            "File name"
        );
        assert_eq!(
            attribute_display_title(None, "custom:field"),
            "custom:field"
        );
    }
}
