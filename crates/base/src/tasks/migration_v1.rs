use crate::{content::ATTR_MAIN_CONTENT, migration_support_v1 as helpers};
use semantic_data::{
    attr::{ATTR_CREATED_AT, ATTR_PARENT, ATTR_TITLE, ATTR_UPDATED_AT},
    schema::*,
    value::SemanticType,
};

pub const CLASS_ID: &str = "semantic:tasks:task";
pub const ATTR_STATUS: &str = "semantic:tasks:task:status";
pub const ATTR_PRIORITY: &str = "semantic:tasks:task:priority";
pub const ATTR_PROGRESS: &str = "semantic:tasks:task:progress";
pub const ATTR_DUE_DATE: &str = "semantic:tasks:task:due_date";
pub const ATTR_ARCHIVED: &str = "semantic:tasks:task:archived";
pub fn attributes() -> Vec<AttributeType> {
    vec![
        helpers::attribute(
            ATTR_STATUS,
            "status",
            string_enum(&[
                "backlog",
                "todo",
                "in_progress",
                "blocked",
                "done",
                "canceled",
            ]),
        ),
        helpers::attribute(
            ATTR_PRIORITY,
            "priority",
            string_enum(&["none", "low", "medium", "high", "urgent"]),
        ),
        progress_attribute(),
        helpers::attribute(ATTR_DUE_DATE, "due_date", helpers::date_type()),
        helpers::attribute(ATTR_ARCHIVED, "archived", bool::semantic_type()),
    ]
}
pub fn classes() -> Vec<ClassType> {
    vec![crate::migration_support_v1::class(
        CLASS_ID,
        "Task",
        &[
            ("title", ATTR_TITLE, true),
            ("main_content", ATTR_MAIN_CONTENT, true),
            ("status", ATTR_STATUS, true),
            ("priority", ATTR_PRIORITY, true),
            ("progress", ATTR_PROGRESS, true),
            ("due_date", ATTR_DUE_DATE, false),
            ("parent", ATTR_PARENT, false),
            ("archived", ATTR_ARCHIVED, true),
            ("created_at", ATTR_CREATED_AT, true),
            ("updated_at", ATTR_UPDATED_AT, true),
        ],
    )]
}

fn string_enum(names: &[&str]) -> Type {
    Type::new(TypeKind::Enum(EnumType {
        repr: EnumRepr::String,
        variants: names
            .iter()
            .map(|name| EnumVariant {
                name: (*name).into(),
                value: None,
                symbol: Some((*name).into()),
                meta: Meta::default(),
            })
            .collect(),
    }))
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
    Migration {
        module: "tasks".into(),
        name: "001_tasks".into(),
        description: Some("Add tasks".into()),
        operations,
        meta: Meta::default(),
    }
}

fn progress_attribute() -> AttributeType {
    let mut attribute = helpers::attribute(ATTR_PROGRESS, "progress", u64::semantic_type());
    attribute
        .ty
        .constraints
        .push(Constraint::Max(NumberBound::Inclusive("100".into())));
    attribute
}
