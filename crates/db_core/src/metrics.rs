//! Query and write execution metrics.
//!
//! Queries collect counters into a shared [`MetricsCollector`] while they
//! run: the storage read layer counts scanned rows, point reads and index
//! probes, operators count their own work (sort retention, hash groups, join
//! sides) and the executor counts emitted rows. Counters are relaxed atomics
//! updated per row or per batch without allocation, so collection is cheap
//! enough to run on every query that asks for it.
//!
//! [`MetricsCollector::with_operator_stats`] additionally records rows and
//! time per plan operator for `EXPLAIN ANALYZE` (see [`QueryAnalysis`]).

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::Stream;
use semantic_data::value::Value;

/// Counters and timings of one query execution.
///
/// Backends that do not collect metrics report all counters as zero.
#[derive(facet::Facet, Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryMetrics {
    /// Rows read by collection scans.
    pub rows_scanned: u64,
    /// Stored rows decoded (scanned or read by id).
    pub rows_decoded: u64,
    /// Rows read by id (index lookups, local reference resolution).
    pub point_reads: u64,
    /// Index scans issued (one per key, key range or token probe).
    pub index_probes: u64,
    /// Index entries read by index scans.
    pub index_entries_read: u64,
    /// Rows returned by the query.
    pub rows_emitted: u64,
    /// Batches returned by the query.
    pub batches: u64,
    /// Rows held by sort operators (bounded by `OFFSET + LIMIT` for top-N
    /// sorts).
    pub sort_rows_retained: u64,
    /// Groups built by aggregations.
    pub hash_groups: u64,
    pub joins: JoinMetrics,
    /// Wall time of the whole query, including parsing and planning.
    pub elapsed: Duration,
    /// Time spent planning.
    pub plan_elapsed: Duration,
    /// Time spent executing the plan.
    pub exec_elapsed: Duration,
    /// Collection accesses made by the plan, one entry per distinct access
    /// path (rows of repeated accesses are summed).
    pub access_paths: Vec<AccessPathUse>,
}

impl QueryMetrics {
    /// These metrics with all timings zeroed, for comparing counters of
    /// different executions.
    pub fn without_timings(mut self) -> Self {
        self.elapsed = Duration::ZERO;
        self.plan_elapsed = Duration::ZERO;
        self.exec_elapsed = Duration::ZERO;
        self
    }
}

/// Rows consumed by join operators.
#[derive(facet::Facet, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JoinMetrics {
    /// Rows streamed against the build side (or used as probe keys).
    pub probe_rows: u64,
    /// Rows materialized into the build side of a join.
    pub build_rows: u64,
}

/// How a query read a collection.
#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum AccessPathKind {
    FullScan,
    FilteredScan,
    IndexLookup,
    IndexRange,
    TextSearch,
    /// Index probes of an index nested loop join.
    IndexProbe,
}

impl AccessPathKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FullScan => "full_scan",
            Self::FilteredScan => "filtered_scan",
            Self::IndexLookup => "index_lookup",
            Self::IndexRange => "index_range",
            Self::TextSearch => "text_search",
            Self::IndexProbe => "index_probe",
        }
    }
}

/// One collection access of a query and the rows it produced.
#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct AccessPathUse {
    /// Collection (or source binding) read.
    pub source: String,
    pub kind: AccessPathKind,
    /// Index used, when known.
    pub index: Option<String>,
    /// Rows produced by the access.
    pub rows: u64,
}

/// Measured execution of a plan (`EXPLAIN ANALYZE`).
#[derive(facet::Facet, Debug, Clone, Default, PartialEq)]
pub struct QueryAnalysis {
    pub metrics: QueryMetrics,
    /// Per-operator statistics in plan pre-order. Empty when the backend
    /// does not instrument operators.
    pub operator_stats: Vec<OperatorStats>,
}

/// Rows and time of one plan operator.
#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct OperatorStats {
    /// Position of the operator in the plan: `0` is the root, `0.1` its
    /// second input.
    pub path: String,
    /// Operator label (see [`crate::PhysicalPlan::node_label`]).
    pub operator: String,
    /// Rows the operator produced.
    pub rows_out: u64,
    /// Time spent producing the operator's rows, including its inputs.
    pub elapsed: Duration,
    /// Operator specific counters and details.
    pub extra: BTreeMap<String, Value>,
}

