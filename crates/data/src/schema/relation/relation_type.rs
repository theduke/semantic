#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct RelationType {
    pub id: String,
    pub name: String,
    pub source_collection: String,
    pub mode: crate::schema::relation::relation_mode::RelationMode,
    pub indexing_mode: crate::schema::relation::relation_indexing_mode::RelationIndexingMode,
    pub meta: crate::schema::core::meta::Meta,
}
