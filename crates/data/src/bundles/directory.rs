use std::collections::BTreeMap;

use crate::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassRef, ClassType, Constraint, Meta, Migration,
    MigrationCollectionKind, MigrationDdlOperation, MigrationIntegrityMode, MigrationOperation,
    NumberType, RelationIndexingMode, RelationMode, RelationType, StringType, TemporalType, Type,
    TypeKind, TypeRef, UIntWidth,
};

pub const MODULE_NAME: &str = "base";
pub const DIRECTORIES_MIGRATION_NAME: &str = "002_directories";
pub const ENTITIES_COLLECTION: &str = "entities";

pub const DIRECTORY_CLASS_ID: &str = "semantic:base:directory";
pub const DIRECTORY_NODE_CLASS_ID: &str = "semantic:base:directory_node";
pub const DIRECTORY_NODE_RELATION_ID: &str = "semantic:base:directory_node";

pub const ATTR_TITLE: &str = "semantic:title";
pub const ATTR_DESCRIPTION: &str = "semantic:description";
pub const ATTR_CREATED_AT: &str = "semantic:created_at";
pub const ATTR_UPDATED_AT: &str = "semantic:updated_at";
pub const ATTR_DIRECTORY_NODE_FROM: &str = "semantic:base:directory_node:from";
pub const ATTR_DIRECTORY_NODE_ORDER: &str = "semantic:base:directory_node:order";

const RELATION_CLASS_ID: &str = "semantic:relation";
const ATTR_RELATION_RELATION: &str = "semantic:relation:relation";
const ATTR_RELATION_TO: &str = "semantic:relation:to";

pub fn attributes() -> Vec<AttributeType> {
    vec![
        title_attribute(),
        description_attribute(),
        created_at_attribute(),
        updated_at_attribute(),
        directory_node_from_attribute(),
        directory_node_order_attribute(),
    ]
}

pub fn classes() -> Vec<ClassType> {
    vec![directory_class(), directory_node_class()]
}

pub fn relationships() -> Vec<RelationType> {
    vec![directory_node_relationship()]
}

pub fn migrations() -> Vec<Migration> {
    vec![migration()]
}

pub fn migration() -> Migration {
    let mut operations = Vec::new();

    for attribute in migration_attributes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute { attribute },
        ));
    }

    for class in migration_classes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertClass { class },
        ));
    }

    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertCollection {
            name: ENTITIES_COLLECTION.to_string(),
            kind: MigrationCollectionKind::Polymorphic,
            integrity_mode: MigrationIntegrityMode::StrictRegisteredSchema,
        },
    ));

    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertRelationship {
            relationship: migration_directory_node_relationship(),
        },
    ));

    Migration {
        module: MODULE_NAME.to_string(),
        name: DIRECTORIES_MIGRATION_NAME.to_string(),
        description: Some("Add directory and directory node schema.".to_string()),
        operations,
        meta: Meta::default(),
    }
}

pub fn directory_class() -> ClassType {
    ClassType {
        id: DIRECTORY_CLASS_ID.to_string(),
        name: "Directory".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "title".to_string(),
                class_attribute(ATTR_TITLE, true, Some(10)),
            ),
            (
                "description".to_string(),
                class_attribute(ATTR_DESCRIPTION, false, Some(20)),
            ),
            (
                "created_at".to_string(),
                class_attribute(ATTR_CREATED_AT, false, Some(30)),
            ),
            (
                "updated_at".to_string(),
                class_attribute(ATTR_UPDATED_AT, false, Some(40)),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta_with_title("Directory"),
    }
}

