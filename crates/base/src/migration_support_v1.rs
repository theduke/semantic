//! Frozen construction helpers for content/tasks/comments initial migrations.
use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, Constraint, EntityRef, ListType, Meta, StringType,
    TemporalType, Type, TypeKind,
};

pub fn string_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

pub fn date_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::Date))
}

pub fn list_type(items: Type) -> Type {
    Type::new(TypeKind::List(ListType {
        items: Box::new(items),
    }))
}

pub fn ref_type(name: &str) -> Type {
    Type::new(TypeKind::Ref(EntityRef::new(name)))
}

pub fn attribute(id: &str, name: &str, ty: Type) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty,
        constraints: Vec::<Constraint>::new(),
        meta: meta_with_title(title_from_name(name)),
    }
}

pub fn class_attribute_with_ui_order(
    attribute_id: &str,
    required: bool,
    ui_order: Option<u32>,
) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef {
            id: attribute_id.to_string(),
        },
        required,
        ui_order,
        computed: None,
        default: None,
        constraints: Vec::new(),
        meta: meta_with_title(title_from_attribute_id(attribute_id)),
    }
}

pub fn meta_with_title(title: impl Into<String>) -> Meta {
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

pub(super) fn class(
    id: &str,
    name: &str,
    attrs: &[(&str, &str, bool)],
) -> semantic_data::schema::ClassType {
    use semantic_data::schema::*;
    ClassType {
        id: id.into(),
        name: name.into(),
        inherits: None,
        extends: vec![],
        strict_schema: false,
        creatable_in_ui: Some(false),
        include_in_ui_listings: None,
        attributes: attrs
            .iter()
            .enumerate()
            .map(|(i, (name, id, required))| {
                (
                    (*name).into(),
                    class_attribute_with_ui_order(id, *required, Some(i as u32 * 10)),
                )
            })
            .collect(),
        constraints: vec![],
        meta: Meta::default(),
    }
}
