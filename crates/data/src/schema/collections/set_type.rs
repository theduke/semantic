#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct SetType {
    pub items: Box<crate::schema::core::type_node::Type>,
}
