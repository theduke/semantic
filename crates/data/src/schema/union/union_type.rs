#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct UnionType {
    pub variants: Vec<crate::schema::core::type_node::Type>,
}
