#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum StringFormat {
    Email,
    Uri,
    Url,
    Hostname,
    Regex,
    Uuid,
    Base64,
    Hex,
    Ascii,
    Utf8,
    JsonPointer,
    JsonPath,
    Sql,
    Custom(String),
}
