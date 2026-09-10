//! Typed, ephemeral work with persistent operational metadata.
//!
//! One owner must open each writable scope. This is a single-process coordinator,
//! not a durable queue: reopening interrupts old work and never replays inputs.
mod runtime;
mod store;

use futures_util::FutureExt;
pub use runtime::{GroupCancellation, ScopeJobs};
pub use semantic_data::jobs::*;
use std::{
    collections::BTreeMap,
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{Arc, Mutex},
};
pub use store::*;
use tokio::sync::{oneshot, watch};
pub use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobsConfig {
    pub max_concurrent_jobs: NonZeroUsize,
    pub history_threshold: NonZeroUsize,
}
impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            max_concurrent_jobs: NonZeroUsize::new(4).unwrap(),
            history_threshold: NonZeroUsize::new(1000).unwrap(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobsHealth {
    Ready,
    StorageUnavailable(String),
    ShuttingDown,
    Closed,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum JobsError {
    #[error("jobs coordinator is closed")]
    Closed,
    #[error("jobs storage: {0}")]
    Store(#[from] JobStoreError),
    #[error("job not found")]
    NotFound,
    #[error("registration belongs to another registry")]
    ForeignRegistration,
    #[error("job group is closed or belongs to another scope")]
    InvalidGroup,
    #[error("duplicate or empty job kind: {0}")]
    InvalidKind(String),
    #[error("history clearing failed after deleting {deleted} records: {source}")]
    PartialClear { deleted: u64, source: JobStoreError },
}

#[derive(Debug, thiserror::Error)]
#[error("job submission {id:?}: {source}")]
pub struct SubmitError {
    pub id: Option<JobId>,
    pub source: JobsError,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum JobCompletionError {
    #[error("{0}")]
    Failed(JobError),
    #[error("{0}")]
    Cancelled(JobError),
    #[error("coordinator stopped before durable completion")]
    Interrupted,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid progress: total below completed or counter decreased within the same phase/unit")]
pub struct ProgressError;

#[derive(Clone)]
pub struct JobContext {
    id: JobId,
    cancellation: CancellationToken,
    progress: Arc<Mutex<JobProgress>>,
}
impl JobContext {
    pub fn id(&self) -> &JobId {
        &self.id
    }
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
    pub fn check_cancelled(&self) -> Result<(), JobError> {
        if self.cancellation.is_cancelled() {
            Err(JobError::new("cancelled", "Cancellation requested"))
        } else {
            Ok(())
        }
    }
    pub fn report_progress(&self, progress: JobProgress) -> Result<(), ProgressError> {
        let mut current = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if progress
            .total
            .is_some_and(|total| total < progress.completed)
            || (current.phase == progress.phase
                && current.unit == progress.unit
                && progress.completed < current.completed)
        {
            return Err(ProgressError);
        }
        *current = progress;
        Ok(())
    }
    fn progress(&self) -> JobProgress {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub trait JobHandler: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + 'static;
    fn kind(&self) -> &JobKindDescriptor;
    fn run<'a>(
        &'a self,
        input: Self::Input,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, JobError>> + Send + 'a>>;
}

pub struct RegisteredJob<H: JobHandler> {
    registry_id: uuid::Uuid,
    handler: Arc<H>,
}
impl<H: JobHandler> Clone for RegisteredJob<H> {
    fn clone(&self) -> Self {
        Self {
            registry_id: self.registry_id,
            handler: self.handler.clone(),
        }
    }
}

pub struct JobsBuilder {
    id: uuid::Uuid,
    kinds: BTreeMap<JobKindId, JobKindDescriptor>,
}
impl Default for JobsBuilder {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            kinds: BTreeMap::new(),
        }
    }
}
impl JobsBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register<H: JobHandler>(&mut self, handler: H) -> Result<RegisteredJob<H>, JobsError> {
        let kind = handler.kind().clone();
        if kind.id.0.is_empty() || self.kinds.contains_key(&kind.id) {
            return Err(JobsError::InvalidKind(kind.id.0));
        }
        self.kinds.insert(kind.id.clone(), kind);
        Ok(RegisteredJob {
            registry_id: self.id,
            handler: Arc::new(handler),
        })
    }
    pub fn build(self) -> JobsRegistry {
        JobsRegistry {
            id: self.id,
            kinds: Arc::new(self.kinds),
        }
    }
}

#[derive(Clone)]
pub struct JobsRegistry {
    id: uuid::Uuid,
    kinds: Arc<BTreeMap<JobKindId, JobKindDescriptor>>,
}
impl Default for JobsRegistry {
    fn default() -> Self {
        JobsBuilder::new().build()
    }
}
impl JobsRegistry {
    pub fn kinds(&self) -> Vec<JobKindDescriptor> {
        self.kinds.values().cloned().collect()
    }
}

#[derive(Clone, Debug)]
pub struct JobGroup {
    scope: uuid::Uuid,
    id: uuid::Uuid,
}
#[derive(Default)]
pub struct SubmitOptions {
    pub groups: Vec<JobGroup>,
}

pub struct JobTicket<T> {
    pub id: JobId,
    receiver: oneshot::Receiver<Result<T, JobCompletionError>>,
    progress: watch::Receiver<JobRecord>,
}
impl<T> JobTicket<T> {
    pub async fn wait(self) -> Result<T, JobCompletionError> {
        self.receiver
            .await
            .unwrap_or(Err(JobCompletionError::Interrupted))
    }
    pub fn subscribe(&self) -> watch::Receiver<JobRecord> {
        self.progress.clone()
    }
}

trait ErasedJob: Send {
    fn start(
        self: Box<Self>,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Completion> + Send>>;
    fn reject(self: Box<Self>, error: JobCompletionError);
}
struct TypedJob<H: JobHandler> {
    handler: Arc<H>,
    input: H::Input,
    sender: oneshot::Sender<Result<H::Output, JobCompletionError>>,
}
struct Completion {
    interrupted: bool,
    error: Option<JobError>,
    deliver: Box<dyn FnOnce(Option<JobCompletionError>) + Send>,
}
impl<H: JobHandler> ErasedJob for TypedJob<H> {
    fn start(
        self: Box<Self>,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Completion> + Send>> {
        Box::pin(async move {
            let Self {
                handler,
                input,
                sender,
            } = *self;
            let result = std::panic::AssertUnwindSafe(async { handler.run(input, context).await })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(JobError::new("handler_panicked", "Job handler panicked")));
            Completion {
                interrupted: false,
                error: result.as_ref().err().cloned(),
                deliver: Box::new(move |override_error| {
                    let _ = sender.send(match override_error {
                        Some(error) => Err(error),
                        None => result.map_err(JobCompletionError::Failed),
                    });
                }),
            }
        })
    }
    fn reject(self: Box<Self>, error: JobCompletionError) {
        let _ = self.sender.send(Err(error));
    }
}
