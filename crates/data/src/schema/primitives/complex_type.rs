#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct ComplexType {
    pub component: crate::schema::primitives::float_width::FloatWidth,
}
