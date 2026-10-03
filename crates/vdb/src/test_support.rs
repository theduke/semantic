use async_trait::async_trait;
use semantic_jobs::{JobId, JobListPage, JobListQuery, JobRecord, JobStore, JobStoreError};

pub(crate) struct EmptyStore;

#[async_trait]
impl JobStore for EmptyStore {
    async fn initialize(&self) -> Result<(), JobStoreError> {
        Ok(())
    }
    async fn get(&self, _: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
        Ok(None)
    }
    async fn put(&self, _: &JobRecord) -> Result<(), JobStoreError> {
        Ok(())
    }
    async fn list(&self, _: JobListQuery) -> Result<JobListPage, JobStoreError> {
        Ok(JobListPage {
            records: vec![],
            next_cursor: None,
        })
    }
    async fn count(&self) -> Result<u64, JobStoreError> {
        Ok(0)
    }
    async fn delete_ids(&self, _: &[JobId]) -> Result<u64, JobStoreError> {
        Ok(0)
    }
}
