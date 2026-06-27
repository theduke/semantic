use std::collections::BTreeMap;

use semantic_data::schema::{AttributeType, ClassType};

use super::helpers;

pub const CLASS_ID: &str = "semantic:base:person";

pub const DISPLAY_NAME_ATTRIBUTE_ID: &str = "semantic:base:person:display_name";
pub const GIVEN_NAME_ATTRIBUTE_ID: &str = "semantic:base:person:given_name";
pub const FAMILY_NAME_ATTRIBUTE_ID: &str = "semantic:base:person:family_name";
pub const MIDDLE_NAME_ATTRIBUTE_ID: &str = "semantic:base:person:middle_name";
pub const HONORIFIC_PREFIX_ATTRIBUTE_ID: &str = "semantic:base:person:honorific_prefix";
pub const HONORIFIC_SUFFIX_ATTRIBUTE_ID: &str = "semantic:base:person:honorific_suffix";
pub const NICKNAME_ATTRIBUTE_ID: &str = "semantic:base:person:nickname";
pub const ALTERNATE_NAMES_ATTRIBUTE_ID: &str = "semantic:base:person:alternate_names";
pub const DESCRIPTION_ATTRIBUTE_ID: &str = "semantic:base:person:description";
pub const BIRTH_DATE_ATTRIBUTE_ID: &str = "semantic:base:person:birth_date";
pub const DEATH_DATE_ATTRIBUTE_ID: &str = "semantic:base:person:death_date";
pub const PARENT_ATTRIBUTE_ID: &str = "semantic:parent";

pub fn attributes() -> Vec<AttributeType> {
    vec![
        helpers::attribute(
            DISPLAY_NAME_ATTRIBUTE_ID,
            "display_name",
            helpers::string_type(),
        ),
        helpers::attribute(
            GIVEN_NAME_ATTRIBUTE_ID,
            "given_name",
            helpers::string_type(),
        ),
        helpers::attribute(
            FAMILY_NAME_ATTRIBUTE_ID,
            "family_name",
            helpers::string_type(),
        ),
        helpers::attribute(
            MIDDLE_NAME_ATTRIBUTE_ID,
            "middle_name",
            helpers::string_type(),
        ),
        helpers::attribute(
            HONORIFIC_PREFIX_ATTRIBUTE_ID,
            "honorific_prefix",
            helpers::string_type(),
        ),
        helpers::attribute(
            HONORIFIC_SUFFIX_ATTRIBUTE_ID,
            "honorific_suffix",
            helpers::string_type(),
        ),
        helpers::attribute(NICKNAME_ATTRIBUTE_ID, "nickname", helpers::string_type()),
        helpers::attribute(
            ALTERNATE_NAMES_ATTRIBUTE_ID,
            "alternate_names",
            helpers::list_type(helpers::string_type()),
        ),
        helpers::attribute(
            DESCRIPTION_ATTRIBUTE_ID,
            "description",
            helpers::string_type(),
        ),
        helpers::attribute(BIRTH_DATE_ATTRIBUTE_ID, "birth_date", helpers::date_type()),
        helpers::attribute(DEATH_DATE_ATTRIBUTE_ID, "death_date", helpers::date_type()),
        helpers::attribute(
            PARENT_ATTRIBUTE_ID,
            "parent",
            helpers::ref_type(semantic_data::builtin::ID_ATTRIBUTE_ID),
        ),
    ]
}

pub fn class() -> ClassType {
    let class_attribute = |attribute_id, ui_order| {
        helpers::class_attribute_with_ui_order(attribute_id, false, Some(ui_order))
    };

    ClassType {
        id: CLASS_ID.to_string(),
        name: "Person".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "display_name".to_string(),
                class_attribute(DISPLAY_NAME_ATTRIBUTE_ID, 10),
            ),
            (
                "given_name".to_string(),
                class_attribute(GIVEN_NAME_ATTRIBUTE_ID, 20),
            ),
            (
                "family_name".to_string(),
                class_attribute(FAMILY_NAME_ATTRIBUTE_ID, 30),
            ),
            (
                "middle_name".to_string(),
                class_attribute(MIDDLE_NAME_ATTRIBUTE_ID, 40),
            ),
            (
                "honorific_prefix".to_string(),
                class_attribute(HONORIFIC_PREFIX_ATTRIBUTE_ID, 50),
            ),
            (
                "honorific_suffix".to_string(),
                class_attribute(HONORIFIC_SUFFIX_ATTRIBUTE_ID, 60),
            ),
            (
                "nickname".to_string(),
                class_attribute(NICKNAME_ATTRIBUTE_ID, 70),
            ),
            (
                "alternate_names".to_string(),
                class_attribute(ALTERNATE_NAMES_ATTRIBUTE_ID, 80),
            ),
            (
                "description".to_string(),
                class_attribute(DESCRIPTION_ATTRIBUTE_ID, 90),
            ),
            (
                "birth_date".to_string(),
                class_attribute(BIRTH_DATE_ATTRIBUTE_ID, 100),
            ),
            (
                "death_date".to_string(),
                class_attribute(DEATH_DATE_ATTRIBUTE_ID, 110),
            ),
            (
                "parent".to_string(),
                class_attribute(PARENT_ATTRIBUTE_ID, 120),
            ),
        ]),
        constraints: Vec::new(),
        meta: helpers::meta_with_title("Person"),
    }
}
