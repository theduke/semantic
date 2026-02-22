#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ArrayType {
    pub items: Box<crate::schema::core::type_node::Type>,
    pub length: Option<crate::schema::constraints::length_spec::LengthSpec>,
}
