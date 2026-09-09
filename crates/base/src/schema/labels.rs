//! Label definitions and entity-to-label membership relations.
use std::collections::BTreeMap;

use semantic_data::{builtin::DEFAULT_COLLECTION, schema::*};

use super::common::helpers;
pub use super::common::person::ATTR_PARENT;
pub use semantic_data::bundles::directory::{ATTR_CREATED_AT, ATTR_DESCRIPTION, ATTR_UPDATED_AT};

pub const CLASS_ID: &str = "semantic:base:label";
pub const GROUP_CLASS_ID: &str = "semantic:base:label_group";
pub const ATTR_NAME: &str = "semantic:base:label:name";
pub const ATTR_COLOR: &str = "semantic:base:label:color";
/// Controls selection of this label's direct children. Missing means multiple.
pub const ATTR_SELECTION_MODE: &str = "semantic:base:label:selection_mode";
pub const MODE_MULTIPLE: &str = "multiple";
pub const MODE_EXCLUSIVE: &str = "exclusive";
pub const RELATION_ID: &str = "semantic:base:entity_label";
pub const ATTR_ENTITY_COLLECTION: &str = "semantic:base:entity_label:collection";
pub const ATTR_RELATION: &str = "semantic:relation:relation";
pub const ATTR_FROM: &str = "semantic:relation:from";
pub const ATTR_TO: &str = "semantic:relation:to";

pub fn attributes() -> Vec<AttributeType> {
    vec![
        helpers::attribute(ATTR_NAME, "label_name", helpers::string_type()),
        helpers::attribute(ATTR_COLOR, "label_color", helpers::string_type()),
        helpers::attribute(
            ATTR_ENTITY_COLLECTION,
            "entity_collection",
            helpers::string_type(),
        ),
        helpers::attribute(
            ATTR_SELECTION_MODE,
            "selection_mode",
            Type::new(TypeKind::Enum(EnumType {
                repr: EnumRepr::String,
                variants: [MODE_MULTIPLE, MODE_EXCLUSIVE]
                    .into_iter()
                    .map(|name| EnumVariant {
                        name: name.into(),
                        value: None,
                        symbol: None,
                        meta: helpers::meta_with_title(name),
                    })
                    .collect(),
            })),
        ),
    ]
}

pub fn classes() -> Vec<ClassType> {
    let mut classes = legacy_classes();
    // Selection mode belongs to virtual groups. Retain the original Label definition
    // in migration 006 so databases that have already applied it can still upgrade.
    classes[0].attributes.remove("selection_mode");
    classes.push(group_class());
    classes
}

pub fn group_class() -> ClassType {
    let mut class = legacy_classes().remove(0);
    class.id = GROUP_CLASS_ID.into();
    class.name = "LabelGroup".into();
    class.meta = helpers::meta_with_title("Label group");
    class
}

/// Frozen class definitions recorded by migration 006.
fn legacy_classes() -> Vec<ClassType> {
    vec![
        make_class(
            CLASS_ID,
            "Label",
            None,
            &[
                ("name", ATTR_NAME, true),
                ("description", ATTR_DESCRIPTION, false),
                ("parent", ATTR_PARENT, false),
                ("color", ATTR_COLOR, false),
                ("selection_mode", ATTR_SELECTION_MODE, false),
                ("created_at", ATTR_CREATED_AT, false),
                ("updated_at", ATTR_UPDATED_AT, false),
            ],
        ),
        make_class(
            RELATION_ID,
            "EntityLabel",
            Some("semantic:relation"),
            &[
                ("relation", ATTR_RELATION, true),
                ("from", ATTR_FROM, true),
                ("to", ATTR_TO, true),
                ("entity_collection", ATTR_ENTITY_COLLECTION, true),
            ],
        ),
    ]
}

fn make_class(
    id: &str,
    name: &str,
    parent: Option<&str>,
    attrs: &[(&str, &str, bool)],
) -> ClassType {
    ClassType {
        id: id.into(),
        name: name.into(),
        inherits: parent.map(|id| ClassRef { id: id.into() }),
        extends: vec![],
        strict_schema: false,
        creatable_in_ui: None,
        attributes: attrs
            .iter()
            .enumerate()
            .map(|(i, (name, id, required))| {
                (
                    (*name).into(),
                    helpers::class_attribute_with_ui_order(id, *required, Some(i as u32 * 10)),
                )
            })
            .collect::<BTreeMap<_, _>>(),
        constraints: vec![],
        meta: helpers::meta_with_title(name),
    }
}

pub fn relationship() -> RelationType {
    RelationType {
        id: RELATION_ID.into(),
        name: "entity_label".into(),
        source_collection: DEFAULT_COLLECTION.into(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Enabled,
        meta: helpers::meta_with_title("Entity label"),
    }
}

pub fn migration() -> Migration {
    let mut operations: Vec<_> = attributes()
        .into_iter()
        .map(|attribute| {
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute })
        })
        .collect();
    operations.extend(
        legacy_classes()
            .into_iter()
            .map(|class| MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class })),
    );
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertRelationship {
            relationship: relationship(),
        },
    ));
    Migration {
        module: crate::MODULE_NAME.into(),
        name: "006_labels".into(),
        description: Some("Add hierarchical labels and entity memberships.".into()),
        operations,
        meta: Meta::default(),
    }
}

