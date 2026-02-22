use crate::schema::core::type_name::TypeName;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct TypeParam {
    pub name: TypeName,
    pub bounds: Vec<crate::schema::core::type_ref::TypeRef>,
    pub default: Option<crate::schema::core::type_node::Type>,
}
