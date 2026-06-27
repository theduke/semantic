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
        meta: Meta::default(),
    }
}

pub fn class_attribute(attribute_id: &str, required: bool) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef {
            id: attribute_id.to_string(),
        },
        required,
        computed: None,
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}
