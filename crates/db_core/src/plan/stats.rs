use crate::catalog::LocalIndexId;
use crate::plan::{FieldRef, SourceRef};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelationStats {
    pub row_count: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldStats {
    pub distinct_count: Option<f64>,
    pub null_fraction: Option<f64>,
}

pub trait StatsProvider: Send + Sync {
    fn relation_stats(&self, source: &SourceRef) -> Option<RelationStats>;
    fn field_stats(&self, source: &SourceRef, field: &FieldRef) -> Option<FieldStats>;
    fn has_equality_index(&self, source: &SourceRef, field: &FieldRef) -> Option<bool>;

    /// Number of entries of `index`, when known.
    fn index_entry_count(&self, source: &SourceRef, index: LocalIndexId) -> Option<f64> {
        let _ = (source, index);
        None
    }
}
