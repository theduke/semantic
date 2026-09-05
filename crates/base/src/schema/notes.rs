use std::collections::BTreeMap;

use semantic_data::expr::{CallExpr, Callee, Expr};
use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, Constraint, EnumRepr, EnumType,
    EnumVariant, Type, TypeKind,
};

use super::common::helpers;

pub const CLASS_ID: &str = "semantic:base:note";
pub const ATTR_NOTE_FORMAT: &str = "semantic:base:note:note_format";
pub const ATTR_NOTE_CONTENT: &str = "semantic:base:note:note_content";
pub const ATTR_CREATED_AT: &str = semantic_data::bundles::directory::ATTR_CREATED_AT;
pub const ATTR_UPDATED_AT: &str = semantic_data::bundles::directory::ATTR_UPDATED_AT;

pub const FORMAT_TEXT: &str = "text";
pub const FORMAT_MARKDOWN: &str = "markdown";

pub fn attributes() -> Vec<AttributeType> {
    vec![
        note_format_attribute(),
        helpers::attribute(ATTR_NOTE_CONTENT, "note_content", helpers::string_type()),
    ]
}

pub fn class() -> ClassType {
    ClassType {
        id: CLASS_ID.to_string(),
        name: "Note".to_string(),
        inherits: None,
        extends: Vec::new(),
        strict_schema: false,
        attributes: BTreeMap::from([
            (
                "note_format".to_string(),
                class_attribute(ATTR_NOTE_FORMAT, true, 10),
            ),
            (
                "note_content".to_string(),
                class_attribute(ATTR_NOTE_CONTENT, true, 20),
            ),
            (
                "created_at".to_string(),
                class_attribute(ATTR_CREATED_AT, false, 30),
            ),
            (
                "updated_at".to_string(),
                class_attribute(ATTR_UPDATED_AT, false, 40),
            ),
        ]),
        constraints: Vec::new(),
        meta: helpers::meta_with_title("Note"),
    }
}

pub fn note_format_attribute() -> AttributeType {
    AttributeType {
        id: ATTR_NOTE_FORMAT.to_string(),
        name: "note_format".to_string(),
        ty: note_format_type(),
        constraints: Vec::<Constraint>::new(),
        meta: helpers::meta_with_title("Note Format"),
    }
}

pub fn note_format_type() -> Type {
    Type::new(TypeKind::Enum(EnumType {
        repr: EnumRepr::String,
        variants: [FORMAT_TEXT, FORMAT_MARKDOWN]
            .into_iter()
            .map(enum_variant)
            .collect(),
    }))
}

fn class_attribute(attribute_id: &str, required: bool, ui_order: u32) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef {
            id: attribute_id.to_string(),
        },
        required,
        ui_order: Some(ui_order),
        computed: None,
        constraints: match attribute_id {
            ATTR_CREATED_AT | ATTR_UPDATED_AT => vec![now_default_constraint()],
            _ => Vec::new(),
        },
        meta: helpers::meta_with_title(match attribute_id {
            ATTR_NOTE_FORMAT => "Note Format",
            ATTR_NOTE_CONTENT => "Note Content",
            ATTR_CREATED_AT => "Created At",
            ATTR_UPDATED_AT => "Updated At",
            _ => attribute_id,
        }),
    }
}

fn now_default_constraint() -> Constraint {
    Constraint::DefaultExpr {
        expr: Expr::Call(Box::new(CallExpr {
            callee: Callee::Name(vec!["time".to_string(), "now".to_string()]),
            args: Vec::new(),
            over: None,
        })),
    }
}

fn enum_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        value: None,
        symbol: None,
        meta: helpers::meta_with_title(match name {
            FORMAT_TEXT => "Text",
            FORMAT_MARKDOWN => "Markdown",
            _ => name,
        }),
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::expr::{Callee, Expr};
    use semantic_data::schema::{EnumRepr, TypeKind};

    use super::{
        ATTR_CREATED_AT, ATTR_NOTE_CONTENT, ATTR_NOTE_FORMAT, ATTR_UPDATED_AT, FORMAT_MARKDOWN,
        FORMAT_TEXT,
    };

    #[test]
    fn note_format_is_string_enum() {
        let attribute = super::note_format_attribute();
        let TypeKind::Enum(enum_type) = attribute.ty.kind else {
            panic!("expected note format enum");
        };

        assert_eq!(enum_type.repr, EnumRepr::String);
        assert_eq!(
            enum_type
                .variants
                .iter()
                .map(|variant| variant.name.as_str())
                .collect::<Vec<_>>(),
            vec![FORMAT_TEXT, FORMAT_MARKDOWN]
        );
    }

    #[test]
    fn note_class_declares_required_attributes() {
        let class = super::class();

        assert_eq!(
            class.attributes["note_format"].attribute.id,
            ATTR_NOTE_FORMAT
        );
        assert_eq!(
            class.attributes["note_content"].attribute.id,
            ATTR_NOTE_CONTENT
        );
        assert!(class.attributes["note_format"].required);
        assert!(class.attributes["note_content"].required);

        assert_eq!(class.attributes["created_at"].attribute.id, ATTR_CREATED_AT);
        assert_eq!(class.attributes["updated_at"].attribute.id, ATTR_UPDATED_AT);
        assert!(!class.attributes["created_at"].required);
        assert!(!class.attributes["updated_at"].required);

        for attribute_name in ["created_at", "updated_at"] {
            let [
                semantic_data::schema::Constraint::DefaultExpr {
                    expr: Expr::Call(call),
                },
            ] = class.attributes[attribute_name].constraints.as_slice()
            else {
                panic!("{attribute_name} should default to an expression");
            };
            assert_eq!(
                call.callee,
                Callee::Name(vec!["time".to_string(), "now".to_string()])
            );
            assert!(call.args.is_empty());
        }
    }
}
