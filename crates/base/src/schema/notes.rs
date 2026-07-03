use std::collections::BTreeMap;

use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, Constraint, EnumRepr, EnumType,
    EnumVariant, Type, TypeKind,
};

use super::common::helpers;

pub const CLASS_ID: &str = "semantic:base:note";
pub const ATTR_NOTE_FORMAT: &str = "semantic:base:note:note_format";
pub const ATTR_NOTE_CONTENT: &str = "semantic:base:note:note_content";

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
        attributes: BTreeMap::from([
            (
                "note_format".to_string(),
                class_attribute(ATTR_NOTE_FORMAT, true, 10),
            ),
            (
                "note_content".to_string(),
                class_attribute(ATTR_NOTE_CONTENT, true, 20),
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
        constraints: Vec::new(),
        meta: helpers::meta_with_title(match attribute_id {
            ATTR_NOTE_FORMAT => "Note Format",
            ATTR_NOTE_CONTENT => "Note Content",
            _ => attribute_id,
        }),
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
    use semantic_data::schema::{EnumRepr, TypeKind};

    use super::{ATTR_NOTE_CONTENT, ATTR_NOTE_FORMAT, FORMAT_MARKDOWN, FORMAT_TEXT};

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
    }
}
