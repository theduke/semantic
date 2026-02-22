#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TimeZoneSpec {
    Required,
    Forbidden,
    Allowed,
    Specific(String),
}
