use async_trait::async_trait;
use semantic_data::jobs::*;

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct JobStoreError {
    pub message: String,
    /// The operation may have committed. It may only be retried when settled.
    pub outcome_unknown: bool,
    pub settled: bool,
}
impl JobStoreError {
    pub fn definitive(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            outcome_unknown: false,
            settled: true,
        }
    }
    pub fn unknown(message: impl Into<String>, settled: bool) -> Self {
        Self {
            message: message.into(),
            outcome_unknown: true,
            settled,
        }
    }
}

/// Scope-bound single-writer metadata storage. An error must describe whether a
/// write has settled; an adapter must never detach a still-running write.
#[async_trait]
pub trait JobStore: Send + Sync + 'static {
    async fn initialize(&self) -> Result<(), JobStoreError>;
    async fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError>;
    async fn put(&self, record: &JobRecord) -> Result<(), JobStoreError>;
    async fn list(&self, query: JobListQuery) -> Result<JobListPage, JobStoreError>;
    async fn count(&self) -> Result<u64, JobStoreError>;
    async fn delete_ids(&self, ids: &[JobId]) -> Result<u64, JobStoreError>;
}
