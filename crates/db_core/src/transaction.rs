/// Isolation of a write transaction.
///
/// Guarantees of the embedded engine (`EmbeddedDb`): every attempt reads at
/// one storage revision and commits only if the storage is still at that
/// revision (a revision-conditional commit on storages with conflict
/// detection), retrying conflicts per [`ConflictPolicy`]. So no level loses
/// updates, and an attempt that observed a concurrent commit never commits.
/// The levels differ in how the reads of an attempt are served:
#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum IsolationLevel {
    /// Reads observe committed data only. They use a consistent snapshot
    /// when the storage offers one and otherwise read the latest state,
    /// fenced by revision checks where the read path supports it.
    ReadCommitted,
    /// Like [`Self::Snapshot`].
    RepeatableRead,
    /// Every read of an attempt is served from a consistent storage snapshot
    /// at the attempt's read revision, so repeated reads return the same
    /// rows. Fails with an `Unsupported` storage error when the storage
    /// cannot provide consistent snapshots.
    Snapshot,
    /// Snapshot reads plus the revision-conditional commit, validated over
    /// the whole database: an attempt commits only if no other transaction
    /// committed since its read revision, so committed transactions are
    /// equivalent to their serial execution in commit order. Additionally
    /// requires a storage with commit-time conflict detection.
    Serializable,
}

impl IsolationLevel {
    /// Whether all reads of a transaction attempt must come from one
    /// consistent snapshot.
    pub fn requires_snapshot(self) -> bool {
        !matches!(self, Self::ReadCommitted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Fail,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TransactionConcurrency {
    StoreDefault,
    Optimistic,
    Mvcc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub struct TransactionOptions {
    pub isolation: IsolationLevel,
    pub conflict_policy: ConflictPolicy,
    pub concurrency: TransactionConcurrency,
    pub read_only: bool,
    pub max_retries: u32,
}

impl Default for TransactionOptions {
    fn default() -> Self {
        Self {
            isolation: IsolationLevel::ReadCommitted,
            conflict_policy: ConflictPolicy::Retry,
            concurrency: TransactionConcurrency::StoreDefault,
            read_only: false,
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub struct TransactionMetrics {
    pub attempts: u32,
    pub conflicts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct TransactionResult<T> {
    pub value: T,
    pub metrics: TransactionMetrics,
}

pub trait TransactionError: std::error::Error {
    fn is_conflict(&self) -> bool;
    fn is_retryable(&self) -> bool;
}

pub fn run_with_transaction_retries<T, E, F>(
    options: TransactionOptions,
    mut operation: F,
) -> Result<TransactionResult<T>, E>
where
    E: TransactionError,
    F: FnMut(u32) -> Result<T, E>,
{
    let mut attempts = 0u32;
    let mut conflicts = 0u32;

    loop {
        attempts += 1;
        match operation(attempts) {
            Ok(value) => {
                return Ok(TransactionResult {
                    value,
                    metrics: TransactionMetrics {
                        attempts,
                        conflicts,
                    },
                });
            }
            Err(err) => {
                let can_retry = err.is_conflict()
                    && err.is_retryable()
                    && options.conflict_policy == ConflictPolicy::Retry
                    && attempts <= options.max_retries;
                if can_retry {
                    conflicts += 1;
                    continue;
                }
                return Err(err);
            }
        }
    }
}

/// A savepoint of an interactive transaction (see
/// `EmbeddedTransaction::savepoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, facet::Facet)]
pub struct SavepointId(pub u64);

/// Result of committing an interactive transaction.
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct TransactionCommit {
    /// Storage revision created by the commit; the transaction's read
    /// revision when it wrote nothing.
    pub revision: Option<u64>,
    /// Rows written by the transaction, including cascade deletes.
    pub stats: crate::BatchStats,
    /// Execution counters of the commit (empty for backends that do not
    /// collect them).
    #[facet(default)]
    pub metrics: crate::WriteMetrics,
}
