use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, Constraint, ListType, MapType, Meta, NumberType,
    StringFormat, StringType, TemporalType, Type, TypeKind, UIntWidth,
};

pub fn string_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

pub fn string_format_type(format: StringFormat) -> Type {
    Type::new(TypeKind::String(StringType {
        format: Some(format),
        normalization: None,
    }))
}

pub fn uint64_type() -> Type {
    Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)))
}

pub fn date_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::Date))
}

pub fn instant_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::Instant))
}

pub fn json_type() -> Type {
    Type::new(TypeKind::Json)
}

pub fn list_type(items: Type) -> Type {
    Type::new(TypeKind::List(ListType {
        items: Box::new(items),
    }))
}

pub fn map_string_string_type() -> Type {
    Type::new(TypeKind::Map(MapType {
        keys: Box::new(string_type()),
        values: Box::new(string_type()),
        ordered: true,
    }))
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
    title_from_name(attribute_id.rsplit('.').next().unwrap_or(attribute_id))
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
        _ => {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{title_from_attribute_id, title_from_name};

    #[test]
    fn builds_titles_from_attribute_names() {
        assert_eq!(title_from_name("display_name"), "Display Name");
        assert_eq!(title_from_name("image_uri"), "Image URI");
        assert_eq!(title_from_name("urls"), "URLs");
    }

    #[test]
    fn builds_titles_from_attribute_ids() {
        assert_eq!(
            title_from_attribute_id("semantic.base.file.content_hash"),
            "Content Hash"
        );
    }
}
