use std::collections::BTreeMap;

use crate::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, Constraint, Meta,
    MigrationCollectionKind, MigrationIntegrityMode, StringFormat, StringType, Type, TypeKind,
};
use crate::schema::{Migration, MigrationDdlOperation, MigrationOperation};

pub const MODULE_NAME: &str = "auth";
pub const INIT_MIGRATION_NAME: &str = "001_init";
pub const USER_CLASS_ID: &str = "semantic:auth:user";
pub const AUTH_COLLECTION: &str = "_semantic.auth";

pub const ATTR_USERNAME: &str = "semantic:auth:user:username";
pub const ATTR_PRIMARY_EMAIL: &str = "semantic:auth:user:primary_email";

pub const ATTR_DESCRIPTION: &str = "semantic:description";
pub const ATTR_CREATED_AT: &str = "semantic:created_at";
pub const ATTR_UPDATED_AT: &str = "semantic:updated_at";

pub fn attributes() -> Vec<AttributeType> {
    vec![username_attribute(), primary_email_attribute()]
}

pub fn classes() -> Vec<ClassType> {
    vec![user_class()]
}

pub fn migrations() -> Vec<Migration> {
    vec![init_migration()]
}

pub fn init_migration() -> Migration {
    Migration {
        module: MODULE_NAME.to_string(),
        name: INIT_MIGRATION_NAME.to_string(),
        description: Some("Initial semantic auth schema.".to_string()),
        operations: vec![
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                attribute: init_migration_username_attribute(),
            }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                attribute: init_migration_primary_email_attribute(),
            }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass {
                class: init_migration_user_class(),
            }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
                name: AUTH_COLLECTION.to_string(),
                kind: MigrationCollectionKind::Schema,
                integrity_mode: MigrationIntegrityMode::StrictRegisteredSchema,
            }),
        ],
        meta: Meta::default(),
    }
}

pub fn user_class() -> ClassType {
    ClassType {
        id: USER_CLASS_ID.to_string(),
        name: "User".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "username".to_string(),
                class_attribute(ATTR_USERNAME, true, Some(10)),
            ),
            (
                "primary_email".to_string(),
                class_attribute(ATTR_PRIMARY_EMAIL, true, Some(20)),
            ),
            (
                "description".to_string(),
                class_attribute(ATTR_DESCRIPTION, false, Some(30)),
            ),
            (
                "created_at".to_string(),
                class_attribute(ATTR_CREATED_AT, false, Some(40)),
            ),
            (
                "updated_at".to_string(),
                class_attribute(ATTR_UPDATED_AT, false, Some(50)),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta_with_title("User"),
    }
}

pub fn username_attribute() -> AttributeType {
    attribute(ATTR_USERNAME, "username", string_type())
}

pub fn primary_email_attribute() -> AttributeType {
    attribute(ATTR_PRIMARY_EMAIL, "primary_email", email_type())
}

fn init_migration_username_attribute() -> AttributeType {
    migration_attribute(
        ATTR_USERNAME,
        "username",
        migration_string_type(),
        "Username",
    )
}

fn init_migration_primary_email_attribute() -> AttributeType {
    migration_attribute(
        ATTR_PRIMARY_EMAIL,
        "primary_email",
        migration_email_type(),
        "Primary Email",
    )
}

fn init_migration_user_class() -> ClassType {
    ClassType {
        id: USER_CLASS_ID.to_string(),
        name: "User".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "username".to_string(),
                migration_class_attribute(ATTR_USERNAME, true, Some(10), "Username"),
            ),
            (
                "primary_email".to_string(),
                migration_class_attribute(ATTR_PRIMARY_EMAIL, true, Some(20), "Primary Email"),
            ),
            (
                "description".to_string(),
                migration_class_attribute(ATTR_DESCRIPTION, false, Some(30), "Description"),
            ),
            (
                "created_at".to_string(),
                migration_class_attribute(ATTR_CREATED_AT, false, Some(40), "Created At"),
            ),
            (
                "updated_at".to_string(),
                migration_class_attribute(ATTR_UPDATED_AT, false, Some(50), "Updated At"),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta_with_title("User"),
    }
}

fn migration_attribute(id: &str, name: &str, ty: Type, title: impl Into<String>) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty,
        constraints: Vec::<Constraint>::new(),
        meta: meta_with_title(title),
    }
}

