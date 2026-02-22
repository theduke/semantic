#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct StringType {
    pub format: Option<crate::schema::primitives::string_format::StringFormat>,
    pub normalization:
        Option<crate::schema::primitives::unicode_normalization::UnicodeNormalization>,
}
