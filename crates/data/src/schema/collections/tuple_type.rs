#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct TupleType {
    pub items: Vec<crate::schema::core::type_node::Type>,
    pub rest: Option<Box<crate::schema::core::type_node::Type>>,
}
