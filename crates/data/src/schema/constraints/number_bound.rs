#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum NumberBound {
    Inclusive(String),
    Exclusive(String),
}
