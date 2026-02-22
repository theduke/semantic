#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct OptionalType {
    pub inner: Box<crate::schema::core::type_node::Type>,
}
