//! Database maintenance operations: index rebuilds, consistency checks and
//! repairs, physical compaction, payload rewrites and backups.
//!
//! The operations are implemented by the embedded database
//! ([`crate::embedded::EmbeddedDb`]) and exposed through [`crate::Backend`]
//! and [`crate::Db`]; backends without them report an `Unsupported` storage
//! error. See `docs/maintenance.md` for an overview.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::catalog::Catalog;
use crate::embedded::StorageStats;
use crate::{DbError, EntityStream, StorageErrorKind};

/// Indexes rebuilt by a reindex.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ReindexTarget {
    /// Every index, including the indexes of internal collections, plus the
    /// derived reverse references and relationship edges.
    #[default]
    All,
    /// Every index of one collection.
    Collection(String),
    /// One index of a collection, by index name.
    Index { collection: String, name: String },
}

/// One index rebuilt by a reindex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReindexedIndex {
    pub collection: String,
    pub index: String,
    /// Rows the index was rebuilt from.
    pub rows: u64,
    /// Entries of the rebuilt index, when the storage maintains entry
    /// counts.
    pub entries: Option<u64>,
}

/// Outcome of a reindex.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReindexReport {
    pub indexes: Vec<ReindexedIndex>,
    /// Derived data rebuilt from the rows (see [`DerivedData`]).
    pub derived: Vec<DerivedData>,
    pub duration: Duration,
}

/// Internally maintained data derived from the rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DerivedData {
    /// Reverse references (incoming references per target entity).
    ReverseReferences,
    /// Relationship contributors, counts and (transitive) edges.
    RelationshipEdges,
    /// Maintained row and index entry counters.
    Stats,
}

impl DerivedData {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReverseReferences => "reverse_references",
            Self::RelationshipEdges => "relationship_edges",
            Self::Stats => "stats",
        }
    }
}

impl fmt::Display for DerivedData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Checks run by a verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyOptions {
    /// Compare every index with the entries derived from the rows (missing
    /// and stale entries).
    pub check_indexes: bool,
    /// Compare the reverse references with the references of the rows.
    pub check_reverse_references: bool,
    /// Compare relationship contributors, counts and edges with the rows.
    pub check_relationship_edges: bool,
    /// Compare maintained row and index entry counters with actual counts.
    pub check_stats: bool,
    /// Run the storage's physical integrity check. Needs exclusive access to
    /// the storage (see [`crate::embedded::EntityStorage::check_storage_integrity`]).
    pub check_storage_integrity: bool,
    /// Decode every stored payload.
    pub check_payloads: bool,
    /// Maximum number of problems listed in the report; further problems
    /// are only counted.
    pub max_problems: usize,
}

impl VerifyOptions {
    /// Default limit of listed problems.
    pub const DEFAULT_MAX_PROBLEMS: usize = 1000;

    /// Every check.
    pub fn all() -> Self {
        Self {
            check_indexes: true,
            check_reverse_references: true,
            check_relationship_edges: true,
            check_stats: true,
            check_storage_integrity: true,
            check_payloads: true,
            max_problems: Self::DEFAULT_MAX_PROBLEMS,
        }
    }

    /// No check; enable individual checks on the result.
    pub fn none() -> Self {
        Self {
            check_indexes: false,
            check_reverse_references: false,
            check_relationship_edges: false,
            check_stats: false,
            check_storage_integrity: false,
            check_payloads: false,
            max_problems: Self::DEFAULT_MAX_PROBLEMS,
        }
    }
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self::all()
    }
}

/// Outcome of the physical storage integrity check of a verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageIntegrityCheck {
    /// The storage is intact.
    Intact,
    /// The storage found and repaired damage.
    Repaired,
    /// The storage reported unrecoverable corruption.
    Corrupt(String),
    /// The check could not run (unsupported, or concurrent readers).
    Skipped(String),
}

