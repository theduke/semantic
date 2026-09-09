use crate::schema::AttributeType;

pub const ATTR_TITLE: &str = "semantic:title";

pub const ATTR_URL: &str = "semantic:url";

/// Shared URL attribute installed by the core schema package.
pub fn url_attribute() -> AttributeType {
    use crate::schema::{Meta, StringFormat, StringType, Type, TypeKind};

    AttributeType {
        id: ATTR_URL.to_string(),
        name: "url".to_string(),
        ty: Type::new(TypeKind::String(StringType {
            format: Some(StringFormat::Url),
            normalization: None,
        })),
        constraints: Vec::new(),
        meta: Meta {
            title: Some("URL".to_string()),
            ..Meta::default()
        },
    }
}

pub trait AttrDescriptor {
    fn attr_schema(&self) -> AttributeType;
}

pub trait AttrDescriptorConst: AttrDescriptor {
    const ID: &'static str;
    const PLAIN_NAME: &'static str;
}

//
// pub struct AttrFieldFormat;

pub const ATTR_UI_CREATABLE_IN_UI: &str = "semantic:ui:creatable_in_ui";

pub fn creatable_in_ui_attribute() -> AttributeType {
    AttributeType {
        id: ATTR_UI_CREATABLE_IN_UI.to_string(),
        name: "creatable_in_ui".to_string(),
        ty: crate::schema::Type::new(crate::schema::TypeKind::Bool(crate::schema::BoolType)),
        constraints: Vec::new(),
        meta: crate::schema::Meta::default(),
    }
}
