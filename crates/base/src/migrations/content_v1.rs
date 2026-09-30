use crate::migration_support_v1 as helpers;
use semantic_data::schema::*;
const ATTR_MAIN_CONTENT: &str = "semantic:base:main_content";
pub fn content_type() -> Type {
    Type::new(TypeKind::Variant(VariantType {
        tag: VariantTag::AdjacentlyTagged {
            tag_field: "kind".into(),
            data_field: "data".into(),
        },
        variants: vec![VariantCase {
            name: "note".into(),
            payload: VariantPayload::Newtype(Box::new(Type::new(TypeKind::Class(
                super::notes_v2::class(),
            )))),
            discriminant: None,
            meta: Meta::default(),
        }],
    }))
}
pub fn attribute() -> AttributeType {
    helpers::attribute(ATTR_MAIN_CONTENT, "main_content", content_type())
}

pub(super) fn migration() -> Migration {
    Migration {
        module: "base".into(),
        name: "012_main_content".into(),
        description: Some("Add embedded main content".into()),
        operations: vec![MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute {
                attribute: attribute(),
            },
        )],
        meta: Meta::default(),
    }
}