/// Kind of an inconsistency found by a verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerifyProblemKind {
    /// An index is not marked as built.
    IndexNotBuilt,
    /// An entry derived from a row is missing from its index.
    MissingIndexEntry,
    /// An index holds an entry no row derives.
    StaleIndexEntry,
    /// A reverse reference derived from a row is missing or differs.
    MissingReverseReference,
    /// A stored reverse reference no row derives.
    StaleReverseReference,
    /// A relationship contributor derived from a row is missing.
    MissingRelationshipContributor,
    /// A stored relationship contributor no row derives.
    StaleRelationshipContributor,
    /// A relationship count differs from the number of contributors.
    WrongRelationshipCount,
    /// A relationship edge derived from the rows is missing or differs.
    MissingRelationshipEdge,
    /// A stored relationship edge the rows do not derive.
    StaleRelationshipEdge,
    /// Derived data was never initialized (missing completion marker).
    DerivedDataNotInitialized,
    /// A maintained row counter differs from the number of rows.
    WrongRowCount,
    /// A maintained index entry counter differs from the number of entries.
    WrongIndexEntryCount,
    /// A stored payload cannot be decoded.
    CorruptPayload,
    /// The storage's physical integrity check failed or repaired the storage.
    StorageIntegrity,
    /// Two rows hold the same value in a unique index. A repair cannot fix
    /// this; one of the rows must be changed or deleted.
    DuplicateUniqueValue,
}

impl VerifyProblemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IndexNotBuilt => "index_not_built",
            Self::MissingIndexEntry => "missing_index_entry",
            Self::StaleIndexEntry => "stale_index_entry",
            Self::MissingReverseReference => "missing_reverse_reference",
            Self::StaleReverseReference => "stale_reverse_reference",
            Self::MissingRelationshipContributor => "missing_relationship_contributor",
            Self::StaleRelationshipContributor => "stale_relationship_contributor",
            Self::WrongRelationshipCount => "wrong_relationship_count",
            Self::MissingRelationshipEdge => "missing_relationship_edge",
            Self::StaleRelationshipEdge => "stale_relationship_edge",
            Self::DerivedDataNotInitialized => "derived_data_not_initialized",
            Self::WrongRowCount => "wrong_row_count",
            Self::WrongIndexEntryCount => "wrong_index_entry_count",
            Self::CorruptPayload => "corrupt_payload",
            Self::StorageIntegrity => "storage_integrity",
            Self::DuplicateUniqueValue => "duplicate_unique_value",
        }
    }
}

impl fmt::Display for VerifyProblemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One inconsistency found by a verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyProblem {
    pub kind: VerifyProblemKind,
    pub collection: Option<String>,
    pub index: Option<String>,
    pub entity_id: Option<String>,
    pub detail: String,
}

impl fmt::Display for VerifyProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(collection) = &self.collection {
            write!(f, " collection={collection}")?;
        }
        if let Some(index) = &self.index {
            write!(f, " index={index}")?;
        }
        if let Some(id) = &self.entity_id {
            write!(f, " id={id:?}")?;
        }
        if !self.detail.is_empty() {
            write!(f, ": {}", self.detail)?;
        }
        Ok(())
    }
}

/// What a verify looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VerifyCounts {
    pub collections: u64,
    pub rows: u64,
    pub indexes: u64,
    pub index_entries: u64,
    pub reverse_references: u64,
    pub relationship_edges: u64,
    pub counters: u64,
}

/// Outcome of a verify.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerifyReport {
    /// Storage revision the checks observed.
    pub revision: Option<u64>,
    /// Problems found, at most [`VerifyOptions::max_problems`].
    pub problems: Vec<VerifyProblem>,
    /// Number of problems found, including those not listed.
    pub problem_count: u64,
    pub checked: VerifyCounts,
    /// Checks that could not run, with the reason.
    pub skipped: Vec<String>,
    pub duration: Duration,
}

impl VerifyReport {
    /// Whether no problem was found.
    pub fn is_ok(&self) -> bool {
        self.problem_count == 0
    }
}

impl fmt::Display for VerifyReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let checked = &self.checked;
        writeln!(
            f,
            "verified revision {}: {} collections, {} rows, {} indexes ({} entries), \
             {} reverse references, {} relationship edges, {} counters in {:.3}s",
            display_revision(self.revision),
            checked.collections,
            checked.rows,
            checked.indexes,
            checked.index_entries,
            checked.reverse_references,
            checked.relationship_edges,
            checked.counters,
            self.duration.as_secs_f64(),
        )?;
        for skipped in &self.skipped {
            writeln!(f, "skipped: {skipped}")?;
        }
        for problem in &self.problems {
            writeln!(f, "problem: {problem}")?;
        }
        let unlisted = self.problem_count - self.problems.len() as u64;
        if unlisted > 0 {
            writeln!(f, "... and {unlisted} more problems")?;
        }
        write!(f, "{} problems", self.problem_count)
    }
}

/// Outcome of a repair.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepairReport {
    /// Verify report before the repair.
    pub before: VerifyReport,
    /// Indexes and derived data rebuilt by the repair.
    pub rebuilt: ReindexReport,
    /// Verify report after the repair; problems the repair cannot fix (for
    /// example corrupt payloads) remain.
    pub after: VerifyReport,
}

