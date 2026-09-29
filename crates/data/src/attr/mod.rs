use crate::schema::AttributeType;
use crate::value::{DateTime, SemanticType};

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

/// Describes an attribute; implemented by marker types declared with [`attr!`].
pub trait AttrDescriptor {
    fn attr_schema() -> AttributeType;
}

pub trait AttrDescriptorConst: AttrDescriptor {
    /// The qualified attribute id, like `semantic:created_at`.
    const ID: &'static str;
    /// The unqualified name, accepted as an alias of [`Self::ID`].
    const PLAIN_NAME: &'static str;
    /// The Rust type of the attribute's values.
    type Value: SemanticType;
}

/// Declares a unit marker struct for an attribute, implementing
/// [`AttrDescriptor`] and [`AttrDescriptorConst`]. Fields of derived structs
/// refer to it with `#[semantic(attr = Marker)]`.
///
/// ```
/// semantic_data::attr!(
///     /// When the entity was created.
///     pub CreatedAt, "semantic:created_at", semantic_data::DateTime
/// );
/// ```
///
/// The plain name defaults to the last `:` segment of the id; override it with
/// `name = "..."`. The schema defaults to one built from the value's
/// [`SemanticType`]; override it with `schema = expr`.
#[macro_export]
macro_rules! attr {
    (@name $id:expr) => {
        $crate::attr::plain_name($id)
    };
    (@name $id:expr, $name:expr) => {
        $name
    };
    (@schema $marker:ident) => {
        $crate::attr::attr_schema::<$marker>()
    };
    (@schema $marker:ident, $schema:expr) => {
        $schema
    };
    (
        $(#[$meta:meta])*
        $vis:vis $marker:ident, $id:expr, $value:ty
        $(, name = $name:expr)?
        $(, schema = $schema:expr)?
        $(,)?
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        $vis struct $marker;

        impl $crate::attr::AttrDescriptorConst for $marker {
            const ID: &'static str = $id;
            const PLAIN_NAME: &'static str = $crate::attr!(@name $id $(, $name)?);
            type Value = $value;
        }

        impl $crate::attr::AttrDescriptor for $marker {
            fn attr_schema() -> $crate::schema::AttributeType {
                $crate::attr!(@schema $marker $(, $schema)?)
            }
        }
    };
}

/// The last `:` segment of an attribute id.
#[doc(hidden)]
pub const fn plain_name(id: &'static str) -> &'static str {
    let bytes = id.as_bytes();
    let mut start = bytes.len();
    while start > 0 && bytes[start - 1] != b':' {
        start -= 1;
    }
    match std::str::from_utf8(bytes.split_at(start).1) {
        Ok(name) => name,
        Err(_) => panic!("ids split at ':' stay valid UTF-8"),
    }
}

/// The default schema of an attribute marker.
#[doc(hidden)]
pub fn attr_schema<A: AttrDescriptorConst>() -> AttributeType {
    AttributeType {
        id: A::ID.to_string(),
        name: A::PLAIN_NAME.to_string(),
        ty: A::Value::semantic_type(),
        constraints: Vec::new(),
        meta: crate::schema::Meta::default(),
    }
}

crate::attr!(
    /// The built-in entity id.
    pub AttrId, crate::builtin::ATTR_ID, String
);
crate::attr!(
    /// The built-in entity type: the class id.
    pub AttrType, crate::builtin::ATTR_TYPE, String
);
crate::attr!(pub AttrTitle, ATTR_TITLE, String, schema = title_attribute());
crate::attr!(pub AttrDescription, ATTR_DESCRIPTION, String, schema = description_attribute());
crate::attr!(pub AttrCreatedAt, ATTR_CREATED_AT, DateTime, schema = created_at_attribute());
crate::attr!(pub AttrUpdatedAt, ATTR_UPDATED_AT, DateTime, schema = updated_at_attribute());
crate::attr!(pub AttrUrl, ATTR_URL, String, schema = url_attribute());
crate::attr!(
    /// The parent entity id.
    pub AttrParent, ATTR_PARENT, String, schema = parent_attribute()
);

/// The shared parent attribute, as last defined by the shared bundle.
fn parent_attribute() -> AttributeType {
    use crate::schema::{EntityRef, Meta, Type, TypeKind};

    AttributeType {
        id: ATTR_PARENT.to_string(),
        name: "parent".to_string(),
        ty: Type::new(TypeKind::Ref(EntityRef::any())),
        constraints: Vec::new(),
        meta: Meta {
            title: Some("Parent".to_string()),
            ..Meta::default()
        },
    }
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