/// Introduce virtual groups without dropping historical assignments. Existing
/// exclusive parents become groups; their old direct assignments remain available
/// for explicit removal through the label editor or remove-labels command.
pub fn group_migration() -> Migration {
    use semantic_data::query::{BinaryOp, Expr, Operand, UpdateQuery};
    use semantic_data::value::{FieldPath, Value};

    let equals = |field: &str, value: &str| Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            field,
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(Value::String(value.into())))),
    };
    let query = UpdateQuery::new()
        .with_collection(DEFAULT_COLLECTION)
        .with_predicate(Expr::Binary {
            op: BinaryOp::And,
            left: Box::new(equals("type", CLASS_ID)),
            right: Box::new(equals(ATTR_SELECTION_MODE, MODE_EXCLUSIVE)),
        })
        .set(
            FieldPath::from_fields(["type"]),
            Expr::Operand(Operand::Literal(Value::String(GROUP_CLASS_ID.into()))),
        );
    Migration {
        module: crate::MODULE_NAME.into(),
        name: "007_label_groups".into(),
        description: Some("Add nonselectable label groups and convert exclusive parents while preserving their memberships.".into()),
        operations: vec![
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class: classes().remove(0) }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class: group_class() }),
            MigrationOperation::Update { query },
        ],
        meta: Meta::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::value::{Object, Value};

    #[test]
    fn group_schema_keeps_metadata_optional_and_old_migration_unchanged() {
        let original = migration();
        let original_classes: Vec<_> = original
            .operations
            .iter()
            .filter_map(|op| match op {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }) => {
                    Some(class)
                }
                _ => None,
            })
            .collect();
        assert_eq!(original.name, "006_labels");
        assert_eq!(original_classes.len(), 2);
        assert_eq!(original_classes[0].id, CLASS_ID);
        assert_eq!(original_classes[1].id, RELATION_ID);
        assert!(
            original_classes[0]
                .attributes
                .contains_key("selection_mode")
        );
        assert!(
            !classes()
                .iter()
                .find(|class| class.id == CLASS_ID)
                .unwrap()
                .attributes
                .contains_key("selection_mode")
        );
        for class in classes()
            .into_iter()
            .filter(|class| class.id != RELATION_ID)
        {
            assert!(class.attributes["name"].required);
            assert!(!class.attributes["description"].required);
            assert!(!class.attributes["color"].required);
        }
        assert!(group_class().attributes.contains_key("selection_mode"));
        assert_eq!(group_migration().name, "007_label_groups");
    }

    #[tokio::test]
    async fn group_upgrade_preserves_hierarchy_and_legacy_assignments() {
        let db = semantic_db_core::Db::new(semantic_db_core::embedded::EmbeddedBackend::new(
            semantic_db_kv::open_memory().unwrap(),
        ));
        let mut original = crate::package();
        let group_index = original
            .migrations
            .iter()
            .position(|migration| migration.name == "007_label_groups")
            .unwrap();
        original.migrations.truncate(group_index);
        // Reconstruct the pre-group classes from their recorded definitions so
        // later, unrelated package migrations cannot change this upgrade fixture.
        original.root.classes = original
            .migrations
            .iter()
            .flat_map(|migration| &migration.operations)
            .filter_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }) => {
                    Some((class.id.clone(), class.clone()))
                }
                _ => None,
            })
            .collect();
        db.upsert_package(original).await.unwrap();

        let object = |fields: &[(&str, &str)]| {
            let mut object = Object::new();
            for (field, value) in fields {
                object.insert(*field, Value::String((*value).into()));
            }
            object
        };
        for (id, value) in [
            (
                "status",
                object(&[
                    ("id", "status"),
                    ("type", CLASS_ID),
                    (ATTR_NAME, "Status"),
                    (ATTR_SELECTION_MODE, MODE_EXCLUSIVE),
                ]),
            ),
            (
                "todo",
                object(&[
                    ("id", "todo"),
                    ("type", CLASS_ID),
                    (ATTR_NAME, "Todo"),
                    (ATTR_PARENT, "status"),
                ]),
            ),
            (
                "subject",
                object(&[
                    ("id", "subject"),
                    ("type", crate::schema::common::person::CLASS_ID),
                ]),
            ),
            (
                "assignment",
                object(&[
                    ("id", "assignment"),
                    ("type", RELATION_ID),
                    (ATTR_RELATION, RELATION_ID),
                    (ATTR_FROM, "subject"),
                    (ATTR_TO, "status"),
                    (ATTR_ENTITY_COLLECTION, DEFAULT_COLLECTION),
                ]),
            ),
        ] {
            db.insert(DEFAULT_COLLECTION, id, value).await.unwrap();
        }
        db.upsert_package(crate::package()).await.unwrap();
        let group = db
            .get(DEFAULT_COLLECTION, "status")
            .await
            .unwrap()
            .unwrap()
            .object;
        assert_eq!(
            group.get("type").and_then(Value::as_str),
            Some(GROUP_CLASS_ID)
        );
        assert_eq!(
            group.get(ATTR_SELECTION_MODE).and_then(Value::as_str),
            Some(MODE_EXCLUSIVE)
        );
        let child = db
            .get(DEFAULT_COLLECTION, "todo")
            .await
            .unwrap()
            .unwrap()
            .object;
        assert_eq!(child.get("type").and_then(Value::as_str), Some(CLASS_ID));
        assert_eq!(
            child.get(ATTR_PARENT).and_then(Value::as_str),
            Some("status")
        );
        let membership = db
            .get(DEFAULT_COLLECTION, "assignment")
            .await
            .unwrap()
            .unwrap()
            .object;
        assert_eq!(
            membership.get(ATTR_TO).and_then(Value::as_str),
            Some("status")
        );
        // Reopening/re-registering never changes the original migration or repeats data loss.
        db.upsert_package(crate::package()).await.unwrap();
        assert!(
            db.get(DEFAULT_COLLECTION, "assignment")
                .await
                .unwrap()
                .is_some()
        );
    }
}
