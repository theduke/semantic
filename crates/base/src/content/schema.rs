use crate::schema::common::helpers;
use semantic_data::schema::*;
pub const ATTR_MAIN_CONTENT: &str = "semantic:base:main_content";
pub fn content_type() -> Type {
    Type::new(TypeKind::Variant(VariantType {
        tag: VariantTag::AdjacentlyTagged {
            tag_field: "kind".into(),
            data_field: "data".into(),
        },
        variants: vec![VariantCase {
            name: "note".into(),
            payload: VariantPayload::Newtype(Box::new(Type::new(TypeKind::Class(
                crate::schema::notes::class(),
            )))),
            discriminant: None,
            meta: Meta::default(),
        }],
    }))
}
pub fn attribute() -> AttributeType {
    helpers::attribute(ATTR_MAIN_CONTENT, "main_content", content_type())
}
