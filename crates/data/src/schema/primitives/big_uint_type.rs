#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct BigUIntType {
    pub min_bits: Option<u32>,
    pub max_bits: Option<u32>,
}
