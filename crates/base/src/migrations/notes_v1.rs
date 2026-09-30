//! Frozen definitions recorded by the original Notes migration.

use std::collections::BTreeMap;

use semantic_data::{
    expr::{CallExpr, Callee, Expr},
    schema::{
        AttributeRef, AttributeType, ClassAttribute, ClassType, Constraint, EnumRepr, EnumType,
        EnumVariant, Meta, StringType, Type, TypeKind,
    },
};

const NOTE_FORMAT: &str = "semantic:base:note:note_format";
const NOTE_CONTENT: &str = "semantic:base:note:note_content";

pub(super) fn attributes() -> Vec<AttributeType> {
    vec![
        AttributeType {
            id: NOTE_FORMAT.to_string(),
            name: "note_format".to_string(),
            ty: Type::new(TypeKind::Enum(EnumType {
                repr: EnumRepr::String,
                variants: ["text", "markdown"]
                    .into_iter()
                    .map(|name| EnumVariant {
                        name: name.to_string(),
                        value: None,
                        symbol: None,
                        meta: meta(match name {
                            "text" => "Text",
                            "markdown" => "Markdown",
                            _ => unreachable!(),
                        }),
                    })
                    .collect(),
            })),
            constraints: Vec::new(),
            meta: meta("Note Format"),
        },
        AttributeType {
            id: NOTE_CONTENT.to_string(),
            name: "note_content".to_string(),
            ty: Type::new(TypeKind::String(StringType {
                format: None,
                normalization: None,
            })),
            constraints: Vec::new(),
            meta: meta("Note Content"),
        },
    ]
}

pub(super) fn class() -> ClassType {
    ClassType {
        id: "semantic:base:note".to_string(),
        name: "Note".to_string(),
        inherits: None,
        extends: Vec::new(),
        strict_schema: false,
        creatable_in_ui: None,
        include_in_ui_listings: None,
        attributes: BTreeMap::from([
            (
                "title".to_string(),
                field("semantic:title", "Title", false, 5, false),
            ),
            (
                "note_format".to_string(),
                field(NOTE_FORMAT, "Note Format", true, 10, false),
            ),
            (
                "note_content".to_string(),
                field(NOTE_CONTENT, "Note Content", true, 20, false),
            ),
            (
                "created_at".to_string(),
                field("semantic:created_at", "Created At", false, 30, true),
            ),
            (
                "updated_at".to_string(),
                field("semantic:updated_at", "Updated At", false, 40, true),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta("Note"),
    }
}

fn field(id: &str, title: &str, required: bool, ui_order: u32, timestamp: bool) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef { id: id.to_string() },
        required,
        ui_order: Some(ui_order),
        computed: None,
        default: None,
        constraints: if timestamp {
            vec![Constraint::DefaultExpr {
                expr: Expr::Call(Box::new(CallExpr {
                    callee: Callee::Name(vec!["time".to_string(), "now".to_string()]),
                    args: Vec::new(),
                    over: None,
                })),
            }]
        } else {
            Vec::new()
        },
        meta: meta(title),
    }
}

fn meta(title: &str) -> Meta {
    Meta {
        title: Some(title.to_string()),
        ..Meta::default()
    }
}