pub fn directory_node_class() -> ClassType {
    ClassType {
        id: DIRECTORY_NODE_CLASS_ID.to_string(),
        name: "DirectoryNode".to_string(),
        inherits: Some(ClassRef {
            id: RELATION_CLASS_ID.to_string(),
        }),
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "relation".to_string(),
                class_attribute(ATTR_RELATION_RELATION, true, Some(10)),
            ),
            (
                "from".to_string(),
                class_attribute(ATTR_DIRECTORY_NODE_FROM, true, Some(20)),
            ),
            (
                "to".to_string(),
                class_attribute(ATTR_RELATION_TO, true, Some(30)),
            ),
            (
                "order".to_string(),
                class_attribute(ATTR_DIRECTORY_NODE_ORDER, false, Some(40)),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta_with_title("Directory Node"),
    }
}

pub fn title_attribute() -> AttributeType {
    attribute(ATTR_TITLE, "title", string_type())
}

pub fn description_attribute() -> AttributeType {
    attribute(ATTR_DESCRIPTION, "description", string_type())
}

pub fn created_at_attribute() -> AttributeType {
    attribute(ATTR_CREATED_AT, "created_at", datetime_type())
}

pub fn updated_at_attribute() -> AttributeType {
    attribute(ATTR_UPDATED_AT, "updated_at", datetime_type())
}

pub fn directory_node_from_attribute() -> AttributeType {
    attribute(
        ATTR_DIRECTORY_NODE_FROM,
        "from",
        ref_type(DIRECTORY_CLASS_ID),
    )
}

pub fn directory_node_order_attribute() -> AttributeType {
    attribute(ATTR_DIRECTORY_NODE_ORDER, "order", uint64_type())
}

pub fn directory_node_relationship() -> RelationType {
    RelationType {
        id: DIRECTORY_NODE_RELATION_ID.to_string(),
        name: "directory_node".to_string(),
        source_collection: ENTITIES_COLLECTION.to_string(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Enabled,
        meta: meta_with_title("Directory Node"),
    }
}

fn migration_attributes() -> Vec<AttributeType> {
    vec![
        migration_attribute(ATTR_TITLE, "title", migration_string_type(), "Title"),
        migration_attribute(
            ATTR_DESCRIPTION,
            "description",
            migration_string_type(),
            "Description",
        ),
        migration_attribute(
            ATTR_CREATED_AT,
            "created_at",
            migration_datetime_type(),
            "Created At",
        ),
        migration_attribute(
            ATTR_UPDATED_AT,
            "updated_at",
            migration_datetime_type(),
            "Updated At",
        ),
        migration_attribute(
            ATTR_DIRECTORY_NODE_FROM,
            "from",
            migration_ref_type(DIRECTORY_CLASS_ID),
            "From",
        ),
        migration_attribute(
            ATTR_DIRECTORY_NODE_ORDER,
            "order",
            migration_uint64_type(),
            "Order",
        ),
    ]
}

fn migration_classes() -> Vec<ClassType> {
    vec![
        migration_directory_class(),
        migration_directory_node_class(),
    ]
}

fn migration_directory_class() -> ClassType {
    ClassType {
        id: DIRECTORY_CLASS_ID.to_string(),
        name: "Directory".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "title".to_string(),
                migration_class_attribute(ATTR_TITLE, true, Some(10), "Title"),
            ),
            (
                "description".to_string(),
                migration_class_attribute(ATTR_DESCRIPTION, false, Some(20), "Description"),
            ),
            (
                "created_at".to_string(),
                migration_class_attribute(ATTR_CREATED_AT, false, Some(30), "Created At"),
            ),
            (
                "updated_at".to_string(),
                migration_class_attribute(ATTR_UPDATED_AT, false, Some(40), "Updated At"),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta_with_title("Directory"),
    }
}

fn migration_directory_node_class() -> ClassType {
    ClassType {
        id: DIRECTORY_NODE_CLASS_ID.to_string(),
        name: "DirectoryNode".to_string(),
        inherits: Some(ClassRef {
            id: RELATION_CLASS_ID.to_string(),
        }),
        extends: Vec::new(),
        attributes: BTreeMap::from([
            (
                "relation".to_string(),
                migration_class_attribute(ATTR_RELATION_RELATION, true, Some(10), "Relation"),
            ),
            (
                "from".to_string(),
                migration_class_attribute(ATTR_DIRECTORY_NODE_FROM, true, Some(20), "From"),
            ),
            (
                "to".to_string(),
                migration_class_attribute(ATTR_RELATION_TO, true, Some(30), "To"),
            ),
            (
                "order".to_string(),
                migration_class_attribute(ATTR_DIRECTORY_NODE_ORDER, false, Some(40), "Order"),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta_with_title("Directory Node"),
    }
}

fn migration_directory_node_relationship() -> RelationType {
    RelationType {
        id: DIRECTORY_NODE_RELATION_ID.to_string(),
        name: "directory_node".to_string(),
        source_collection: ENTITIES_COLLECTION.to_string(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Enabled,
        meta: meta_with_title("Directory Node"),
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

fn migration_datetime_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::DateTime))
}

fn migration_uint64_type() -> Type {
    Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)))
}

fn migration_ref_type(name: &str) -> Type {
    Type::new(TypeKind::Ref(TypeRef {
        name: name.to_string(),
        args: Vec::new(),
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

fn datetime_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::DateTime))
}

fn uint64_type() -> Type {
    Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)))
}

