#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyPath {
    pub segments: Vec<String>,
}
