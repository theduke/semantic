#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct MapType {
    pub keys: Box<crate::schema::core::type_node::Type>,
    pub values: Box<crate::schema::core::type_node::Type>,
    pub ordered: bool,
}