fn ref_type(name: &str) -> Type {
    Type::new(TypeKind::Ref(TypeRef {
        name: name.to_string(),
        args: Vec::new(),
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
        "urls" => "URLs".to_string(),
        "ui" => "UI".to_string(),
        "mime" => "MIME".to_string(),
        "sha256" => "SHA256".to_string(),
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
    fn migrations_include_directory_migration() {
        let migrations = super::migrations();

        assert_eq!(migrations.len(), 1);
        assert_eq!(migrations[0].module, MODULE_NAME);
        assert_eq!(migrations[0].name, DIRECTORIES_MIGRATION_NAME);
    }

    #[test]
    fn migration_uses_directory_schema_snapshot() {
        let migration = super::migration();

        assert_eq!(
            migration_upsert_attribute_ids(&migration),
            vec![
                ATTR_TITLE,
                ATTR_DESCRIPTION,
                ATTR_CREATED_AT,
                ATTR_UPDATED_AT,
                ATTR_DIRECTORY_NODE_FROM,
                ATTR_DIRECTORY_NODE_ORDER,
            ]
        );

        let classes = migration_upsert_classes(&migration);
        assert_eq!(
            classes
                .iter()
                .map(|class| class.id.as_str())
                .collect::<Vec<_>>(),
            vec![DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID]
        );
        assert!(classes[0].attributes["title"].required);
        assert!(classes[1].attributes["from"].required);
        assert!(!classes[1].attributes["order"].required);

        assert_eq!(
            migration_upsert_collection_names(&migration),
            vec![ENTITIES_COLLECTION]
        );
        assert_eq!(
            migration_upsert_relationship_ids(&migration),
            vec![DIRECTORY_NODE_RELATION_ID]
        );
    }

    #[test]
    fn directory_node_from_attribute_references_directory_class() {
        let attribute = directory_node_from_attribute();
        let TypeKind::Ref(type_ref) = attribute.ty.kind else {
            panic!("directory node from attribute should be a ref");
        };

        assert_eq!(type_ref.name, DIRECTORY_CLASS_ID);
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

    fn migration_upsert_classes(migration: &Migration) -> Vec<&ClassType> {
        migration
            .operations
            .iter()
            .filter_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }) => {
                    Some(class)
                }
                _ => None,
            })
            .collect()
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

    fn migration_upsert_relationship_ids(migration: &Migration) -> Vec<&str> {
        migration
            .operations
            .iter()
            .filter_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertRelationship {
                    relationship,
                }) => Some(relationship.id.as_str()),
                _ => None,
            })
            .collect()
    }
}
