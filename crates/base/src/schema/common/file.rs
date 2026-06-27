use std::collections::BTreeMap;

use semantic_data::schema::{AttributeType, ClassType};

use super::helpers;

pub const CLASS_ID: &str = "semantic.base.file";

pub const FILENAME_ATTRIBUTE_ID: &str = "semantic.base.file.filename";
pub const BYTE_SIZE_ATTRIBUTE_ID: &str = "semantic.base.file.byte_size";
pub const MIME_TYPE_ATTRIBUTE_ID: &str = "semantic.base.file.mime_type";
pub const CONTENT_HASH_SHA256_ATTRIBUTE_ID: &str = "semantic.base.file.content_hash_sha256";

pub fn attributes() -> Vec<AttributeType> {
    vec![
        helpers::attribute(FILENAME_ATTRIBUTE_ID, "filename", helpers::string_type()),
        helpers::attribute(BYTE_SIZE_ATTRIBUTE_ID, "byte_size", helpers::uint64_type()),
        helpers::attribute(MIME_TYPE_ATTRIBUTE_ID, "mime_type", helpers::string_type()),
        helpers::attribute(
            CONTENT_HASH_SHA256_ATTRIBUTE_ID,
            "content_hash_sha256",
            helpers::string_type(),
        ),
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
            (
                "filename".to_string(),
                class_attribute(FILENAME_ATTRIBUTE_ID, 10),
            ),
            (
                "byte_size".to_string(),
                class_attribute(BYTE_SIZE_ATTRIBUTE_ID, 20),
            ),
            (
                "mime_type".to_string(),
                class_attribute(MIME_TYPE_ATTRIBUTE_ID, 30),
            ),
            (
                "content_hash_sha256".to_string(),
                class_attribute(CONTENT_HASH_SHA256_ATTRIBUTE_ID, 40),
            ),
        ]),
        constraints: Vec::new(),
        meta: helpers::meta_with_title("File"),
    }
}
