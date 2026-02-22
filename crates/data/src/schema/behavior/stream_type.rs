#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct StreamType {
    pub element: Box<crate::schema::core::type_node::Type>,
    pub end: Option<Box<crate::schema::core::type_node::Type>>,
}
