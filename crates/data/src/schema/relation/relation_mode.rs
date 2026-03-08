#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum RelationMode {
    Embedded { attribute: String },
    External,
}
