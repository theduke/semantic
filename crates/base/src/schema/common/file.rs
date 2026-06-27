use std::collections::BTreeMap;

use semantic_data::schema::{AttributeType, ClassType, StringFormat};

use super::helpers;

pub const CLASS_ID: &str = "semantic.base.file";

pub const NAME_ATTRIBUTE_ID: &str = "semantic.base.file.name";
pub const PATH_ATTRIBUTE_ID: &str = "semantic.base.file.path";
pub const URI_ATTRIBUTE_ID: &str = "semantic.base.file.uri";
pub const MEDIA_TYPE_ATTRIBUTE_ID: &str = "semantic.base.file.media_type";
pub const EXTENSION_ATTRIBUTE_ID: &str = "semantic.base.file.extension";
pub const BYTE_SIZE_ATTRIBUTE_ID: &str = "semantic.base.file.byte_size";
pub const CONTENT_HASH_ATTRIBUTE_ID: &str = "semantic.base.file.content_hash";
pub const HASH_ALGORITHM_ATTRIBUTE_ID: &str = "semantic.base.file.hash_algorithm";
pub const CREATED_AT_ATTRIBUTE_ID: &str = "semantic.base.file.created_at";
pub const MODIFIED_AT_ATTRIBUTE_ID: &str = "semantic.base.file.modified_at";
pub const ACCESSED_AT_ATTRIBUTE_ID: &str = "semantic.base.file.accessed_at";
pub const ENCODING_ATTRIBUTE_ID: &str = "semantic.base.file.encoding";
pub const LANGUAGE_ATTRIBUTE_ID: &str = "semantic.base.file.language";
pub const TAGS_ATTRIBUTE_ID: &str = "semantic.base.file.tags";
pub const METADATA_ATTRIBUTE_ID: &str = "semantic.base.file.metadata";

pub fn attributes() -> Vec<AttributeType> {
    vec![
        helpers::attribute(NAME_ATTRIBUTE_ID, "name", helpers::string_type()),
        helpers::attribute(PATH_ATTRIBUTE_ID, "path", helpers::string_type()),
        helpers::attribute(
            URI_ATTRIBUTE_ID,
            "uri",
            helpers::string_format_type(StringFormat::Uri),
        ),
        helpers::attribute(
            MEDIA_TYPE_ATTRIBUTE_ID,
            "media_type",
            helpers::string_type(),
        ),
        helpers::attribute(EXTENSION_ATTRIBUTE_ID, "extension", helpers::string_type()),
        helpers::attribute(BYTE_SIZE_ATTRIBUTE_ID, "byte_size", helpers::uint64_type()),
        helpers::attribute(
            CONTENT_HASH_ATTRIBUTE_ID,
            "content_hash",
            helpers::string_type(),
        ),
        helpers::attribute(
            HASH_ALGORITHM_ATTRIBUTE_ID,
            "hash_algorithm",
            helpers::string_type(),
        ),
        helpers::attribute(
            CREATED_AT_ATTRIBUTE_ID,
            "created_at",
            helpers::instant_type(),
        ),
        helpers::attribute(
            MODIFIED_AT_ATTRIBUTE_ID,
            "modified_at",
            helpers::instant_type(),
        ),
        helpers::attribute(
            ACCESSED_AT_ATTRIBUTE_ID,
            "accessed_at",
            helpers::instant_type(),
        ),
        helpers::attribute(ENCODING_ATTRIBUTE_ID, "encoding", helpers::string_type()),
        helpers::attribute(LANGUAGE_ATTRIBUTE_ID, "language", helpers::string_type()),
        helpers::attribute(
            TAGS_ATTRIBUTE_ID,
            "tags",
            helpers::list_type(helpers::string_type()),
        ),
        helpers::attribute(METADATA_ATTRIBUTE_ID, "metadata", helpers::json_type()),
    ]
}

pub fn class() -> ClassType {
    let class_attribute = |attribute_id, ui_order| {
        helpers::class_attribute_with_ui_order(attribute_id, false, Some(ui_order))
    };

    ClassType {
        id: CLASS_ID.to_string(),
        name: "File".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            ("name".to_string(), class_attribute(NAME_ATTRIBUTE_ID, 10)),
            ("path".to_string(), class_attribute(PATH_ATTRIBUTE_ID, 20)),
            ("uri".to_string(), class_attribute(URI_ATTRIBUTE_ID, 30)),
            (
                "media_type".to_string(),
                class_attribute(MEDIA_TYPE_ATTRIBUTE_ID, 40),
            ),
            (
                "extension".to_string(),
                class_attribute(EXTENSION_ATTRIBUTE_ID, 50),
            ),
            (
                "byte_size".to_string(),
                class_attribute(BYTE_SIZE_ATTRIBUTE_ID, 60),
            ),
            (
                "content_hash".to_string(),
                class_attribute(CONTENT_HASH_ATTRIBUTE_ID, 70),
            ),
            (
                "hash_algorithm".to_string(),
                class_attribute(HASH_ALGORITHM_ATTRIBUTE_ID, 80),
            ),
            (
                "created_at".to_string(),
                class_attribute(CREATED_AT_ATTRIBUTE_ID, 90),
            ),
            (
                "modified_at".to_string(),
                class_attribute(MODIFIED_AT_ATTRIBUTE_ID, 100),
            ),
            (
                "accessed_at".to_string(),
                class_attribute(ACCESSED_AT_ATTRIBUTE_ID, 110),
            ),
            (
                "encoding".to_string(),
                class_attribute(ENCODING_ATTRIBUTE_ID, 120),
            ),
            (
                "language".to_string(),
                class_attribute(LANGUAGE_ATTRIBUTE_ID, 130),
            ),
            ("tags".to_string(), class_attribute(TAGS_ATTRIBUTE_ID, 140)),
            (
                "metadata".to_string(),
                class_attribute(METADATA_ATTRIBUTE_ID, 150),
            ),
        ]),
        constraints: Vec::new(),
        meta: helpers::meta_with_title("File"),
    }
}
