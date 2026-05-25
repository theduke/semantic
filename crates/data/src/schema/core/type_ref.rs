use crate::schema::core::type_name::TypeName;

/// Reference to a named type definition, with optional type arguments.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct TypeRef {
    pub name: TypeName,
    pub args: Vec<crate::schema::core::type_node::Type>,
}

impl TypeRef {
    pub fn new(name: impl Into<TypeName>) -> Self {
        Self {
            name: name.into(),
            args: Vec::new(),
        }
    }
}
