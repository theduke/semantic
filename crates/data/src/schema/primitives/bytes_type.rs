#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct BytesType {
    pub encoding: Option<crate::schema::primitives::bytes_encoding::BytesEncoding>,
}
