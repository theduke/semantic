//! Durable cleanup intent. Recording intent never authorizes deleting bytes.
use super::*;

pub(super) fn attributes() -> Vec<AttributeType> {
    [
        ("store", string_type()),
        ("locator", string_type()),
        ("not_before", datetime_type()),
        (
            "attempts",
            Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U32))),
        ),
        ("last_error", string_type()),
    ]
    .into_iter()
    .map(|(name, ty)| attribute(&format!("{CLEANUP_CLASS_ID}:{name}"), name, ty))
    .collect()
}

pub(super) fn class() -> ClassType {
    let attributes = attributes()
        .into_iter()
        .map(|attribute| {
            let mut field = class_attribute_with_ui_order(&attribute.id, false, None);
            field.required = attribute.name != "last_error";
            (attribute.name, field)
        })
        .collect();
    ClassType {
        id: CLEANUP_CLASS_ID.into(),
        name: "Cleanup".into(),
        inherits: None,
        extends: Vec::new(),
        strict_schema: true,
        creatable_in_ui: Some(false),
        attributes,
        constraints: Vec::new(),
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
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass { class: class() },
    ));
    Migration {
        module: MODULE_NAME.into(),
        name: "008_cleanup_intent".into(),
        description: Some(
            "Persist native file cleanup intent atomically with metadata deletion.".into(),
        ),
        operations,
        meta: Meta::default(),
    }
}
