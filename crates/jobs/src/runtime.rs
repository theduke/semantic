use super::*;
use semantic_data::DateTime;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

type Reply<T> = oneshot::Sender<Result<T, JobsError>>;
enum Command {
    Submit {
        record: JobRecord,
        job: Box<dyn ErasedJob>,
        groups: Vec<JobGroup>,
        reply: Reply<watch::Receiver<JobRecord>>,
    },
    Get(JobId, Reply<Option<JobRecord>>),
    List(JobListQuery, Reply<JobListPage>),
    Cancel(JobId, Reply<JobRecord>),
    Clear(Reply<ClearCompletedResult>),
    Group(Reply<JobGroup>),
    Invalidate(JobGroup, JobError, Reply<GroupCancellation>),
    Shutdown(Reply<()>),
}

#[derive(Clone)]
pub struct ScopeJobs {
    tx: mpsc::UnboundedSender<Command>,
    registry: JobsRegistry,
    health: watch::Receiver<JobsHealth>,
    active: Arc<AtomicBool>,
}
pub struct GroupCancellation {
    receivers: Vec<watch::Receiver<JobRecord>>,
    persistence_error: Option<JobsError>,
}
impl GroupCancellation {
    /// Initial persistence failure, if any. Cancellation is already effective in
    /// memory; `wait` still tracks durable completion while storage recovers.
    pub fn persistence_error(&self) -> Option<&JobsError> {
        self.persistence_error.as_ref()
    }
    pub async fn wait(mut self) -> Result<(), JobsError> {
        for receiver in &mut self.receivers {
            while !receiver.borrow().status.is_terminal() {
                receiver.changed().await.map_err(|_| JobsError::Closed)?;
            }
        }
        Ok(())
    }
}

