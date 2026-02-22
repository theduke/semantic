#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct SchemaImport {
    pub namespace: String,
    pub location: String,
    pub version: Option<crate::schema::core::schema_version::SchemaVersion>,
    pub optional: bool,
}
