use crate::schema::AttributeType;

pub const ATTR_TITLE: &str = "semantic:title";
pub const ATTR_DESCRIPTION: &str = "semantic:description";
pub const ATTR_CREATED_AT: &str = "semantic:created_at";
pub const ATTR_UPDATED_AT: &str = "semantic:updated_at";

pub const ATTR_URL: &str = "semantic:url";
pub const ATTR_PARENT: &str = "semantic:parent";

pub const RELATION_CLASS_ID: &str = "semantic:relation";
pub const ATTR_RELATION_RELATION: &str = "semantic:relation:relation";
pub const ATTR_RELATION_FROM: &str = "semantic:relation:from";
pub const ATTR_RELATION_TO: &str = "semantic:relation:to";

pub fn title_attribute() -> AttributeType {
    string_attribute(ATTR_TITLE, "title", "Title")
}

pub fn description_attribute() -> AttributeType {
    string_attribute(ATTR_DESCRIPTION, "description", "Description")
}

pub fn created_at_attribute() -> AttributeType {
    temporal_attribute(ATTR_CREATED_AT, "created_at", "Created At")
}

pub fn updated_at_attribute() -> AttributeType {
    temporal_attribute(ATTR_UPDATED_AT, "updated_at", "Updated At")
}

fn string_attribute(id: &str, name: &str, title: &str) -> AttributeType {
    use crate::schema::{Meta, StringType, Type, TypeKind};

    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty: Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        })),
        constraints: Vec::new(),
        meta: Meta {
            title: Some(title.to_string()),
            ..Meta::default()
        },
    }
}

fn temporal_attribute(id: &str, name: &str, title: &str) -> AttributeType {
    use crate::schema::{Meta, TemporalType, Type, TypeKind};

    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty: Type::new(TypeKind::Temporal(TemporalType::DateTime)),
        constraints: Vec::new(),
        meta: Meta {
            title: Some(title.to_string()),
            ..Meta::default()
        },
    }
}

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
