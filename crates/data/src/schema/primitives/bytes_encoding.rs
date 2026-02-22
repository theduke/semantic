#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BytesEncoding {
    Raw,
    Base64,
    Base64Url,
    Hex,
    Ascii85,
}
