use async_trait::async_trait;
use semantic_jobs::*;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::{mpsc, oneshot};

#[derive(Default)]
struct Store {
    rows: Mutex<BTreeMap<JobId, JobRecord>>,
    writes: AtomicUsize,
    fault: AtomicUsize,
    read_fault: AtomicUsize,
    delete_fault: AtomicUsize,
    unsettled: Mutex<Option<JobRecord>>,
}
#[async_trait]
impl JobStore for Store {
    async fn initialize(&self) -> Result<(), JobStoreError> {
        Ok(())
    }
    async fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
        if self.read_fault.load(Ordering::SeqCst) != 0 {
            return Err(JobStoreError::definitive("read offline"));
        }
        Ok(self.rows.lock().unwrap().get(id).cloned())
    }
    async fn put(&self, record: &JobRecord) -> Result<(), JobStoreError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let fault = self.fault.load(Ordering::SeqCst);
        if fault == 1 || (fault == 5 && record.status == JobStatus::Running) {
            return Err(JobStoreError::definitive("offline"));
        }
        if fault == 3 {
            *self.unsettled.lock().unwrap() = Some(record.clone());
            return Err(JobStoreError::unknown("write still running", false));
        }
        self.rows
            .lock()
            .unwrap()
            .insert(record.id.clone(), record.clone());
        if fault == 2 {
            return Err(JobStoreError::unknown("lost acknowledgement", true));
        }
        Ok(())
    }
    async fn list(&self, query: JobListQuery) -> Result<JobListPage, JobStoreError> {
        let mut records: Vec<_> = self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|r| {
                (query.statuses.is_empty() || query.statuses.contains(&r.status))
                    && query.kind.as_ref().is_none_or(|k| k == &r.kind)
            })
            .cloned()
            .collect();
        records.sort_by_key(|r| (r.created_at, r.id.clone()));
        if !query.oldest_first {
            records.reverse();
        }
        if let Some(cursor) = &query.cursor {
            records.retain(|r| {
                if query.oldest_first {
                    (r.created_at, &r.id) > (cursor.created_at, &cursor.id)
                } else {
                    (r.created_at, &r.id) < (cursor.created_at, &cursor.id)
                }
            });
        }
        let more = records.len() > query.limit as usize;
        records.truncate(query.limit as usize);
        let next_cursor = if more {
            records.last().map(|r| JobListCursor {
                created_at: r.created_at,
                id: r.id.clone(),
            })
        } else {
            None
        };
        Ok(JobListPage {
            records,
            next_cursor,
        })
    }
    async fn count(&self) -> Result<u64, JobStoreError> {
        Ok(self.rows.lock().unwrap().len() as u64)
    }
    async fn delete_ids(&self, ids: &[JobId]) -> Result<u64, JobStoreError> {
        if self.delete_fault.load(Ordering::SeqCst) != 0 {
            return Err(JobStoreError::definitive("delete offline"));
        }
        let mut rows = self.rows.lock().unwrap();
        Ok(ids.iter().filter(|id| rows.remove(id).is_some()).count() as u64)
    }
}

