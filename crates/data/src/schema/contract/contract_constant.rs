#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ContractConstant {
    pub name: String,
    pub ty: crate::schema::core::type_node::Type,
    pub value: crate::schema::core::literal_value::LiteralValue,
    pub meta: crate::schema::core::meta::Meta,
}
