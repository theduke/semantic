#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct IntersectionType {
    pub variants: Vec<crate::schema::core::type_node::Type>,
}
