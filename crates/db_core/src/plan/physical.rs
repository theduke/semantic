use semantic_data::{
    query::{JoinType, SortDirection, TextMatchMode},
    value::{FieldPath, Value},
};

use crate::catalog::{LocalAttrId, LocalCollectionId, LocalFieldId, LocalIndexId};
use crate::query::Expr;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SourceRef {
    pub source_name: Option<String>,
    pub collection_id: Option<LocalCollectionId>,
    pub binding: Option<String>,
    pub backend_tag: Option<String>,
    /// Identifies a source occurrence when distinct scans share a collection
    /// and SQL binding. Federation assigns this after logical optimization;
    /// ordinary embedded plans leave it unset.
    pub occurrence_id: Option<u64>,
}

impl std::fmt::Debug for SourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut source = f.debug_struct("SourceRef");
        source
            .field("source_name", &self.source_name)
            .field("collection_id", &self.collection_id)
            .field("binding", &self.binding)
            .field("backend_tag", &self.backend_tag);
        if let Some(id) = self.occurrence_id {
            source.field("occurrence_id", &id);
        }
        source.finish()
    }
}

impl SourceRef {
    pub fn unnamed() -> Self {
        Self {
            source_name: None,
            collection_id: None,
            binding: None,
            backend_tag: None,
            occurrence_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldRef {
    AttrId(LocalAttrId),
    FieldId(LocalFieldId),
    CanonicalName(String),
    Path(FieldPath),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalProjectionField {
    pub expr: crate::query::Expr,
    pub field: Option<FieldRef>,
    pub source_path: Option<FieldPath>,
    pub alias: Option<String>,
    pub wildcard: Option<FieldPath>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalOrderField {
    pub expr: crate::query::Expr,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalJoinKey {
    pub field: FieldRef,
    pub source_path: FieldPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalJoinAlgorithm {
    Hash,
    IndexNestedLoop,
    NestedLoop,
    Merge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalIndexProbe {
    pub source: SourceRef,
    pub field: FieldRef,
    pub residual_predicate: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalJoinCondition {
    True,
    Predicate(Expr),
    Eq {
        left: PhysicalJoinKey,
        right: PhysicalJoinKey,
        residual_predicate: Option<Expr>,
    },
}

/// Read of one collection through ordered scans of an equality or range
/// index.
///
/// Each range is one contiguous run of index entries: equality on leading
/// key columns and optionally a range (bounds or string prefix) on the next.
/// Several ranges (`IN` probes) are disjoint and listed in ascending key
/// order, so the concatenated scan is ordered by the index key.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalIndexScan {
    pub source: SourceRef,
    pub index: LocalIndexId,
    pub index_name: String,
    /// Canonical paths of the key columns, in key order.
    pub columns: Vec<FieldPath>,
    pub ranges: Vec<crate::IndexScanRange>,
    /// Ascending or descending key order.
    pub direction: SortDirection,
    /// Whether the plan relies on rows arriving in key order: the scan
    /// replaced a sort by the key columns (ties by entity id).
    pub ordered: bool,
    /// Upper bound of the rows the consumer reads (`LIMIT` plus `OFFSET`),
    /// when a `LIMIT` consumes the scan's rows one to one (possibly through
    /// projections). The scan stops once that many rows passed its residual
    /// predicate.
    pub limit_hint: Option<usize>,
    /// Serve rows from the index keys (key columns and id) without reading
    /// stored rows. Rows whose key does not decode losslessly are read.
    pub index_only: bool,
    /// The complete predicate of the source: the rows produced are exactly
    /// the rows of the collection matching it. Fallbacks without index
    /// access filter a scan with it.
    pub predicate: Option<Expr>,
    /// The conjuncts of `predicate` the key ranges do not guarantee, checked
    /// on every row read.
    pub residual_predicate: Option<Expr>,
}

impl PhysicalIndexScan {
    /// The ordering the scan produces, as sort fields over the key columns.
    pub fn order_by(&self) -> Vec<PhysicalOrderField> {
        self.columns
            .iter()
            .map(|path| PhysicalOrderField {
                expr: Expr::Operand(crate::query::Operand::Field(path.clone())),
                direction: self.direction,
            })
            .collect()
    }
}

/// Read of one collection through the token entries of a full-text index.
///
/// Each token is one equality probe of the index. The ids of the probes are
/// intersected (`All`) or united (`Any`), and the rows read by id in id
/// order.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalTextSearch {
    pub source: SourceRef,
    pub index: LocalIndexId,
    pub index_name: String,
    /// Canonical paths of the indexed columns.
    pub columns: Vec<FieldPath>,
    /// The distinct query tokens, in ascending order.
    pub tokens: Vec<String>,
    pub mode: TextMatchMode,
    /// The complete predicate of the source: the rows produced are exactly
    /// the rows of the collection matching it. Fallbacks without index
    /// access filter a scan with it.
    pub predicate: Option<Expr>,
    /// The conjuncts of `predicate` the probes do not guarantee, checked on
    /// every row read.
    pub residual_predicate: Option<Expr>,
    /// Upper bound of the rows the consumer reads, see
    /// [`PhysicalIndexScan::limit_hint`].
    pub limit_hint: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalSource {
    Scan {
        source: SourceRef,
    },
    FilteredScan {
        source: SourceRef,
        predicate: Expr,
    },
    IndexLookup {
        source: SourceRef,
        field: FieldRef,
        value: Value,
        residual_predicate: Option<Expr>,
        /// Upper bound of the rows the consumer reads, see
        /// [`PhysicalIndexScan::limit_hint`].
        limit_hint: Option<usize>,
    },
    /// Ordered, range, prefix, multi-probe or index-only read of an
    /// equality or range index.
    IndexRange(PhysicalIndexScan),
    /// Token probes of a full-text index.
    TextSearch(PhysicalTextSearch),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalJoinPlan {
    pub left: Box<PhysicalPlan>,
    pub right: Box<PhysicalPlan>,
    pub join_type: JoinType,
    pub algorithm: PhysicalJoinAlgorithm,
    pub condition: PhysicalJoinCondition,
    pub index_probe: Option<PhysicalIndexProbe>,
    pub left_binding: String,
    pub right_binding: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalPlan {
    Source(PhysicalSource),
    Values {
        values: Vec<semantic_data::value::Object>,
    },
    Filter {
        input: Box<PhysicalPlan>,
        predicate: Expr,
    },
    Sort {
        input: Box<PhysicalPlan>,
        order_by: Vec<PhysicalOrderField>,
    },
    /// `Sort` fused with the `Limit` above it: keeps only the first
    /// `offset + limit` rows of the ordering in a bounded heap and emits the
    /// `limit` rows after `offset`. Ties keep input order, exactly like a
    /// stable sort followed by the limit.
    TopN {
        input: Box<PhysicalPlan>,
        order_by: Vec<PhysicalOrderField>,
        offset: crate::query::Expr,
        limit: crate::query::Expr,
    },
    Project {
        input: Box<PhysicalPlan>,
        projection: Vec<PhysicalProjectionField>,
    },
    Aggregate {
        input: Box<PhysicalPlan>,
        group_by: Vec<crate::query::Expr>,
        projection: Vec<PhysicalProjectionField>,
        having: Option<Expr>,
    },
    Limit {
        input: Box<PhysicalPlan>,
        offset: crate::query::Expr,
        limit: Option<crate::query::Expr>,
    },
    Distinct {
        input: Box<PhysicalPlan>,
    },
    Union {
        inputs: Vec<PhysicalPlan>,
        all: bool,
    },
    Join(PhysicalJoinPlan),
    ApplyExists {
        input: Box<PhysicalPlan>,
        subquery: Box<PhysicalPlan>,
        negated: bool,
    },
    ApplyInSubquery {
        input: Box<PhysicalPlan>,
        left: crate::query::Expr,
        subquery: Box<PhysicalPlan>,
        negated: bool,
    },
    Exchange {
        input: Box<PhysicalPlan>,
        partition_count: usize,
    },
    RepartitionHash {
        input: Box<PhysicalPlan>,
        partition_count: usize,
        partition_keys: Vec<PhysicalJoinKey>,
    },
    Materialize {
        input: Box<PhysicalPlan>,
    },
}
