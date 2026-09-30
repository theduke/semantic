use crate::{content::ATTR_MAIN_CONTENT, migration_support_v1 as helpers};
use semantic_data::{
    attr::{
        ATTR_CREATED_AT, ATTR_PARENT, ATTR_RELATION_FROM, ATTR_RELATION_RELATION, ATTR_RELATION_TO,
        ATTR_UPDATED_AT, RELATION_CLASS_ID,
    },
    builtin::DEFAULT_COLLECTION,
    schema::*,
    value::SemanticType,
};
pub const CLASS_ID: &str = "semantic:comments:comment";
pub const RELATION_ID: &str = "semantic:comments:entity_comment";
pub const ATTR_AUTHOR_ID: &str = "semantic:comments:comment:author_id";
pub const ATTR_DELETED: &str = "semantic:comments:comment:deleted";
pub const ATTR_DELETED_AT: &str = "semantic:comments:comment:deleted_at";
pub const ATTR_EDITED_AT: &str = "semantic:comments:comment:edited_at";
pub const ATTR_ENTITY_COLLECTION: &str = "semantic:comments:entity_comment:collection";
pub fn attributes() -> Vec<AttributeType> {
    vec![
        helpers::attribute(ATTR_AUTHOR_ID, "author_id", String::semantic_type()),
        helpers::attribute(ATTR_DELETED, "deleted", bool::semantic_type()),
        helpers::attribute(
            ATTR_DELETED_AT,
            "deleted_at",
            semantic_data::value::DateTime::semantic_type(),
        ),
        helpers::attribute(
            ATTR_EDITED_AT,
            "edited_at",
            semantic_data::value::DateTime::semantic_type(),
        ),
        helpers::attribute(
            ATTR_ENTITY_COLLECTION,
            "entity_collection",
            String::semantic_type(),
        ),
    ]
}
pub fn classes() -> Vec<ClassType> {
    let comment = crate::migration_support_v1::class(
        CLASS_ID,
        "Comment",
        &[
            ("main_content", ATTR_MAIN_CONTENT, true),
            ("author_id", ATTR_AUTHOR_ID, true),
            ("parent", ATTR_PARENT, false),
            ("deleted", ATTR_DELETED, true),
            ("created_at", ATTR_CREATED_AT, true),
            ("updated_at", ATTR_UPDATED_AT, true),
            ("edited_at", ATTR_EDITED_AT, false),
            ("deleted_at", ATTR_DELETED_AT, false),
        ],
    );
    let mut relation = crate::migration_support_v1::class(
        RELATION_ID,
        "EntityComment",
        &[
            ("relation", ATTR_RELATION_RELATION, true),
            ("from", ATTR_RELATION_FROM, true),
            ("to", ATTR_RELATION_TO, true),
            ("entity_collection", ATTR_ENTITY_COLLECTION, true),
        ],
    );
    relation.inherits = Some(ClassRef {
        id: RELATION_CLASS_ID.into(),
    });
    vec![comment, relation]
}
pub fn relationship() -> RelationType {
    RelationType {
        id: RELATION_ID.into(),
        name: "entity_comment".into(),
        source_collection: DEFAULT_COLLECTION.into(),
        mode: RelationMode::External,
        indexing_mode: RelationIndexingMode::Enabled,
        meta: Meta::default(),
    }
}

pub(super) fn migration() -> Migration {
    let mut operations: Vec<_> = attributes()
        .into_iter()
        .map(|attribute| {
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute })
        })
        .collect();
    operations.extend(
        classes()
            .into_iter()
            .map(|class| MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class })),
    );
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertRelationship {
            relationship: relationship(),
        },
    ));
    Migration {
        module: "comments".into(),
        name: "001_comments".into(),
        description: Some("Add generic comments".into()),
        operations,
        meta: Meta::default(),
    }
}
