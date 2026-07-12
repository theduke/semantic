#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ContractConstant {
    pub name: String,
    pub ty: crate::schema::core::type_node::Type,
    pub value: crate::value::Value,
    pub meta: crate::schema::core::meta::Meta,
}