struct Input {
    number: usize,
    started: mpsc::UnboundedSender<usize>,
    finish: oneshot::Receiver<()>,
    cooperative: bool,
}
struct Handler(JobKindDescriptor);
impl Handler {
    fn new() -> Self {
        Self(JobKindDescriptor {
            id: JobKindId("test.compute".into()),
            title: "Compute".into(),
            description: None,
        })
    }
}
impl JobHandler for Handler {
    type Input = Input;
    type Output = usize;
    fn kind(&self) -> &JobKindDescriptor {
        &self.0
    }
    fn run<'a>(
        &'a self,
        input: Input,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Result<usize, JobError>> + Send + 'a>> {
        Box::pin(async move {
            input.started.send(input.number).unwrap();
            for completed in 0..1000 {
                context
                    .report_progress(JobProgress {
                        completed,
                        total: Some(1000),
                        ..Default::default()
                    })
                    .unwrap();
            }
            if input.cooperative {
                tokio::select! { _ = input.finish => {}, _ = context.cancellation().cancelled() => {} }
            } else {
                let _ = input.finish.await;
            }
            if input.number == usize::MAX {
                panic!("test panic");
            }
            Ok(input.number)
        })
    }
}
fn input(
    number: usize,
    started: &mpsc::UnboundedSender<usize>,
    cooperative: bool,
) -> (Input, oneshot::Sender<()>) {
    let (tx, rx) = oneshot::channel();
    (
        Input {
            number,
            started: started.clone(),
            finish: rx,
            cooperative,
        },
        tx,
    )
}
async fn setup(limit: usize) -> (Arc<Store>, ScopeJobs, RegisteredJob<Handler>) {
    let store = Arc::new(Store::default());
    let mut builder = JobsBuilder::new();
    let handler = builder.register(Handler::new()).unwrap();
    let jobs = ScopeJobs::open(
        store.clone(),
        builder.build(),
        JobsConfig {
            max_concurrent_jobs: limit.try_into().unwrap(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (store, jobs, handler)
}

#[tokio::test]
async fn fifo_limit_native_payload_and_coalesced_progress() {
    let (store, jobs, handler) = setup(1).await;
    let (started, mut starts) = mpsc::unbounded_channel();
    let (one, finish_one) = input(1, &started, false);
    let ticket_one = jobs
        .submit(&handler, one, Default::default())
        .await
        .unwrap();
    assert_eq!(starts.recv().await, Some(1));
    let (two, finish_two) = input(2, &started, false);
    let ticket_two = jobs
        .submit(&handler, two, Default::default())
        .await
        .unwrap();
    let (three, finish_three) = input(3, &started, false);
    let ticket_three = jobs
        .submit(&handler, three, Default::default())
        .await
        .unwrap();
    assert_eq!(
        jobs.get(ticket_two.id.clone())
            .await
            .unwrap()
            .unwrap()
            .status,
        JobStatus::Queued
    );
    assert!(starts.try_recv().is_err());
    finish_one.send(()).unwrap();
    assert_eq!(ticket_one.wait().await.unwrap(), 1);
    assert_eq!(starts.recv().await, Some(2));
    finish_two.send(()).unwrap();
    assert_eq!(ticket_two.wait().await.unwrap(), 2);
    assert_eq!(starts.recv().await, Some(3));
    finish_three.send(()).unwrap();
    let id = ticket_three.id.clone();
    assert_eq!(ticket_three.wait().await.unwrap(), 3);
    assert_eq!(jobs.get(id).await.unwrap().unwrap().progress.completed, 999);
    assert!(store.writes.load(Ordering::SeqCst) < 20);
    jobs.shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_and_noncooperative_running_cancellation_keep_slot() {
    let (_, jobs, handler) = setup(1).await;
    let (started, mut starts) = mpsc::unbounded_channel();
    let (one, finish) = input(1, &started, false);
    let ticket = jobs
        .submit(&handler, one, Default::default())
        .await
        .unwrap();
    starts.recv().await;
    let (two, _) = input(2, &started, false);
    let queued = jobs
        .submit(&handler, two, Default::default())
        .await
        .unwrap();
    assert_eq!(
        jobs.cancel(queued.id.clone()).await.unwrap().status,
        JobStatus::Cancelled
    );
    assert!(matches!(
        queued.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    assert_eq!(
        jobs.cancel(ticket.id.clone()).await.unwrap().status,
        JobStatus::Cancelling
    );
    let (three, finish_three) = input(3, &started, true);
    let next = jobs
        .submit(&handler, three, Default::default())
        .await
        .unwrap();
    assert!(starts.try_recv().is_err());
    finish.send(()).unwrap();
    assert!(matches!(
        ticket.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    assert_eq!(starts.recv().await, Some(3));
    finish_three.send(()).unwrap();
    next.wait().await.unwrap();
    jobs.shutdown().await.unwrap();
}

#[tokio::test]
async fn group_invalidation_rejects_stale_and_foreign_handles() {
    let (_, jobs, handler) = setup(1).await;
    let (_, other, foreign) = setup(1).await;
    let group = jobs.create_group().await.unwrap();
    let wrong_group = other.create_group().await.unwrap();
    let (started, mut starts) = mpsc::unbounded_channel();
    let (one, _finish) = input(1, &started, true);
    let ticket = jobs
        .submit(
            &handler,
            one,
            SubmitOptions {
                groups: vec![group.clone()],
            },
        )
        .await
        .unwrap();
    starts.recv().await;
    let (two, _finish) = input(2, &started, true);
    let queued = jobs
        .submit(
            &handler,
            two,
            SubmitOptions {
                groups: vec![group.clone()],
            },
        )
        .await
        .unwrap();
    jobs.invalidate_group(&group, JobError::new("plugin_changed", "changed"))
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(matches!(
        ticket.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    assert!(matches!(
        queued.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    for group in [group, wrong_group] {
        let (input, _) = input(4, &started, true);
        assert!(matches!(
            jobs.submit(
                &handler,
                input,
                SubmitOptions {
                    groups: vec![group]
                }
            )
            .await
            .err()
            .unwrap()
            .source,
            JobsError::InvalidGroup
        ));
    }
    let (input, _) = input(5, &started, true);
    assert!(matches!(
        jobs.submit(&foreign, input, Default::default())
            .await
            .err()
            .unwrap()
            .source,
        JobsError::ForeignRegistration
    ));
    assert!(starts.try_recv().is_err());
    jobs.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}

#[tokio::test]
async fn panic_and_dropped_ticket_do_not_stop_runner() {
    let (_, jobs, handler) = setup(1).await;
    let (started, mut starts) = mpsc::unbounded_channel();
    let (panicking, finish) = input(usize::MAX, &started, false);
    let ticket = jobs
        .submit(&handler, panicking, Default::default())
        .await
        .unwrap();
    starts.recv().await;
    finish.send(()).unwrap();
    assert!(
        matches!(ticket.wait().await, Err(JobCompletionError::Failed(JobError { code, .. })) if code == "handler_panicked")
    );
    let (normal, finish) = input(2, &started, false);
    let ticket = jobs
        .submit(&handler, normal, Default::default())
        .await
        .unwrap();
    let mut updates = ticket.subscribe();
    drop(ticket);
    starts.recv().await;
    finish.send(()).unwrap();
    while !updates.borrow().status.is_terminal() {
        updates.changed().await.unwrap();
    }
    assert_eq!(updates.borrow().status, JobStatus::Succeeded);
    jobs.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn lost_ack_and_storage_outage_never_replay_domain_work() {
    let (store, jobs, handler) = setup(1).await;
    store.fault.store(2, Ordering::SeqCst);
    let (started, mut starts) = mpsc::unbounded_channel();
    let (one, finish) = input(1, &started, false);
    let ticket = jobs
        .submit(&handler, one, Default::default())
        .await
        .unwrap();
    starts.recv().await;
    store.fault.store(1, Ordering::SeqCst);
    finish.send(()).unwrap();
    let mut health = jobs.subscribe_health();
    while !matches!(*health.borrow(), JobsHealth::StorageUnavailable(_)) {
        health.changed().await.unwrap();
    }
    let mut result = Box::pin(ticket.wait());
    assert!(futures_util::poll!(&mut result).is_pending());
    let (rejected, _) = input(2, &started, true);
    assert!(
        jobs.submit(&handler, rejected, Default::default())
            .await
            .is_err()
    );
    store.fault.store(0, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert_eq!(result.await.unwrap(), 1);
    assert!(starts.try_recv().is_err());
    jobs.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn shutdown_reports_late_terminal_failure_and_retains_recovery() {
    let (store, jobs, handler) = setup(1).await;
    let (started, mut starts) = mpsc::unbounded_channel();
    let (one, finish) = input(1, &started, false);
    let ticket = jobs
        .submit(&handler, one, Default::default())
        .await
        .unwrap();
    starts.recv().await.unwrap();
    let mut updates = ticket.subscribe();
    let mut shutdown = Box::pin(jobs.shutdown());
    assert!(futures_util::poll!(&mut shutdown).is_pending());
    while updates.borrow().status != JobStatus::Cancelling {
        updates.changed().await.unwrap();
    }
    store.fault.store(1, Ordering::SeqCst);
    finish.send(()).unwrap();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), shutdown)
            .await
            .unwrap(),
        Err(JobsError::Store(_))
    ));
    assert!(jobs.has_active_work());
    assert!(!updates.borrow().status.is_terminal());
    store.fault.store(0, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert!(matches!(
        ticket.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    jobs.shutdown().await.unwrap();
    assert_eq!(jobs.health(), JobsHealth::Closed);
}

#[tokio::test(start_paused = true)]
async fn group_invalidation_keeps_drain_handle_during_storage_failure() {
    let (store, jobs, handler) = setup(1).await;
    let group = jobs.create_group().await.unwrap();
    let (started, mut starts) = mpsc::unbounded_channel();
    let (one, finish) = input(1, &started, false);
    let ticket = jobs
        .submit(
            &handler,
            one,
            SubmitOptions {
                groups: vec![group.clone()],
            },
        )
        .await
        .unwrap();
    starts.recv().await.unwrap();
    let (two, _finish_two) = input(2, &started, false);
    let queued = jobs
        .submit(
            &handler,
            two,
            SubmitOptions {
                groups: vec![group.clone()],
            },
        )
        .await
        .unwrap();
    store.fault.store(1, Ordering::SeqCst);
    let cancellation = jobs
        .invalidate_group(&group, JobError::new("changed", "changed"))
        .await
        .unwrap();
    assert!(matches!(
        cancellation.persistence_error(),
        Some(JobsError::Store(_))
    ));
    let mut drain = Box::pin(cancellation.wait());
    assert!(futures_util::poll!(&mut drain).is_pending());
    finish.send(()).unwrap();
    store.fault.store(0, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    drain.await.unwrap();
    assert!(matches!(
        ticket.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    assert!(matches!(
        queued.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    let (three, _) = input(3, &started, true);
    assert!(matches!(
        jobs.submit(
            &handler,
            three,
            SubmitOptions {
                groups: vec![group]
            }
        )
        .await
        .err()
        .unwrap()
        .source,
        JobsError::InvalidGroup
    ));
    assert!(starts.try_recv().is_err());
    jobs.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn unsettled_admission_is_not_retried_after_absence_or_read_failure() {
    let (store, jobs, handler) = setup(1).await;
    let (started, mut starts) = mpsc::unbounded_channel();
    store.fault.store(3, Ordering::SeqCst);
    let (one, _finish) = input(1, &started, false);
    assert!(
        jobs.submit(&handler, one, Default::default())
            .await
            .is_err()
    );
    let writes = store.writes.load(Ordering::SeqCst);
    store.fault.store(0, Ordering::SeqCst);
    // No row yet: the original unsettled write could still complete later.
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    jobs.list(Default::default()).await.unwrap();
    assert_eq!(store.writes.load(Ordering::SeqCst), writes);
    store.read_fault.store(1, Ordering::SeqCst);
    tokio::time::advance(std::time::Duration::from_secs(2)).await;
    jobs.list(Default::default()).await.unwrap();
    assert_eq!(store.writes.load(Ordering::SeqCst), writes);
    let pending = store.unsettled.lock().unwrap().take().unwrap();
    let id = pending.id.clone();
    store.rows.lock().unwrap().insert(id.clone(), pending);
    store.read_fault.store(0, Ordering::SeqCst);
    let mut health = jobs.subscribe_health();
    while *health.borrow() != JobsHealth::Ready {
        health.changed().await.unwrap();
    }
    assert_eq!(
        jobs.get(id).await.unwrap().unwrap().status,
        JobStatus::Cancelled
    );
    assert!(starts.try_recv().is_err());
    jobs.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn failed_running_snapshot_recovery_starts_handler_exactly_once() {
    let (store, jobs, handler) = setup(1).await;
    let (started, mut starts) = mpsc::unbounded_channel();
    store.fault.store(5, Ordering::SeqCst);
    let (one, finish) = input(1, &started, false);
    let ticket = jobs
        .submit(&handler, one, Default::default())
        .await
        .unwrap();
    let mut health = jobs.subscribe_health();
    while !matches!(*health.borrow(), JobsHealth::StorageUnavailable(_)) {
        health.changed().await.unwrap();
    }
    assert!(starts.try_recv().is_err());
    store.fault.store(0, Ordering::SeqCst);
    assert_eq!(starts.recv().await, Some(1));
    finish.send(()).unwrap();
    assert_eq!(ticket.wait().await.unwrap(), 1);
    assert!(starts.try_recv().is_err());
    store.delete_fault.store(1, Ordering::SeqCst);
    assert!(jobs.clear_completed().await.is_err());
    assert_eq!(store.rows.lock().unwrap().len(), 1);
    store.delete_fault.store(0, Ordering::SeqCst);
    assert_eq!(jobs.clear_completed().await.unwrap().deleted, 1);
    jobs.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_interrupts_without_inputs_and_clear_preserves_active() {
    let store = Arc::new(Store::default());
    let now = semantic_data::DateTime::now_utc();
    for (number, status) in [JobStatus::Queued, JobStatus::Running, JobStatus::Cancelling]
        .into_iter()
        .enumerate()
    {
        let row = JobRecord {
            id: JobId(number.to_string()),
            kind: JobKindId("uninstalled.kind".into()),
            status,
            progress: Default::default(),
            error: None,
            created_at: now,
            updated_at: now,
            started_at: (number != 0).then_some(now),
            finished_at: None,
            snapshot_seq: 3,
        };
        store.put(&row).await.unwrap();
    }
    let mut builder = JobsBuilder::new();
    let handler = builder.register(Handler::new()).unwrap();
    let jobs = ScopeJobs::open(store.clone(), builder.build(), Default::default())
        .await
        .unwrap();
    assert!(
        jobs.list(Default::default())
            .await
            .unwrap()
            .records
            .iter()
            .all(|r| r.status == JobStatus::Interrupted && r.snapshot_seq == 4)
    );
    let (started, mut starts) = mpsc::unbounded_channel();
    let (one, finish) = input(1, &started, false);
    let ticket = jobs
        .submit(&handler, one, Default::default())
        .await
        .unwrap();
    starts.recv().await;
    assert_eq!(jobs.clear_completed().await.unwrap().deleted, 3);
    assert!(jobs.get(ticket.id.clone()).await.unwrap().is_some());
    finish.send(()).unwrap();
    ticket.wait().await.unwrap();
    assert_eq!(jobs.clear_completed().await.unwrap().deleted, 1);
    assert_eq!(jobs.clear_completed().await.unwrap().deleted, 0);
    jobs.shutdown().await.unwrap();
}

#[tokio::test]
async fn retention_deletes_oldest_terminal_rows_with_stable_ties() {
    let store = Arc::new(Store::default());
    let now = semantic_data::DateTime::now_utc();
    for id in ["e", "b", "d", "a", "c"] {
        store
            .put(&JobRecord {
                id: JobId(id.into()),
                kind: JobKindId("historical.kind".into()),
                status: JobStatus::Cancelled,
                progress: Default::default(),
                error: None,
                created_at: now,
                updated_at: now,
                started_at: None,
                finished_at: Some(now),
                snapshot_seq: 1,
            })
            .await
            .unwrap();
    }
    let jobs = ScopeJobs::open(
        store.clone(),
        Default::default(),
        JobsConfig {
            history_threshold: 2.try_into().unwrap(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .rows
            .lock()
            .unwrap()
            .keys()
            .map(|id| id.0.clone())
            .collect::<Vec<_>>(),
        vec!["d", "e"]
    );
    jobs.shutdown().await.unwrap();
}
