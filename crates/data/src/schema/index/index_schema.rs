#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct IndexSchema {
    pub id: String,
    pub name: String,
    pub kind: crate::schema::index::IndexKind,
    pub collection: String,
    pub key_path: crate::schema::collections::key_path::KeyPath,
    pub unique: bool,
}
