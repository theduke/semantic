#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct BigIntType {
    pub min_bits: Option<u32>,
    pub max_bits: Option<u32>,
}
