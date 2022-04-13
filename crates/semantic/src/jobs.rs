//! Job system for long-running jobs.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use anyhow::anyhow;
use factordb::{data::Timestamp, AnyError};
use semantic_core::api::{Job, JobId, JobStatus, JobStep};

#[derive(Clone)]
pub struct JobManager(Arc<RwLock<State>>);

struct State {
    jobs: HashMap<JobId, Job>,
}

#[derive(Debug, Clone)]
pub struct JobInit {
    pub name: String,
    pub steps: Vec<String>,
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

    pub fn register_job(&self, init: JobInit) -> Job {
        let id = uuid::Uuid::new_v4();
        let steps: Vec<JobStep> = init
            .steps
            .into_iter()
            .map(|s| JobStep {
                name: s,
                started_at: None,
                finished_at: None,
            })
            .collect();
        let job = Job {
            id,
            name: init.name,
            created_at: Timestamp::now(),
            started_at: None,
            finished_at: None,
            steps,
            status: JobStatus::Queued {
                queue_position: None,
            },
        };
        self.0.write().unwrap().jobs.insert(job.id, job.clone());
        job
    }

    pub fn job_update(&self, id: JobId, status: JobStatus) -> Result<Job, AnyError> {
        let mut lock = self.0.write().unwrap();
        let job = lock
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Job not found: '{id}'"))?;
        job.update(status);

        tracing::trace!(job=?job, "job status update");
        Ok(job.clone())
    }

    pub fn job_add_steps(&self, id: JobId, steps: Vec<String>) -> Result<Job, AnyError> {
        let mut lock = self.0.write().unwrap();
        let job = lock
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Job not found: '{id}'"))?;

        let steps = steps.into_iter().map(|name| JobStep {
            name,
            started_at: None,
            finished_at: None,
        });
        job.steps.extend(steps);

        Ok(job.clone())
    }

    pub fn remove_job(&self, id: JobId) -> Option<Job> {
        self.0.write().unwrap().jobs.remove(&id)
    }

    /// Remove completed jobs that are older than the given timestamp.
    pub fn purge_completed(&self, completed_before: Timestamp) {
        self.0
            .write()
            .unwrap()
            .jobs
            .retain(|_, j| j.finished_at.map(|t| t > completed_before).unwrap_or(true));
    }
}
