#[derive(facet::Facet, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum IndexKind {
    Equality,
    Range,
    FullText,
}