impl fmt::Display for RepairReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "found {} problems", self.before.problem_count)?;
        writeln!(f, "{}", self.rebuilt)?;
        write!(f, "{}", self.after)
    }
}

impl fmt::Display for ReindexReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for index in &self.indexes {
            write!(
                f,
                "reindexed {}.{} from {} rows",
                index.collection, index.index, index.rows
            )?;
            if let Some(entries) = index.entries {
                write!(f, " ({entries} entries)")?;
            }
            writeln!(f)?;
        }
        for derived in &self.derived {
            writeln!(f, "rebuilt {derived}")?;
        }
        write!(
            f,
            "rebuilt {} indexes and {} derived structures in {:.3}s",
            self.indexes.len(),
            self.derived.len(),
            self.duration.as_secs_f64()
        )
    }
}

/// Outcome of a storage compaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompactReport {
    pub before: StorageStats,
    pub after: StorageStats,
    /// Whether the storage compacted anything.
    pub compacted: bool,
    /// Reduction of the file size, when the storage reports file sizes.
    pub freed_bytes: Option<u64>,
    pub duration: Duration,
}

impl fmt::Display for CompactReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "compacted: {}; file size {} -> {} bytes",
            self.compacted,
            display_option(self.before.file_size_bytes),
            display_option(self.after.file_size_bytes),
        )?;
        if let Some(freed) = self.freed_bytes {
            write!(f, "; freed {freed} bytes")?;
        }
        write!(f, " in {:.3}s", self.duration.as_secs_f64())
    }
}

/// Default number of rows per write transaction of a payload rewrite.
pub const DEFAULT_REWRITE_BATCH_SIZE: usize = 1000;

/// Outcome of a payload rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RewriteReport {
    /// Stored payloads examined.
    pub scanned: u64,
    /// Payloads rewritten in the current format.
    pub rewritten: u64,
    /// Write transactions committed.
    pub batches: u64,
    pub duration: Duration,
}

impl fmt::Display for RewriteReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rewrote {} of {} payloads in {} batches in {:.3}s",
            self.rewritten,
            self.scanned,
            self.batches,
            self.duration.as_secs_f64()
        )
    }
}

/// Outcome of a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub path: PathBuf,
    /// Storage revision of the copied state.
    pub revision: Option<u64>,
    /// Copied key-value entries.
    pub entries: u64,
    /// Size of the backup file.
    pub bytes: Option<u64>,
    pub duration: Duration,
}

impl fmt::Display for BackupReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "backed up revision {} ({} entries, {} bytes) to {} in {:.3}s",
            display_revision(self.revision),
            self.entries,
            display_option(self.bytes),
            self.path.display(),
            self.duration.as_secs_f64()
        )
    }
}

/// A consistent export of all portable entities at one revision.
pub struct ExportSnapshot {
    /// Storage revision the stream reads.
    pub revision: Option<u64>,
    /// Version of [`Self::catalog`] in the database's shared catalog.
    pub catalog_version: u64,
    /// Catalog the entities were written under.
    pub catalog: Arc<Catalog>,
    /// Every entity of the non-internal collections at [`Self::revision`].
    pub stream: EntityStream,
}

impl fmt::Debug for ExportSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExportSnapshot")
            .field("revision", &self.revision)
            .field("catalog_version", &self.catalog_version)
            .finish_non_exhaustive()
    }
}

/// Human-readable storage statistics.
pub fn format_storage_stats(stats: &StorageStats) -> String {
    let mut out = format!(
        "file size: {} bytes\nentries: {}\nallocated: {} bytes\nstored: {} bytes\nfragmented: {} bytes",
        display_option(stats.file_size_bytes),
        display_option(stats.entries),
        display_option(stats.allocated_bytes),
        display_option(stats.stored_bytes),
        display_option(stats.fragmented_bytes),
    );
    for table in &stats.tables {
        out.push_str(&format!(
            "\ntable {}: {} entries",
            table.name, table.entries
        ));
    }
    out
}

/// Error returned by backends without the maintenance `operation`.
pub fn unsupported_maintenance(operation: &str) -> DbError {
    DbError::storage(
        StorageErrorKind::Unsupported,
        format!("{operation} is not supported by this backend"),
    )
}

fn display_option(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_string(), |value| value.to_string())
}

fn display_revision(revision: Option<u64>) -> String {
    display_option(revision)
}
