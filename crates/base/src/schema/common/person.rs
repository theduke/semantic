use std::collections::BTreeMap;

use semantic_data::schema::{AttributeType, ClassType, Meta, StringFormat};

use super::helpers;

pub const CLASS_ID: &str = "semantic.base.person";

pub const DISPLAY_NAME_ATTRIBUTE_ID: &str = "semantic.base.person.display_name";
pub const GIVEN_NAME_ATTRIBUTE_ID: &str = "semantic.base.person.given_name";
pub const FAMILY_NAME_ATTRIBUTE_ID: &str = "semantic.base.person.family_name";
pub const MIDDLE_NAME_ATTRIBUTE_ID: &str = "semantic.base.person.middle_name";
pub const HONORIFIC_PREFIX_ATTRIBUTE_ID: &str = "semantic.base.person.honorific_prefix";
pub const HONORIFIC_SUFFIX_ATTRIBUTE_ID: &str = "semantic.base.person.honorific_suffix";
pub const NICKNAME_ATTRIBUTE_ID: &str = "semantic.base.person.nickname";
pub const PRONOUNS_ATTRIBUTE_ID: &str = "semantic.base.person.pronouns";
pub const ALTERNATE_NAMES_ATTRIBUTE_ID: &str = "semantic.base.person.alternate_names";
pub const DESCRIPTION_ATTRIBUTE_ID: &str = "semantic.base.person.description";
pub const BIRTH_DATE_ATTRIBUTE_ID: &str = "semantic.base.person.birth_date";
pub const DEATH_DATE_ATTRIBUTE_ID: &str = "semantic.base.person.death_date";
pub const EMAILS_ATTRIBUTE_ID: &str = "semantic.base.person.emails";
pub const PHONES_ATTRIBUTE_ID: &str = "semantic.base.person.phones";
pub const URLS_ATTRIBUTE_ID: &str = "semantic.base.person.urls";
pub const IMAGE_URI_ATTRIBUTE_ID: &str = "semantic.base.person.image_uri";
pub const LOCALE_ATTRIBUTE_ID: &str = "semantic.base.person.locale";
pub const TIMEZONE_ATTRIBUTE_ID: &str = "semantic.base.person.timezone";
pub const IDENTIFIERS_ATTRIBUTE_ID: &str = "semantic.base.person.identifiers";
pub const METADATA_ATTRIBUTE_ID: &str = "semantic.base.person.metadata";

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
        helpers::attribute(PRONOUNS_ATTRIBUTE_ID, "pronouns", helpers::string_type()),
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
            EMAILS_ATTRIBUTE_ID,
            "emails",
            helpers::list_type(helpers::string_format_type(StringFormat::Email)),
        ),
        helpers::attribute(
            PHONES_ATTRIBUTE_ID,
            "phones",
            helpers::list_type(helpers::string_type()),
        ),
        helpers::attribute(
            URLS_ATTRIBUTE_ID,
            "urls",
            helpers::list_type(helpers::string_format_type(StringFormat::Url)),
        ),
        helpers::attribute(
            IMAGE_URI_ATTRIBUTE_ID,
            "image_uri",
            helpers::string_format_type(StringFormat::Uri),
        ),
        helpers::attribute(LOCALE_ATTRIBUTE_ID, "locale", helpers::string_type()),
        helpers::attribute(TIMEZONE_ATTRIBUTE_ID, "timezone", helpers::string_type()),
        helpers::attribute(
            IDENTIFIERS_ATTRIBUTE_ID,
            "identifiers",
            helpers::map_string_string_type(),
        ),
        helpers::attribute(METADATA_ATTRIBUTE_ID, "metadata", helpers::json_type()),
    ]
}

pub fn class() -> ClassType {
    ClassType {
        id: CLASS_ID.to_string(),
        name: "Person".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "display_name".to_string(),
                helpers::class_attribute(DISPLAY_NAME_ATTRIBUTE_ID, false),
            ),
            (
                "given_name".to_string(),
                helpers::class_attribute(GIVEN_NAME_ATTRIBUTE_ID, false),
            ),
            (
                "family_name".to_string(),
                helpers::class_attribute(FAMILY_NAME_ATTRIBUTE_ID, false),
            ),
            (
                "middle_name".to_string(),
                helpers::class_attribute(MIDDLE_NAME_ATTRIBUTE_ID, false),
            ),
            (
                "honorific_prefix".to_string(),
                helpers::class_attribute(HONORIFIC_PREFIX_ATTRIBUTE_ID, false),
            ),
            (
                "honorific_suffix".to_string(),
                helpers::class_attribute(HONORIFIC_SUFFIX_ATTRIBUTE_ID, false),
            ),
            (
                "nickname".to_string(),
                helpers::class_attribute(NICKNAME_ATTRIBUTE_ID, false),
            ),
            (
                "pronouns".to_string(),
                helpers::class_attribute(PRONOUNS_ATTRIBUTE_ID, false),
            ),
            (
                "alternate_names".to_string(),
                helpers::class_attribute(ALTERNATE_NAMES_ATTRIBUTE_ID, false),
            ),
            (
                "description".to_string(),
                helpers::class_attribute(DESCRIPTION_ATTRIBUTE_ID, false),
            ),
            (
                "birth_date".to_string(),
                helpers::class_attribute(BIRTH_DATE_ATTRIBUTE_ID, false),
            ),
            (
                "death_date".to_string(),
                helpers::class_attribute(DEATH_DATE_ATTRIBUTE_ID, false),
            ),
            (
                "emails".to_string(),
                helpers::class_attribute(EMAILS_ATTRIBUTE_ID, false),
            ),
            (
                "phones".to_string(),
                helpers::class_attribute(PHONES_ATTRIBUTE_ID, false),
            ),
            (
                "urls".to_string(),
                helpers::class_attribute(URLS_ATTRIBUTE_ID, false),
            ),
            (
                "image_uri".to_string(),
                helpers::class_attribute(IMAGE_URI_ATTRIBUTE_ID, false),
            ),
            (
                "locale".to_string(),
                helpers::class_attribute(LOCALE_ATTRIBUTE_ID, false),
            ),
            (
                "timezone".to_string(),
                helpers::class_attribute(TIMEZONE_ATTRIBUTE_ID, false),
            ),
            (
                "identifiers".to_string(),
                helpers::class_attribute(IDENTIFIERS_ATTRIBUTE_ID, false),
            ),
            (
                "metadata".to_string(),
                helpers::class_attribute(METADATA_ATTRIBUTE_ID, false),
            ),
        ]),
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}
