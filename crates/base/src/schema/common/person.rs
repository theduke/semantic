use std::collections::BTreeMap;

use semantic_data::{
    attr::ATTR_TITLE,
    schema::{AttributeType, ClassType},
};

use super::helpers;

pub const CLASS_ID: &str = "semantic:base:person";

pub const ATTR_DISPLAY_NAME: &str = "semantic:base:person:display_name";
pub const ATTR_GIVEN_NAME: &str = "semantic:base:person:given_name";
pub const ATTR_FAMILY_NAME: &str = "semantic:base:person:family_name";
pub const ATTR_MIDDLE_NAME: &str = "semantic:base:person:middle_name";
pub const ATTR_HONORIFIC_PREFIX: &str = "semantic:base:person:honorific_prefix";
pub const ATTR_HONORIFIC_SUFFIX: &str = "semantic:base:person:honorific_suffix";
pub const ATTR_NICKNAME: &str = "semantic:base:person:nickname";
pub const ATTR_ALTERNATE_NAMES: &str = "semantic:base:person:alternate_names";
pub const ATTR_DESCRIPTION: &str = "semantic:base:person:description";
pub const ATTR_BIRTH_DATE: &str = "semantic:base:person:birth_date";
pub const ATTR_DEATH_DATE: &str = "semantic:base:person:death_date";
pub const ATTR_PARENT: &str = "semantic:parent";

pub fn attributes() -> Vec<AttributeType> {
    vec![
        helpers::attribute(ATTR_TITLE, "title", helpers::string_type()),
        helpers::attribute(ATTR_DISPLAY_NAME, "display_name", helpers::string_type()),
        helpers::attribute(ATTR_GIVEN_NAME, "given_name", helpers::string_type()),
        helpers::attribute(ATTR_FAMILY_NAME, "family_name", helpers::string_type()),
        helpers::attribute(ATTR_MIDDLE_NAME, "middle_name", helpers::string_type()),
        helpers::attribute(
            ATTR_HONORIFIC_PREFIX,
            "honorific_prefix",
            helpers::string_type(),
        ),
        helpers::attribute(
            ATTR_HONORIFIC_SUFFIX,
            "honorific_suffix",
            helpers::string_type(),
        ),
        helpers::attribute(ATTR_NICKNAME, "nickname", helpers::string_type()),
        helpers::attribute(
            ATTR_ALTERNATE_NAMES,
            "alternate_names",
            helpers::list_type(helpers::string_type()),
        ),
        helpers::attribute(ATTR_DESCRIPTION, "description", helpers::string_type()),
        helpers::attribute(ATTR_BIRTH_DATE, "birth_date", helpers::date_type()),
        helpers::attribute(ATTR_DEATH_DATE, "death_date", helpers::date_type()),
        helpers::attribute(
            ATTR_PARENT,
            "parent",
            helpers::ref_type(semantic_data::builtin::ATTR_ID),
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
        strict_schema: false,
        attributes: BTreeMap::from([
            ("title".to_string(), class_attribute(ATTR_TITLE, 5)),
            (
                "display_name".to_string(),
                class_attribute(ATTR_DISPLAY_NAME, 10),
            ),
            (
                "given_name".to_string(),
                class_attribute(ATTR_GIVEN_NAME, 20),
            ),
            (
                "family_name".to_string(),
                class_attribute(ATTR_FAMILY_NAME, 30),
            ),
            (
                "middle_name".to_string(),
                class_attribute(ATTR_MIDDLE_NAME, 40),
            ),
            (
                "honorific_prefix".to_string(),
                class_attribute(ATTR_HONORIFIC_PREFIX, 50),
            ),
            (
                "honorific_suffix".to_string(),
                class_attribute(ATTR_HONORIFIC_SUFFIX, 60),
            ),
            ("nickname".to_string(), class_attribute(ATTR_NICKNAME, 70)),
            (
                "alternate_names".to_string(),
                class_attribute(ATTR_ALTERNATE_NAMES, 80),
            ),
            (
                "description".to_string(),
                class_attribute(ATTR_DESCRIPTION, 90),
            ),
            (
                "birth_date".to_string(),
                class_attribute(ATTR_BIRTH_DATE, 100),
            ),
            (
                "death_date".to_string(),
                class_attribute(ATTR_DEATH_DATE, 110),
            ),
            ("parent".to_string(), class_attribute(ATTR_PARENT, 120)),
        ]),
        constraints: Vec::new(),
        meta: helpers::meta_with_title("Person"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_declared_attributes_have_titles() {
        for attribute in attributes() {
            assert!(
                attribute
                    .meta
                    .title
                    .as_deref()
                    .is_some_and(|title| !title.is_empty()),
                "{} should have a title",
                attribute.id
            );
        }
    }

    #[test]
    fn all_declared_class_attributes_have_titles() {
        for (name, attribute) in class().attributes {
            assert!(
                attribute
                    .meta
                    .title
                    .as_deref()
                    .is_some_and(|title| !title.is_empty()),
                "{name} should have a title"
            );
        }
    }
}
