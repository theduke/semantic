#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct RationalType {
    pub numerator: Option<Box<crate::schema::primitives::number_type::NumberType>>,
    pub denominator: Option<Box<crate::schema::primitives::number_type::NumberType>>,
}
