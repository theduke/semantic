#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct DecimalType {
    pub precision: Option<u32>,
    pub scale: Option<i32>,
    pub encoding: crate::schema::primitives::decimal_encoding::DecimalEncoding,
}
