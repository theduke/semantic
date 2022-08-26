//! Job system for long-running jobs.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use anyhow::anyhow;
use factdb::Timestamp;
use semantic_core::api::{Job, JobEvent, JobId, JobStatus, JobStep};

#[derive(Clone)]
pub struct JobManager(Arc<RwLock<State>>);

#[derive(Clone, Debug)]
struct JobState {
    job: Job,
    events: Vec<JobEvent>,
}

struct State {
    jobs: HashMap<JobId, JobState>,
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
        self.0.read().unwrap().jobs.get(&id).map(|i| i.job.clone())
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
        let item = JobState {
            job: job.clone(),
            events: Vec::new(),
        };
        self.0.write().unwrap().jobs.insert(job.id, item);
        job
    }

    pub fn job_update(&self, id: JobId, status: JobStatus) -> Result<Job, anyhow::Error> {
        let mut lock = self.0.write().unwrap();
        let item = lock
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Job not found: '{id}'"))?;
        item.job.update(status);

        tracing::trace!(job=?item.job, "job status update");
        Ok(item.job.clone())
    }

    pub fn job_add_steps(&self, id: JobId, steps: Vec<String>) -> Result<Job, anyhow::Error> {
        let mut lock = self.0.write().unwrap();
        let item = lock
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Job not found: '{id}'"))?;

        let steps = steps.into_iter().map(|name| JobStep {
            name,
            started_at: None,
            finished_at: None,
        });
        item.job.steps.extend(steps);

        Ok(item.job.clone())
    }

    pub fn job_add_events(&self, id: JobId, events: Vec<JobEvent>) -> Result<(), anyhow::Error> {
        let mut lock = self.0.write().unwrap();
        let item = lock
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Job not found: '{id}'"))?;

        item.events.extend(events);
        Ok(())
    }

    pub fn job_take_events(&self, id: JobId) -> Result<Vec<JobEvent>, anyhow::Error> {
        let mut lock = self.0.write().unwrap();
        let item = lock
            .jobs
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Job not found: '{id}'"))?;
        let events = std::mem::take(&mut item.events);
        Ok(events)
    }

    pub fn remove_job(&self, id: JobId) -> Option<Job> {
        let item = self.0.write().unwrap().jobs.remove(&id)?;
        Some(item.job)
    }

    /// Remove completed jobs that are older than the given timestamp.
    pub fn purge_completed(&self, completed_before: Timestamp) {
        self.0.write().unwrap().jobs.retain(|_, item| {
            item.job
                .finished_at
                .map(|t| t > completed_before)
                .unwrap_or(true)
        });
    }
}
