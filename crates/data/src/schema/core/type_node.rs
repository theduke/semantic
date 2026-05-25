use crate::schema::core::type_kind::TypeKind;

/// The main schema type node: shape + constraints + metadata + annotations.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub constraints: Vec<crate::schema::constraints::constraint::Constraint>,
    pub annotations: Vec<crate::schema::core::annotation::Annotation>,
}

impl Type {
    pub fn new(kind: TypeKind) -> Self {
        Self {
            kind,
            constraints: Vec::new(),
            annotations: Vec::new(),
        }
    }

    pub fn new_bool() -> Self {
        Self::new(TypeKind::Bool(crate::schema::BoolType))
    }
}

impl From<TypeKind> for Type {
    fn from(value: TypeKind) -> Self {
        Self::new(value)
    }
}