impl ScopeJobs {
    /// Caller guarantees exclusive ownership of this writable scope until shutdown.
    pub async fn open(
        store: Arc<dyn JobStore>,
        registry: JobsRegistry,
        config: JobsConfig,
    ) -> Result<Self, JobsError> {
        store.initialize().await?;
        let mut cursor = None;
        loop {
            let page = store
                .list(JobListQuery {
                    oldest_first: true,
                    cursor,
                    limit: 128,
                    ..Default::default()
                })
                .await?;
            for mut record in page.records {
                record.validate().map_err(JobStoreError::definitive)?;
                if record.status.is_terminal() {
                    continue;
                }
                record.status = JobStatus::Interrupted;
                record.error = Some(JobError::new(
                    "coordinator_restarted",
                    "Runtime inputs were lost when the coordinator stopped",
                ));
                record.finished_at = Some(DateTime::now_utc());
                record.updated_at = DateTime::now_utc();
                record.snapshot_seq = record
                    .snapshot_seq
                    .checked_add(1)
                    .ok_or_else(|| JobStoreError::definitive("job snapshot sequence exhausted"))?;
                write_snapshot(&*store, &record, None).await?;
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let (health_tx, health) = watch::channel(JobsHealth::Ready);
        let active = Arc::new(AtomicBool::new(false));
        let mut coordinator = Coordinator {
            scope: uuid::Uuid::new_v4(),
            store,
            config,
            rx,
            rx_open: true,
            health: health_tx,
            active: active.clone(),
            live: BTreeMap::new(),
            queue: VecDeque::new(),
            groups: BTreeMap::new(),
            running: 0,
            pending: None,
            shutdown: Vec::new(),
            closing: false,
            workers: tokio::task::JoinSet::new(),
            worker_ids: BTreeMap::new(),
            clearing: VecDeque::new(),
            maintenance_due: false,
        };
        coordinator.cleanup(false).await?;
        tokio::spawn(coordinator.run());
        Ok(Self {
            tx,
            registry,
            health,
            active,
        })
    }
    pub fn health(&self) -> JobsHealth {
        self.health.borrow().clone()
    }
    pub fn subscribe_health(&self) -> watch::Receiver<JobsHealth> {
        self.health.clone()
    }
    pub fn has_active_work(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub fn kinds(&self) -> Vec<JobKindDescriptor> {
        self.registry.kinds()
    }
    pub async fn submit<H: JobHandler>(
        &self,
        registration: &RegisteredJob<H>,
        input: H::Input,
        options: SubmitOptions,
    ) -> Result<JobTicket<H::Output>, SubmitError> {
        if registration.registry_id != self.registry.id {
            return Err(SubmitError {
                id: None,
                source: JobsError::ForeignRegistration,
            });
        }
        let id = JobId(uuid::Uuid::new_v4().to_string());
        let now = DateTime::now_utc();
        let record = JobRecord {
            id: id.clone(),
            kind: registration.handler.kind().id.clone(),
            status: JobStatus::Queued,
            progress: JobProgress::default(),
            error: None,
            created_at: now,
            started_at: None,
            updated_at: now,
            finished_at: None,
            snapshot_seq: 1,
        };
        let (sender, receiver) = oneshot::channel();
        let job = Box::new(TypedJob {
            handler: registration.handler.clone(),
            input,
            sender,
        });
        let progress = self
            .request(|reply| Command::Submit {
                record,
                job,
                groups: options.groups,
                reply,
            })
            .await
            .map_err(|source| SubmitError {
                id: Some(id.clone()),
                source,
            })?;
        Ok(JobTicket {
            id,
            receiver,
            progress,
        })
    }
    pub async fn get(&self, id: JobId) -> Result<Option<JobRecord>, JobsError> {
        self.request(|r| Command::Get(id, r)).await
    }
    pub async fn list(&self, query: JobListQuery) -> Result<JobListPage, JobsError> {
        self.request(|r| Command::List(query, r)).await
    }
    pub async fn cancel(&self, id: JobId) -> Result<JobRecord, JobsError> {
        self.request(|r| Command::Cancel(id, r)).await
    }
    pub async fn clear_completed(&self) -> Result<ClearCompletedResult, JobsError> {
        self.request(Command::Clear).await
    }
    pub async fn create_group(&self) -> Result<JobGroup, JobsError> {
        self.request(Command::Group).await
    }
    pub async fn invalidate_group(
        &self,
        group: &JobGroup,
        reason: JobError,
    ) -> Result<GroupCancellation, JobsError> {
        self.request(|r| Command::Invalidate(group.clone(), reason, r))
            .await
    }
    pub async fn shutdown(&self) -> Result<(), JobsError> {
        if self.health() == JobsHealth::Closed {
            return Ok(());
        }
        match self.request(Command::Shutdown).await {
            Err(JobsError::Closed) if self.health() == JobsHealth::Closed => Ok(()),
            result => result,
        }
    }
    async fn request<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, JobsError> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(command(tx)).map_err(|_| JobsError::Closed)?;
        rx.await.map_err(|_| JobsError::Closed)?
    }
}

struct Live {
    record: JobRecord,
    context: JobContext,
    job: Option<Box<dyn ErasedJob>>,
    completion: Option<Completion>,
    groups: Vec<uuid::Uuid>,
    updates: watch::Sender<JobRecord>,
    dirty: bool,
    executing: bool,
}
struct Pending {
    record: JobRecord,
    error: JobStoreError,
}
struct Clearing {
    ids: VecDeque<JobId>,
    deleted: u64,
    reply: Reply<ClearCompletedResult>,
}
struct Coordinator {
    scope: uuid::Uuid,
    store: Arc<dyn JobStore>,
    config: JobsConfig,
    rx: mpsc::UnboundedReceiver<Command>,
    health: watch::Sender<JobsHealth>,
    active: Arc<AtomicBool>,
    live: BTreeMap<JobId, Live>,
    queue: VecDeque<JobId>,
    groups: BTreeMap<uuid::Uuid, BTreeSet<JobId>>,
    running: usize,
    rx_open: bool,
    pending: Option<Pending>,
    closing: bool,
    shutdown: Vec<Reply<()>>,
    workers: tokio::task::JoinSet<(JobId, Completion)>,
    worker_ids: BTreeMap<tokio::task::Id, JobId>,
    clearing: VecDeque<Clearing>,
    maintenance_due: bool,
}
impl Coordinator {
    async fn run(mut self) {
        let mut progress = tokio::time::interval(Duration::from_secs(1));
        progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut maintenance = tokio::time::interval(Duration::from_secs(60));
        maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            self.active.store(
                !self.live.is_empty() || self.pending.is_some(),
                Ordering::Release,
            );
            if self.closing
                && self.live.is_empty()
                && self.pending.is_none()
                && self.clearing.is_empty()
            {
                break;
            }
            tokio::select! {
                command = self.rx.recv(), if self.rx_open => {
                    match command { Some(command) => self.command(command).await, None => { self.rx_open = false; self.begin_shutdown().await; } }
                }
                outcome = self.workers.join_next_with_id(), if !self.workers.is_empty() => {
                    match outcome {
                        Some(Ok((task, (id, completion)))) => { self.worker_ids.remove(&task); self.complete(id, completion).await; }
                        Some(Err(error)) => {
                            if let Some(id) = self.worker_ids.remove(&error.id()) {
                                let code = if error.is_panic() { "handler_panicked" } else { "handler_interrupted" };
                                self.complete(id, Completion { interrupted: !error.is_panic(), error: Some(JobError::new(code, "Worker terminated unexpectedly")), deliver: Box::new(|_| {}) }).await;
                            }
                        }
                        None => {}
                    }
                }
                _ = progress.tick() => { self.repair().await; self.flush_progress().await; }
                _ = maintenance.tick() => { self.maintenance_due = true; }
                _ = std::future::ready(()), if self.pending.is_none() && (!self.clearing.is_empty() || self.maintenance_due) => {
                    self.cleanup_chunk().await;
                }
            }
            self.dispatch().await;
        }
        self.active.store(false, Ordering::Release);
        self.health.send_replace(JobsHealth::Closed);
        for reply in self.shutdown {
            let _ = reply.send(Ok(()));
        }
    }

    async fn command(&mut self, command: Command) {
        match command {
            Command::Get(id, reply) => {
                let _ = reply.send(self.store.get(&id).await.map_err(Into::into));
            }
            Command::List(query, reply) => {
                let _ = reply.send(self.store.list(query).await.map_err(Into::into));
            }
            Command::Submit {
                record,
                job,
                groups,
                reply,
            } => {
                let rejection = if self.closing {
                    Some(JobsError::Closed)
                } else if let Some(pending) = &self.pending {
                    Some(JobsError::Store(pending.error.clone()))
                } else if groups
                    .iter()
                    .any(|g| g.scope != self.scope || !self.groups.contains_key(&g.id))
                {
                    Some(JobsError::InvalidGroup)
                } else {
                    None
                };
                if let Some(error) = rejection {
                    let _ = reply.send(Err(error));
                    return;
                }
                let id = record.id.clone();
                let context = JobContext {
                    id: id.clone(),
                    cancellation: CancellationToken::new(),
                    progress: Arc::new(Mutex::new(record.progress.clone())),
                };
                let (updates, receiver) = watch::channel(record.clone());
                for group in &groups {
                    self.groups.get_mut(&group.id).unwrap().insert(id.clone());
                }
                self.live.insert(
                    id.clone(),
                    Live {
                        record,
                        context,
                        job: Some(job),
                        completion: None,
                        groups: groups.into_iter().map(|g| g.id).collect(),
                        updates,
                        dirty: true,
                        executing: false,
                    },
                );
                match self.persist(&id).await {
                    Ok(()) => {
                        self.active.store(true, Ordering::Release);
                        self.queue.push_back(id);
                        let _ = reply.send(Ok(receiver));
                    }
                    Err(error) => {
                        self.cancel_memory(
                            &id,
                            JobError::new("admission_failed", "Job admission storage failed"),
                        );
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Command::Cancel(id, reply) => {
                let result = if self.live.contains_key(&id) {
                    self.cancel_memory(&id, JobError::new("user_cancelled", "Cancelled by user"));
                    self.persist(&id).await.and_then(|_| {
                        self.live
                            .get(&id)
                            .map(|l| l.record.clone())
                            .ok_or(JobsError::NotFound)
                    })
                } else {
                    self.store
                        .get(&id)
                        .await
                        .map_err(Into::into)
                        .and_then(|v| v.ok_or(JobsError::NotFound))
                };
                let _ = reply.send(result);
                self.retire();
            }
            Command::Group(reply) => {
                let result = if self.closing {
                    Err(JobsError::Closed)
                } else {
                    let id = uuid::Uuid::new_v4();
                    self.groups.insert(id, BTreeSet::new());
                    Ok(JobGroup {
                        scope: self.scope,
                        id,
                    })
                };
                let _ = reply.send(result);
            }
            Command::Invalidate(group, reason, reply) => {
                if group.scope != self.scope || !self.groups.contains_key(&group.id) {
                    let _ = reply.send(Err(JobsError::InvalidGroup));
                    return;
                }
                let ids: Vec<_> = self.groups.remove(&group.id).unwrap().into_iter().collect();
                let receivers = ids
                    .iter()
                    .map(|id| self.live[id].updates.subscribe())
                    .collect();
                for id in &ids {
                    self.cancel_memory(id, reason.clone());
                }
                let mut cancellation = GroupCancellation {
                    receivers,
                    persistence_error: None,
                };
                for id in ids {
                    if let Err(error) = self.persist(&id).await {
                        cancellation.persistence_error = Some(error);
                        break;
                    }
                }
                let _ = reply.send(Ok(cancellation));
                self.retire();
            }
            Command::Clear(reply) => {
                if let Some(pending) = &self.pending {
                    let _ = reply.send(Err(JobsError::Store(pending.error.clone())));
                } else {
                    match self.capture_completed().await {
                        Ok(ids) => self.clearing.push_back(Clearing {
                            ids,
                            deleted: 0,
                            reply,
                        }),
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
            }
            Command::Shutdown(reply) => {
                self.shutdown.push(reply);
                self.begin_shutdown().await;
                if let Some(pending) = &self.pending {
                    for reply in self.shutdown.drain(..) {
                        let _ = reply.send(Err(JobsError::Store(pending.error.clone())));
                    }
                }
            }
        }
    }

    fn cancel_memory(&mut self, id: &JobId, reason: JobError) {
        let Some(live) = self.live.get_mut(id) else {
            return;
        };
        if live.record.status.is_terminal() {
            return;
        }
        live.context.cancellation.cancel();
        live.record.error = Some(reason);
        live.record.status = if live.executing {
            JobStatus::Cancelling
        } else {
            JobStatus::Cancelled
        };
        if !live.executing {
            live.record.finished_at = Some(DateTime::now_utc());
        }
        live.dirty = true;
    }

    async fn begin_shutdown(&mut self) {
        self.closing = true;
        self.health.send_replace(JobsHealth::ShuttingDown);
        self.groups.clear();
        let ids: Vec<_> = self.live.keys().cloned().collect();
        for id in &ids {
            self.cancel_memory(id, JobError::new("scope_closed", "Scope is closing"));
        }
        for id in ids {
            if self.persist(&id).await.is_err() {
                break;
            }
        }
        self.retire();
    }

    async fn dispatch(&mut self) {
        while !self.closing
            && self.pending.is_none()
            && self.running < self.config.max_concurrent_jobs.get()
        {
            let Some(id) = self.queue.pop_front() else {
                break;
            };
            let Some(live) = self.live.get_mut(&id) else {
                continue;
            };
            if live.record.status != JobStatus::Queued {
                continue;
            }
            live.record.status = JobStatus::Running;
            live.record.started_at = Some(DateTime::now_utc());
            live.dirty = true;
            if self.persist(&id).await.is_err() {
                self.queue.push_front(id);
                break;
            }
            self.start(&id);
        }
        self.retire();
    }
    fn start(&mut self, id: &JobId) {
        let Some(live) = self.live.get_mut(id) else {
            return;
        };
        if live.executing || live.record.status != JobStatus::Running {
            return;
        }
        let Some(job) = live.job.take() else {
            return;
        };
        live.executing = true;
        self.running += 1;
        let context = live.context.clone();
        let id = id.clone();
        let job_id = id.clone();
        let task = self
            .workers
            .spawn(async move { (id, job.start(context).await) });
        self.worker_ids.insert(task.id(), job_id);
    }
    async fn complete(&mut self, id: JobId, completion: Completion) {
        let Some(live) = self.live.get_mut(&id) else {
            return;
        };
        live.executing = false;
        self.running -= 1;
        if live.record.status == JobStatus::Cancelling {
            live.record.status = JobStatus::Cancelled;
        } else {
            live.record.status = if completion.interrupted {
                JobStatus::Interrupted
            } else if completion.error.is_some() {
                JobStatus::Failed
            } else {
                JobStatus::Succeeded
            };
            live.record.error = completion.error.clone();
        }
        live.record.finished_at = Some(DateTime::now_utc());
        live.completion = Some(completion);
        live.dirty = true;
        let _ = self.persist(&id).await;
        self.retire();
        self.maintenance_due = true;
    }

    async fn persist(&mut self, id: &JobId) -> Result<(), JobsError> {
        if let Some(pending) = &self.pending {
            return Err(JobsError::Store(pending.error.clone()));
        }
        let Some(live) = self.live.get_mut(id) else {
            return Ok(());
        };
        if !live.dirty {
            return Ok(());
        }
        live.record.progress = live.context.progress();
        live.record.updated_at = DateTime::now_utc();
        // One sequence per desired snapshot; retries use exactly this snapshot.
        let mut snapshot = live.record.clone();
        snapshot.snapshot_seq += 1;
        match write_snapshot(&*self.store, &snapshot, None).await {
            Ok(()) => {
                tracing::debug!(job_id = %snapshot.id.0, kind = %snapshot.kind.0, status = snapshot.status.as_str(), sequence = snapshot.snapshot_seq, "job snapshot persisted");
                live.record = snapshot;
                live.dirty = false;
                live.updates.send_replace(live.record.clone());
                Ok(())
            }
            Err(error) => {
                self.pending = Some(Pending {
                    record: snapshot,
                    error: error.clone(),
                });
                self.health
                    .send_replace(JobsHealth::StorageUnavailable(error.to_string()));
                // Shutdown may have begun successfully before a later terminal
                // snapshot fails. Release callers, but retain this coordinator
                // and its pending snapshot for recovery and a subsequent retry.
                for reply in self.shutdown.drain(..) {
                    let _ = reply.send(Err(JobsError::Store(error.clone())));
                }
                let ids: Vec<_> = self.live.keys().cloned().collect();
                for id in ids {
                    if self.live[&id].executing {
                        self.cancel_memory(
                            &id,
                            JobError::new(
                                "storage_unavailable",
                                "Job metadata storage is unavailable",
                            ),
                        );
                    }
                }
                Err(JobsError::Store(error))
            }
        }
    }
    async fn repair(&mut self) {
        if let Some(pending) = self.pending.take() {
            match write_snapshot(&*self.store, &pending.record, Some(&pending.error)).await {
                Ok(()) => {
                    if let Some(live) = self.live.get_mut(&pending.record.id) {
                        live.record.snapshot_seq = pending.record.snapshot_seq;
                        // Keep changes made while degraded; publish only the acknowledged snapshot.
                        live.updates.send_replace(pending.record);
                        live.dirty = true;
                    }
                }
                Err(error) => {
                    self.pending = Some(Pending {
                        record: pending.record,
                        error,
                    });
                    return;
                }
            }
        }
        let ids: Vec<_> = self.live.keys().cloned().collect();
        for id in &ids {
            if self.persist(id).await.is_err() {
                return;
            }
        }
        if !self.closing {
            self.health.send_replace(JobsHealth::Ready);
        }
        // A repaired Running admission still owns its original input, and starts once.
        for id in ids {
            if !self.closing && self.running < self.config.max_concurrent_jobs.get() {
                self.start(&id);
            }
        }
        self.retire();
    }
    async fn flush_progress(&mut self) {
        let ids: Vec<_> = self
            .live
            .iter_mut()
            .filter_map(|(id, live)| {
                if live.context.progress() != live.record.progress {
                    live.dirty = true;
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();
        for id in ids {
            if self.persist(&id).await.is_err() {
                break;
            }
        }
    }
    fn retire(&mut self) {
        let ids: Vec<_> = self
            .live
            .iter()
            .filter(|(id, live)| {
                live.record.status.is_terminal()
                    && !live.executing
                    && !live.dirty
                    && self.pending.as_ref().is_none_or(|p| &p.record.id != *id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let live = self.live.remove(&id).unwrap();
            for group in &live.groups {
                if let Some(ids) = self.groups.get_mut(group) {
                    ids.remove(&id);
                }
            }
            let error = if live.record.status == JobStatus::Cancelled {
                Some(JobCompletionError::Cancelled(
                    live.record
                        .error
                        .clone()
                        .unwrap_or_else(|| JobError::new("cancelled", "Cancelled")),
                ))
            } else {
                None
            };
            if let Some(completion) = live.completion {
                (completion.deliver)(error);
            } else if let Some(job) = live.job {
                job.reject(error.unwrap_or(JobCompletionError::Interrupted));
            }
        }
    }

    async fn cleanup(&mut self, clear: bool) -> Result<u64, JobsError> {
        let excess = if clear {
            u64::MAX
        } else {
            self.store
                .count()
                .await?
                .saturating_sub(self.config.history_threshold.get() as u64)
        };
        if excess == 0 {
            return Ok(0);
        }
        let mut deleted = 0;
        let mut cursor = None;
        // Coordinator serialization captures exactly the terminal set at this command.
        loop {
            let page = self
                .store
                .list(JobListQuery {
                    statuses: JobStatus::ALL
                        .into_iter()
                        .filter(|s| s.is_terminal())
                        .collect(),
                    oldest_first: true,
                    limit: (excess - deleted).min(128) as u32,
                    cursor,
                    ..Default::default()
                })
                .await
                .map_err(|source| JobsError::PartialClear { deleted, source })?;
            let ids: Vec<_> = page
                .records
                .into_iter()
                .filter(|r| !self.live.contains_key(&r.id))
                .map(|r| r.id)
                .collect();
            deleted += self
                .store
                .delete_ids(&ids)
                .await
                .map_err(|source| JobsError::PartialClear { deleted, source })?;
            cursor = page.next_cursor;
            if cursor.is_none() || deleted >= excess {
                return Ok(deleted);
            }
            tokio::task::yield_now().await;
        }
    }

    async fn capture_completed(&mut self) -> Result<VecDeque<JobId>, JobsError> {
        let mut ids = VecDeque::new();
        let mut cursor = None;
        loop {
            let page = self
                .store
                .list(JobListQuery {
                    statuses: JobStatus::ALL
                        .into_iter()
                        .filter(|s| s.is_terminal())
                        .collect(),
                    oldest_first: true,
                    cursor,
                    limit: 128,
                    ..Default::default()
                })
                .await?;
            ids.extend(
                page.records
                    .into_iter()
                    .filter(|r| !self.live.contains_key(&r.id))
                    .map(|r| r.id),
            );
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(ids);
            }
        }
    }

    /// One deletion batch per coordinator turn so cancellation/completion can run
    /// between batches. Clear's IDs were captured before any later completion.
    async fn cleanup_chunk(&mut self) {
        if let Some(mut clearing) = self.clearing.pop_front() {
            let ids: Vec<_> = clearing.ids.drain(..clearing.ids.len().min(128)).collect();
            match self.store.delete_ids(&ids).await {
                Ok(count) => {
                    clearing.deleted += count;
                    if clearing.ids.is_empty() {
                        let _ = clearing.reply.send(Ok(ClearCompletedResult {
                            deleted: clearing.deleted,
                        }));
                    } else {
                        self.clearing.push_front(clearing);
                    }
                }
                Err(source) => {
                    let _ = clearing.reply.send(Err(JobsError::PartialClear {
                        deleted: clearing.deleted,
                        source,
                    }));
                }
            }
            return;
        }
        self.maintenance_due = false;
        let store = self.store.clone();
        let history_threshold = self.config.history_threshold.get() as u64;
        let live_ids: BTreeSet<_> = self.live.keys().cloned().collect();
        let result = async move {
            let excess = store.count().await?.saturating_sub(history_threshold);
            if excess == 0 {
                return Ok::<bool, JobStoreError>(false);
            }
            let page = store
                .list(JobListQuery {
                    statuses: JobStatus::ALL
                        .into_iter()
                        .filter(|s| s.is_terminal())
                        .collect(),
                    oldest_first: true,
                    limit: excess.min(128) as u32,
                    ..Default::default()
                })
                .await?;
            let ids: Vec<_> = page
                .records
                .into_iter()
                .filter(|r| !live_ids.contains(&r.id))
                .map(|r| r.id)
                .collect();
            let deleted = store.delete_ids(&ids).await?;
            Ok(deleted > 0 && deleted < excess)
        }
        .await;
        match result {
            Ok(more) => self.maintenance_due = more,
            Err(error) => tracing::warn!(%error, "job history maintenance failed"),
        }
    }
}

async fn write_snapshot(
    store: &dyn JobStore,
    record: &JobRecord,
    previous: Option<&JobStoreError>,
) -> Result<(), JobStoreError> {
    if let Some(error) = previous {
        // A failed reconciliation read cannot make an unsettled write safe to retry.
        match store.get(&record.id).await {
            Ok(stored) if stored.as_ref() == Some(record) => return Ok(()),
            Err(_) => return Err(error.clone()),
            _ => {}
        }
        if error.outcome_unknown && !error.settled {
            return Err(error.clone());
        }
    }
    match store.put(record).await {
        Ok(()) => Ok(()),
        Err(error) => {
            if error.outcome_unknown
                && store.get(&record.id).await.ok().flatten().as_ref() == Some(record)
            {
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}