impl OperatorStats {
    /// Nesting depth of the operator (the root has depth 0).
    pub fn depth(&self) -> usize {
        self.path.matches('.').count()
    }
}

/// Counters of one write (batch or transaction commit).
///
/// Backends that do not collect write metrics report all counters as zero.
#[derive(facet::Facet, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteMetrics {
    /// Rows read by id.
    pub point_reads: u64,
    /// Index scans issued.
    pub index_reads: u64,
    /// Collection scans made by predicate mutations without an index.
    pub collection_scans: u64,
    /// Times the write fell back to materializing whole collections. The
    /// embedded engine no longer materializes collections for data writes,
    /// so it always reports zero.
    pub fallback_scans: u64,
    /// Rows visited by scans and materialized collections.
    pub visited_rows: u64,
    /// Storage write operations committed.
    pub storage_writes: u64,
    /// Attempts the write took, including retried conflicts (zero when the
    /// backend does not report them).
    #[facet(default)]
    pub attempts: u64,
    /// Attempts that ended in a conflict with a concurrent commit and were
    /// retried.
    #[facet(default)]
    pub conflicts: u64,
}

impl WriteMetrics {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// These counters with the attempts and conflicts of `transaction`.
    pub fn with_transaction(mut self, transaction: crate::TransactionMetrics) -> Self {
        self.attempts = u64::from(transaction.attempts);
        self.conflicts = u64::from(transaction.conflicts);
        self
    }
}

/// Shared, thread-safe sink for the metrics of one query execution.
#[derive(Debug, Default)]
pub struct MetricsCollector {
    rows_scanned: AtomicU64,
    rows_decoded: AtomicU64,
    point_reads: AtomicU64,
    index_probes: AtomicU64,
    index_entries_read: AtomicU64,
    rows_emitted: AtomicU64,
    batches: AtomicU64,
    sort_rows_retained: AtomicU64,
    hash_groups: AtomicU64,
    join_probe_rows: AtomicU64,
    join_build_rows: AtomicU64,
    access_paths: Mutex<Vec<AccessPathSlot>>,
    /// Operator slots by plan path; `None` unless operators are profiled.
    operators: Option<Mutex<BTreeMap<Vec<u32>, Arc<OperatorSlot>>>>,
}

#[derive(Debug)]
struct AccessPathSlot {
    source: String,
    kind: AccessPathKind,
    index: Option<String>,
    rows: Arc<AtomicU64>,
}

fn add(counter: &AtomicU64, value: u64) {
    counter.fetch_add(value, Ordering::Relaxed);
}

impl MetricsCollector {
    /// A collector of query-wide counters.
    pub fn new() -> Self {
        Self::default()
    }

    /// A collector that also records per-operator statistics.
    pub fn with_operator_stats() -> Self {
        Self {
            operators: Some(Mutex::default()),
            ..Self::default()
        }
    }

    /// Whether per-operator statistics are recorded.
    pub fn profiles_operators(&self) -> bool {
        self.operators.is_some()
    }

    pub fn add_rows_scanned(&self, rows: u64) {
        add(&self.rows_scanned, rows);
    }

    pub fn add_rows_decoded(&self, rows: u64) {
        add(&self.rows_decoded, rows);
    }

    pub fn add_point_reads(&self, reads: u64) {
        add(&self.point_reads, reads);
    }

    pub fn add_index_probes(&self, probes: u64) {
        add(&self.index_probes, probes);
    }

    pub fn add_index_entries_read(&self, entries: u64) {
        add(&self.index_entries_read, entries);
    }

    pub fn add_emitted(&self, rows: u64) {
        add(&self.rows_emitted, rows);
        add(&self.batches, 1);
    }

    pub fn add_sort_rows_retained(&self, rows: u64) {
        add(&self.sort_rows_retained, rows);
    }

    pub fn add_hash_groups(&self, groups: u64) {
        add(&self.hash_groups, groups);
    }

    pub fn add_join_probe_rows(&self, rows: u64) {
        add(&self.join_probe_rows, rows);
    }

    pub fn add_join_build_rows(&self, rows: u64) {
        add(&self.join_build_rows, rows);
    }

