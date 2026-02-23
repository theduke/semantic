#[derive(Debug, Clone, Copy, PartialEq, Eq, facet::Facet)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum IsolationLevel {
    ReadCommitted,
    RepeatableRead,
    Snapshot,
    Serializable,
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
