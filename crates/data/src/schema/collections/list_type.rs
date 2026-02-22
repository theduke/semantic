#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ListType {
    pub items: Box<crate::schema::core::type_node::Type>,
}
