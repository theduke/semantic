#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct OpaqueType {
    pub id: String,
    pub domain: Option<String>,
    pub repr: Option<String>,
}