fn migration_class_attribute(
    attribute_id: &str,
    required: bool,
    ui_order: Option<u32>,
    title: impl Into<String>,
) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef {
            id: attribute_id.to_string(),
        },
        required,
        ui_order,
        computed: None,
        constraints: Vec::new(),
        meta: meta_with_title(title),
    }
}

fn migration_string_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn migration_email_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: Some(StringFormat::Email),
        normalization: None,
    }))
}

fn attribute(id: &str, name: &str, ty: Type) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty,
        constraints: Vec::<Constraint>::new(),
        meta: meta_with_title(title_from_name(name)),
    }
}

fn class_attribute(attribute_id: &str, required: bool, ui_order: Option<u32>) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef {
            id: attribute_id.to_string(),
        },
        required,
        ui_order,
        computed: None,
        constraints: Vec::new(),
        meta: meta_with_title(title_from_attribute_id(attribute_id)),
    }
}

fn string_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn email_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: Some(StringFormat::Email),
        normalization: None,
    }))
}

fn meta_with_title(title: impl Into<String>) -> Meta {
    Meta {
        title: Some(title.into()),
        ..Meta::default()
    }
}

fn title_from_attribute_id(attribute_id: &str) -> String {
    title_from_name(
        attribute_id
            .rsplit(['.', ':'])
            .next()
            .unwrap_or(attribute_id),
    )
}

fn title_from_name(name: &str) -> String {
    name.split('_')
        .map(title_word)
        .collect::<Vec<String>>()
        .join(" ")
}

fn title_word(word: &str) -> String {
    match word {
        "id" => "ID".to_string(),
        "uri" => "URI".to_string(),
        "url" => "URL".to_string(),
        "ui" => "UI".to_string(),
        "email" => "Email".to_string(),
        _ => {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_include_init_migration() {
        let migrations = super::migrations();

        assert_eq!(migrations.len(), 1);
        assert_eq!(migrations[0].module, MODULE_NAME);
        assert_eq!(migrations[0].name, INIT_MIGRATION_NAME);
    }

    #[test]
    fn init_migration_uses_initial_schema_snapshot() {
        let migration = super::init_migration();

        assert_eq!(
            migration_upsert_attribute_ids(&migration),
            vec![ATTR_USERNAME, ATTR_PRIMARY_EMAIL]
        );

        let class = migration_upsert_class(&migration);
        assert_eq!(class.id, USER_CLASS_ID);
        assert_eq!(
            class
                .attributes
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "created_at",
                "description",
                "primary_email",
                "updated_at",
                "username"
            ]
        );
        assert!(class.attributes["username"].required);
        assert!(class.attributes["primary_email"].required);
        assert!(!class.attributes["description"].required);
        assert!(!class.attributes["created_at"].required);
        assert!(!class.attributes["updated_at"].required);

        assert_eq!(
            migration_upsert_collection_names(&migration),
            vec![AUTH_COLLECTION]
        );
    }

    #[test]
    fn user_class_has_expected_fields() {
        let class = user_class();

        assert_eq!(class.id, USER_CLASS_ID);
        assert!(class.attributes["username"].required);
        assert!(class.attributes["primary_email"].required);
        assert!(!class.attributes["description"].required);
        assert!(!class.attributes["created_at"].required);
        assert!(!class.attributes["updated_at"].required);
        assert_eq!(
            class.attributes["description"].attribute.id,
            ATTR_DESCRIPTION
        );
        assert_eq!(class.attributes["created_at"].attribute.id, ATTR_CREATED_AT);
        assert_eq!(class.attributes["updated_at"].attribute.id, ATTR_UPDATED_AT);
    }

    #[test]
    fn primary_email_uses_email_format() {
        let attribute = primary_email_attribute();
        let TypeKind::String(string_type) = attribute.ty.kind else {
            panic!("primary_email should be a string");
        };

        assert_eq!(string_type.format, Some(StringFormat::Email));
    }

    fn migration_upsert_attribute_ids(migration: &Migration) -> Vec<&str> {
        migration
            .operations
            .iter()
            .filter_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute }) => {
                    Some(attribute.id.as_str())
                }
                _ => None,
            })
            .collect()
    }

    fn migration_upsert_class(migration: &Migration) -> &ClassType {
        migration
            .operations
            .iter()
            .find_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }) => {
                    Some(class)
                }
                _ => None,
            })
            .expect("migration should upsert user class")
    }

    fn migration_upsert_collection_names(migration: &Migration) -> Vec<&str> {
        migration
            .operations
            .iter()
            .filter_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
                    name, ..
                }) => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }
}
