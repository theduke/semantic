#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Charset {
    Utf8,
    Utf16,
    Ascii,
    Latin1,
    Custom(String),
}