    /// Register a collection access; returns the counter of the rows it
    /// produces. Accesses with the same source, kind and index share one
    /// counter.
    pub fn register_access_path(
        &self,
        source: &str,
        kind: AccessPathKind,
        index: Option<&str>,
    ) -> Arc<AtomicU64> {
        let mut paths = self
            .access_paths
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(slot) = paths.iter().find(|slot| {
            slot.kind == kind && slot.source == source && slot.index.as_deref() == index
        }) {
            return slot.rows.clone();
        }
        let rows = Arc::new(AtomicU64::new(0));
        paths.push(AccessPathSlot {
            source: source.to_string(),
            kind,
            index: index.map(ToOwned::to_owned),
            rows: rows.clone(),
        });
        rows
    }

    /// The statistics slot of the operator at `path`, registered with
    /// `label` on first use; `None` unless operators are profiled.
    pub(crate) fn operator_slot(
        &self,
        path: &[u32],
        label: impl FnOnce() -> String,
    ) -> Option<Arc<OperatorSlot>> {
        let operators = self.operators.as_ref()?;
        let mut operators = operators.lock().unwrap_or_else(|err| err.into_inner());
        Some(
            operators
                .entry(path.to_vec())
                .or_insert_with(|| Arc::new(OperatorSlot::new(label())))
                .clone(),
        )
    }

    /// The counters collected so far. Timings are left zero for the caller
    /// to fill in.
    pub fn snapshot(&self) -> QueryMetrics {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        let access_paths = self
            .access_paths
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .map(|slot| AccessPathUse {
                source: slot.source.clone(),
                kind: slot.kind,
                index: slot.index.clone(),
                rows: load(&slot.rows),
            })
            .collect();
        QueryMetrics {
            rows_scanned: load(&self.rows_scanned),
            rows_decoded: load(&self.rows_decoded),
            point_reads: load(&self.point_reads),
            index_probes: load(&self.index_probes),
            index_entries_read: load(&self.index_entries_read),
            rows_emitted: load(&self.rows_emitted),
            batches: load(&self.batches),
            sort_rows_retained: load(&self.sort_rows_retained),
            hash_groups: load(&self.hash_groups),
            joins: JoinMetrics {
                probe_rows: load(&self.join_probe_rows),
                build_rows: load(&self.join_build_rows),
            },
            elapsed: Duration::ZERO,
            plan_elapsed: Duration::ZERO,
            exec_elapsed: Duration::ZERO,
            access_paths,
        }
    }

    /// Statistics of the profiled operators, in plan pre-order.
    pub fn operator_stats(&self) -> Vec<OperatorStats> {
        let Some(operators) = &self.operators else {
            return Vec::new();
        };
        operators
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .map(|(path, slot)| slot.stats(path))
            .collect()
    }
}

/// Live statistics of one profiled operator.
#[derive(Debug)]
pub(crate) struct OperatorSlot {
    label: String,
    rows_out: AtomicU64,
    elapsed_nanos: AtomicU64,
    extra: Mutex<BTreeMap<String, Value>>,
}

impl OperatorSlot {
    fn new(label: String) -> Self {
        Self {
            label,
            rows_out: AtomicU64::new(0),
            elapsed_nanos: AtomicU64::new(0),
            extra: Mutex::default(),
        }
    }

