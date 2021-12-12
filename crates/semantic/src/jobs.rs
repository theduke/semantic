//! Job system for long-running jobs.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use anyhow::anyhow;
use factordb::{data::Timestamp, AnyError};
use semantic_core::api::{Job, JobId, JobStatus};

#[derive(Clone)]
pub struct JobManager(Arc<RwLock<State>>);

struct State {
    jobs: HashMap<JobId, Job>,
}

impl JobManager {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(State {
            jobs: HashMap::new(),
        })))
    }

    pub fn job(&self, id: JobId) -> Option<Job> {
        self.0.read().unwrap().jobs.get(&id).cloned()
    }

    pub fn register_job(&self, job: Job) {
        self.0.write().unwrap().jobs.insert(job.id, job);
    }

    pub fn update_job(&self, id: JobId, status: JobStatus) -> Result<(), AnyError> {
        self.0
            .write()
            .unwrap()
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Job not found: '{id}'"))?
            .status = status;

        Ok(())
    }

    pub fn remove_job(&self, id: JobId) -> Option<Job> {
        self.0.write().unwrap().jobs.remove(&id)
    }

    /// Remove completed jobs that are older than the given timestamp.
    pub fn purge_completed(&self, completed_before: Timestamp) {
        self.0.write().unwrap().jobs.retain(|_, j| match &j.status {
            JobStatus::Finished { finished_at, .. } => *finished_at < completed_before,
            _ => false,
        })
    }
}
