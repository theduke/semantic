use crate::schema::collections::key_path::KeyPath;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct IndexSchema {
    pub id: String,
    pub name: String,
    pub kind: crate::schema::index::IndexKind,
    pub collection: String,
    /// The first (for single-column indexes the only) key column.
    pub key_path: KeyPath,
    pub unique: bool,
    /// Key columns after [`Self::key_path`] of a composite index, in key
    /// order. Empty for single-column indexes (and in schemas written before
    /// composite indexes existed).
    #[facet(default)]
    #[facet(skip_serializing_if = Vec::is_empty)]
    pub extra_key_paths: Vec<KeyPath>,
    /// Predicate of a partial index: only rows matching it are indexed.
    /// `None` indexes every row.
    #[facet(default)]
    #[facet(skip_serializing_if = Option::is_none)]
    pub predicate: Option<crate::query::Expr>,
    /// Tokenization of a full-text index (default for other kinds).
    #[facet(default)]
    #[facet(skip_serializing_if = crate::query::TextAnalyzer::is_default)]
    pub analyzer: crate::query::TextAnalyzer,
}

impl IndexSchema {
    /// All key columns in key order.
    pub fn key_paths(&self) -> impl Iterator<Item = &KeyPath> {
        std::iter::once(&self.key_path).chain(self.extra_key_paths.iter())
    }

    /// Whether the index has more than one key column.
    pub fn is_composite(&self) -> bool {
        !self.extra_key_paths.is_empty()
    }

    /// Whether the index only covers rows matching a predicate.
    pub fn is_partial(&self) -> bool {
        self.predicate.is_some()
    }
}
