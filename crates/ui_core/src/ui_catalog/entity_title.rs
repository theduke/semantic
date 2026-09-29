use semantic_data::filestore::ATTR_FILE_FILENAME;
use semantic_data::value::{Object, Value};

use crate::ui_catalog::UiCatalog;

/// Object fields considered, in priority order, when deriving an entity title.
pub const ENTITY_TITLE_FIELDS: [&str; 9] = [
    "semantic:title",
    "semantic:base:label:name",
    ATTR_FILE_FILENAME,
    "title",
    "name",
    "display_name",
    "semantic:base:person:display_name",
    "semantic:base:file:filename",
    "filename",
];

/// Title shown when an entity has neither a title field nor an id.
pub const UNKNOWN_ENTITY_TITLE: &str = "<unknown>";

impl UiCatalog {
    /// Resolves the human-readable title of an entity object.
    ///
    /// Uses the first non-empty title field, then the entity id.
    pub fn entity_title(&self, object: &Object) -> String {
        ENTITY_TITLE_FIELDS
            .iter()
            .chain(["id"].iter())
            .find_map(|field| {
                object
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or(UNKNOWN_ENTITY_TITLE)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::filestore::ATTR_FILE_FILENAME;
    use semantic_data::value::{Object, Value};

    use super::UNKNOWN_ENTITY_TITLE;
    use crate::ui_catalog::UiCatalog;

    fn object_entity_title(object: &Object) -> String {
        UiCatalog::empty().entity_title(object)
    }

    fn object(fields: &[(&str, &str)]) -> Object {
        let mut object = Object::new();
        for (key, value) in fields {
            object.insert(*key, Value::String(value.to_string()));
        }
        object
    }

    #[test]
    fn prefers_title_fields_in_priority_order() {
        let mut object = object(&[("id", "file-1"), ("filename", "photo.jpg")]);
        assert_eq!(object_entity_title(&object), "photo.jpg");

        object.insert(ATTR_FILE_FILENAME, Value::String("canonical.jpg".into()));
        assert_eq!(object_entity_title(&object), "canonical.jpg");

        object.insert("semantic:title", Value::String("Featured".into()));
        assert_eq!(object_entity_title(&object), "Featured");
    }

    #[test]
    fn falls_back_to_id_then_unknown() {
        assert_eq!(
            object_entity_title(&object(&[("id", "entity-1"), ("semantic:title", " ")])),
            "entity-1"
        );
        assert_eq!(object_entity_title(&Object::new()), UNKNOWN_ENTITY_TITLE);
    }
}
