#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct TypeDef {
    pub name: crate::schema::core::type_name::TypeName,
    pub params: Vec<crate::schema::core::type_param::TypeParam>,
    pub ty: crate::schema::core::type_node::Type,
    pub visibility: crate::schema::core::visibility::Visibility,
    pub meta: crate::schema::core::meta::Meta,
}