    fn extra(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Value>> {
        self.extra.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Set the operator detail `key`.
    pub(crate) fn set_extra(&self, key: &str, value: Value) {
        self.extra().insert(key.to_string(), value);
    }

    /// Add `value` to the operator counter `key`.
    pub(crate) fn add_extra(&self, key: &str, value: u64) {
        let mut extra = self.extra();
        let entry = extra.entry(key.to_string()).or_insert(Value::U64(0));
        if let Value::U64(current) = entry {
            *current += value;
        }
    }

    fn stats(&self, path: &[u32]) -> OperatorStats {
        OperatorStats {
            path: operator_path_string(path),
            operator: self.label.clone(),
            rows_out: self.rows_out.load(Ordering::Relaxed),
            elapsed: Duration::from_nanos(self.elapsed_nanos.load(Ordering::Relaxed)),
            extra: self.extra().clone(),
        }
    }
}

/// Log a query at `warn` level when it ran longer than `threshold`, with the
/// one-line summary of its plan (when known) and its metrics.
pub(crate) fn log_slow_query(
    threshold: Option<Duration>,
    metrics: &QueryMetrics,
    plan: Option<&crate::PhysicalPlan>,
) {
    let Some(threshold) = threshold else {
        return;
    };
    if metrics.elapsed <= threshold {
        return;
    }
    let plan = plan.map_or_else(|| "-".to_string(), crate::PhysicalPlan::summary);
    tracing::warn!(
        elapsed = ?metrics.elapsed,
        threshold = ?threshold,
        plan = %plan,
        rows_emitted = metrics.rows_emitted,
        rows_scanned = metrics.rows_scanned,
        rows_decoded = metrics.rows_decoded,
        point_reads = metrics.point_reads,
        index_probes = metrics.index_probes,
        index_entries_read = metrics.index_entries_read,
        sort_rows_retained = metrics.sort_rows_retained,
        hash_groups = metrics.hash_groups,
        "Slow query"
    );
}

/// Render an operator path: `0` for the root, `0.1` for its second input.
pub(crate) fn operator_path_string(path: &[u32]) -> String {
    let mut out = String::from("0");
    for index in path {
        out.push('.');
        out.push_str(&index.to_string());
    }
    out
}

/// Stream of row batches that records the rows it yields and the time spent
/// polling it in an [`OperatorSlot`].
pub(crate) struct ProfiledStream<S> {
    inner: S,
    slot: Arc<OperatorSlot>,
}

impl<S> ProfiledStream<S> {
    pub(crate) fn new(inner: S, slot: Arc<OperatorSlot>) -> Self {
        Self { inner, slot }
    }
}

impl<S, T, E> Stream for ProfiledStream<S>
where
    S: Stream<Item = Result<Vec<T>, E>> + Unpin,
{
    type Item = Result<Vec<T>, E>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let started = Instant::now();
        let poll = Pin::new(&mut self.inner).poll_next(cx);
        let elapsed = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        add(&self.slot.elapsed_nanos, elapsed);
        if let Poll::Ready(Some(Ok(batch))) = &poll {
            add(&self.slot.rows_out, batch.len() as u64);
        }
        poll
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn access_paths_share_counters_per_distinct_access() {
        let collector = MetricsCollector::new();
        let a = collector.register_access_path("items", AccessPathKind::FullScan, None);
        let b = collector.register_access_path("items", AccessPathKind::FullScan, None);
        let c = collector.register_access_path("items", AccessPathKind::IndexRange, Some("by_n"));
        a.fetch_add(2, Ordering::Relaxed);
        b.fetch_add(3, Ordering::Relaxed);
        c.fetch_add(1, Ordering::Relaxed);
        let metrics = collector.snapshot();
        assert_eq!(
            metrics.access_paths,
            vec![
                AccessPathUse {
                    source: "items".into(),
                    kind: AccessPathKind::FullScan,
                    index: None,
                    rows: 5,
                },
                AccessPathUse {
                    source: "items".into(),
                    kind: AccessPathKind::IndexRange,
                    index: Some("by_n".into()),
                    rows: 1,
                },
            ]
        );
    }

    #[test]
    fn operator_stats_are_recorded_only_when_profiling() {
        assert!(
            MetricsCollector::new()
                .operator_slot(&[], String::new)
                .is_none()
        );

        let collector = MetricsCollector::with_operator_stats();
        let root = collector.operator_slot(&[], || "Project".into()).unwrap();
        let child = collector
            .operator_slot(&[0], || "Scan(items)".into())
            .unwrap();
        child.add_extra("probes", 2);
        child.add_extra("probes", 3);
        let batches = futures::stream::iter(vec![Ok::<_, ()>(vec![1, 2]), Ok(vec![3])]);
        let rows =
            futures::executor::block_on(ProfiledStream::new(batches, root).collect::<Vec<_>>());
        assert_eq!(rows.len(), 2);

        let stats = collector.operator_stats();
        assert_eq!(stats.len(), 2);
        assert_eq!((stats[0].path.as_str(), stats[0].rows_out), ("0", 3));
        assert_eq!(stats[0].operator, "Project");
        assert_eq!(stats[1].path, "0.0");
        assert_eq!(stats[1].depth(), 1);
        assert_eq!(stats[1].extra.get("probes"), Some(&Value::U64(5)));
    }
}
