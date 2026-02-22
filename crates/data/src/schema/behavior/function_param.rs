#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct FunctionParam {
    pub name: Option<String>,
    pub ty: crate::schema::core::type_node::Type,
}
