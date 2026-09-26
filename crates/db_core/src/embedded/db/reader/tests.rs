//! Execution metrics, `EXPLAIN ANALYZE` and the slow query log of the
//! embedded database.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use semantic_data::query::QueryInput;
use semantic_data::schema::IndexKind;

use super::*;
use crate::catalog::{CollectionKind, IndexDefinition};
use crate::embedded::EmbeddedBackend;
use crate::{AccessPathKind, AccessPathUse, Db, QueryMetrics};

/// Indexed collection: equality index on `code`, range index on `n`.
const ITEMS: &str = "metric_items";
/// Same rows as [`ITEMS`], without indexes.
const PLAIN: &str = "metric_plain";
/// One row per owner (`a`, `b`, `c`), joined to [`PLAIN`] by handle.
const OWNERS: &str = "metric_owners";
const ROWS: usize = 400;

/// Row `index`: `n` = index, `code` = `code-{index % 100}`, and `owner`
/// cycles through `a`, `b`, `c`.
fn item(index: usize) -> Object {
    Object::from_iter([
        ("id".to_string(), Value::String(format!("item-{index:04}"))),
        ("n".to_string(), Value::I64(index as i64)),
        (
            "owner".to_string(),
            Value::String(["a", "b", "c"][index % 3].to_string()),
        ),
        (
            "code".to_string(),
            Value::String(format!("code-{}", index % 100)),
        ),
    ])
}

fn index(lid: LocalCollectionId, name: &str, field: &str, kind: IndexKind) -> IndexDefinition {
    IndexDefinition {
        name: name.to_string(),
        collection: lid,
        fields: vec![field.to_string()],
        unique: false,
        kind,
        predicate: None,
        analyzer: Default::default(),
    }
}

fn populated_db(config: DbConfig) -> EmbeddedDb<crate::embedded::MemoryEntityStorage> {
    let mut db = EmbeddedDb::in_memory_with_config(config);
    let items = db
        .create_collection(ITEMS, CollectionKind::Polymorphic)
        .unwrap();
    db.create_index_definition(index(items, "by_code", "code", IndexKind::Equality))
        .unwrap();
    db.create_index_definition(index(items, "by_n", "n", IndexKind::Range))
        .unwrap();
    db.create_collection(PLAIN, CollectionKind::Polymorphic)
        .unwrap();
    db.create_collection(OWNERS, CollectionKind::Polymorphic)
        .unwrap();
    let mut operations = Vec::new();
    for collection in [ITEMS, PLAIN] {
        operations.extend((0..ROWS).map(|index| BatchOperation::Upsert {
            collection: collection.into(),
            id: format!("item-{index:04}"),
            object: item(index),
        }));
    }
    operations.extend(["a", "b", "c"].map(|name| BatchOperation::Upsert {
        collection: OWNERS.into(),
        id: format!("owner-{name}"),
        object: Object::from_iter([
            ("id".to_string(), Value::String(format!("owner-{name}"))),
            ("handle".to_string(), Value::String(name.to_string())),
        ]),
    }));
    db.transact(Batch { operations }).unwrap();
    db
}

fn select(sql: &str) -> SelectQuery {
    match crate::sql::parse_sql_query(sql, Default::default())
        .unwrap()
        .query
    {
        Query::Select(select) => select,
        other => panic!("expected a SELECT, got {other:?}"),
    }
}

/// Rows and explain (with metrics and operator statistics) of `sql`.
fn analyze(
    db: &EmbeddedDb<crate::embedded::MemoryEntityStorage>,
    sql: &str,
) -> (Vec<Object>, QueryExplain) {
    db.reader()
        .unwrap()
        .select_analyzed(select(sql), true)
        .unwrap()
}

fn metrics(explain: &QueryExplain) -> &QueryMetrics {
    &explain.analyze.as_ref().expect("analyzed").metrics
}

#[test]
fn full_scan_counts_scanned_rows_without_point_reads() {
    let db = populated_db(DbConfig::default());
    let (rows, explain) = analyze(&db, &format!("SELECT * FROM {PLAIN}"));
    assert_eq!(rows.len(), ROWS);
    let metrics = metrics(&explain);
    assert_eq!(metrics.rows_scanned, ROWS as u64);
    assert_eq!(metrics.rows_decoded, ROWS as u64);
    assert_eq!(metrics.point_reads, 0);
    assert_eq!(metrics.index_probes, 0);
    assert_eq!(metrics.rows_emitted, ROWS as u64);
    assert_eq!(metrics.batches, 1);
    assert_eq!(
        metrics.access_paths,
        vec![AccessPathUse {
            source: PLAIN.into(),
            kind: AccessPathKind::FullScan,
            index: None,
            rows: ROWS as u64,
        }]
    );
    assert!(metrics.elapsed >= metrics.exec_elapsed);
}

#[test]
fn indexed_lookup_reads_matches_by_id() {
    let db = populated_db(DbConfig::default());
    let (rows, explain) = analyze(&db, &format!("SELECT * FROM {ITEMS} WHERE code = 'code-7'"));
    let matches = ROWS / 100;
    assert_eq!(rows.len(), matches);
    let metrics = metrics(&explain);
    assert_eq!(metrics.rows_scanned, 0, "{}", explain.physical);
    assert_eq!(metrics.point_reads, matches as u64);
    assert_eq!(metrics.rows_decoded, matches as u64);
    assert_eq!(metrics.index_probes, 1);
    assert_eq!(metrics.index_entries_read, matches as u64);
    assert_eq!(metrics.access_paths.len(), 1);
    assert_eq!(metrics.access_paths[0].rows, matches as u64);
}

#[test]
fn range_scan_reads_only_the_range() {
    let db = populated_db(DbConfig::default());
    let (rows, explain) = analyze(
        &db,
        &format!("SELECT * FROM {ITEMS} WHERE n >= 5 AND n < 20"),
    );
    assert_eq!(rows.len(), 15);
    let metrics = metrics(&explain);
    assert_eq!(metrics.rows_scanned, 0, "{}", explain.physical);
    assert_eq!(metrics.index_entries_read, 15);
    assert_eq!(metrics.point_reads, 15);
    assert_eq!(
        metrics.access_paths,
        vec![AccessPathUse {
            source: ITEMS.into(),
            kind: AccessPathKind::IndexRange,
            index: Some("by_n".into()),
            rows: 15,
        }]
    );
    assert!(
        explain
            .summary()
            .contains("IndexRange(metric_items.by_n [5, 20))"),
        "{}",
        explain.summary()
    );
}

/// [`analyze`] on an owned reader, whose scans read rows lazily.
fn analyze_owned(
    db: &EmbeddedDb<crate::embedded::MemoryEntityStorage>,
    sql: &str,
) -> (Vec<Object>, QueryExplain) {
    db.owned_reader()
        .unwrap()
        .expect("owned snapshots")
        .select_analyzed(select(sql), true)
        .unwrap()
}

#[test]
fn unordered_limited_index_scans_stop_after_offset_plus_limit_rows() {
    let db = populated_db(DbConfig::default());
    // Range without residual: the scan reads offset + limit entries.
    let (rows, explain) = analyze_owned(
        &db,
        &format!("SELECT * FROM {ITEMS} WHERE n >= 5 AND n < 300 LIMIT 10 OFFSET 5"),
    );
    assert_eq!(rows.len(), 10);
    let range = metrics(&explain);
    assert_eq!(range.rows_scanned, 0, "{}", explain.physical);
    assert!(range.index_entries_read <= 15, "{range:?}");
    assert!(range.point_reads <= 15, "{range:?}");

    // Through a projection, with a residual predicate (the planner may probe
    // either index): the scan stops once offset + limit rows passed it.
    let (rows, explain) = analyze_owned(
        &db,
        &format!("SELECT id FROM {ITEMS} WHERE n >= 5 AND n < 300 AND owner = 'a' LIMIT 10"),
    );
    assert_eq!(rows.len(), 10);
    let residual = metrics(&explain);
    assert_eq!(residual.rows_scanned, 0, "{}", explain.physical);
    assert!(residual.point_reads <= 31, "{residual:?}");

    // Equality lookups read the matching ids, but only the rows taken.
    let (rows, explain) = analyze_owned(
        &db,
        &format!("SELECT * FROM {ITEMS} WHERE code = 'code-7' LIMIT 1"),
    );
    assert_eq!(rows.len(), 1);
    let lookup = metrics(&explain);
    assert_eq!(lookup.rows_scanned, 0, "{}", explain.physical);
    assert_eq!(lookup.point_reads, 1, "{lookup:?}");

    // A sort between the limit and the scan needs every row.
    let (rows, explain) = analyze_owned(
        &db,
        &format!("SELECT * FROM {ITEMS} WHERE n >= 5 AND n < 300 ORDER BY code LIMIT 10"),
    );
    assert_eq!(rows.len(), 10);
    assert_eq!(metrics(&explain).point_reads, 295);
}

#[test]
fn top_n_retains_only_offset_plus_limit_rows() {
    let db = populated_db(DbConfig::default());
    let (rows, explain) = analyze(
        &db,
        &format!("SELECT id, n FROM {PLAIN} ORDER BY n DESC LIMIT 10 OFFSET 5"),
    );
    assert_eq!(rows.len(), 10);
    let metrics = metrics(&explain);
    assert_eq!(metrics.sort_rows_retained, 15, "{}", explain.physical);
    assert_eq!(metrics.rows_scanned, ROWS as u64);
    assert_eq!(metrics.rows_emitted, 10);
}

#[test]
fn aggregate_counts_hash_groups() {
    let db = populated_db(DbConfig::default());
    let (rows, explain) = analyze(
        &db,
        &format!("SELECT owner, count(*) AS total FROM {PLAIN} GROUP BY owner"),
    );
    assert_eq!(rows.len(), 3);
    assert_eq!(metrics(&explain).hash_groups, 3);
}

#[test]
fn join_counts_build_and_probe_rows() {
    let db = populated_db(DbConfig::default());
    let (rows, explain) = analyze(
        &db,
        &format!(
            "SELECT p.id AS item, o.id AS owner FROM {PLAIN} AS p \
             JOIN {OWNERS}._ AS o ON p.owner = o.handle"
        ),
    );
    assert_eq!(rows.len(), ROWS);
    let joins = metrics(&explain).joins;
    assert_eq!(
        joins.build_rows + joins.probe_rows,
        (ROWS + 3) as u64,
        "{}",
        explain.physical
    );
    assert!(joins.build_rows > 0 && joins.probe_rows > 0);
}

#[test]
fn explain_analyze_reports_rows_of_every_operator() {
    let db = populated_db(DbConfig::default());
    let explain = db
        .reader()
        .unwrap()
        .explain_analyze_query(Query::Select(select(&format!(
            "SELECT id FROM {PLAIN} ORDER BY n LIMIT 10"
        ))))
        .unwrap();
    let analysis = explain.analyze.as_ref().unwrap();
    let mut expected = Vec::new();
    explain.physical.walk(&mut |path, plan| {
        expected.push((
            crate::metrics::operator_path_string(path),
            plan.node_label(),
        ));
    });
    let actual = analysis
        .operator_stats
        .iter()
        .map(|stats| (stats.path.clone(), stats.operator.clone()))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    for stats in &analysis.operator_stats {
        let expected_rows = if stats.operator.starts_with("Scan(") {
            ROWS as u64
        } else {
            10
        };
        assert_eq!(stats.rows_out, expected_rows, "{stats:?}");
    }
    let top_n = analysis
        .operator_stats
        .iter()
        .find(|stats| stats.operator.starts_with("TopN("))
        .expect("TopN operator");
    assert_eq!(top_n.extra.get("retained_rows"), Some(&Value::U64(10)));
    let rendered = explain.to_string();
    assert!(rendered.contains("rows=400"), "{rendered}");
    assert!(rendered.contains("rows_emitted=10"), "{rendered}");
}

#[test]
fn explain_analyze_rejects_mutations() {
    let db = populated_db(DbConfig::default());
    let delete = crate::sql::parse_sql_query(
        &format!("DELETE FROM {PLAIN} WHERE n = 1"),
        Default::default(),
    )
    .unwrap()
    .query;
    assert!(db.reader().unwrap().explain_analyze_query(delete).is_err());
}

/// Runtime running blocking work on a fresh thread, so the executor's
/// nested local pool never runs inside the test's executor.
struct ThreadRuntime;

impl crate::AsyncRuntime for ThreadRuntime {
    fn spawn_blocking_erased(
        &self,
        op: Box<dyn FnOnce() -> std::result::Result<Box<dyn std::any::Any + Send>, DbError> + Send>,
    ) -> futures::future::BoxFuture<
        'static,
        std::result::Result<Box<dyn std::any::Any + Send>, DbError>,
    > {
        let (sender, receiver) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(op());
        });
        Box::pin(async move {
            receiver
                .await
                .unwrap_or_else(|_| panic!("blocking thread panicked"))
        })
    }
}

fn backend_db(config: DbConfig) -> Db {
    Db::new(EmbeddedBackend::with_runtime(
        populated_db(config),
        Arc::new(ThreadRuntime),
    ))
}

fn sql_input(sql: String) -> QueryInput {
    QueryInput::Text {
        format: semantic_data::query::TextQueryFormat::Sql,
        query: sql,
        params: Default::default(),
    }
}

#[test]
fn query_with_metrics_matches_explain_analyze() {
    let db = backend_db(DbConfig::default());
    for sql in [
        format!("SELECT * FROM {ITEMS} WHERE n >= 3 AND n < 9"),
        format!("SELECT owner, count(*) AS total FROM {PLAIN} GROUP BY owner"),
        format!("SELECT id FROM {PLAIN} ORDER BY n DESC LIMIT 4"),
    ] {
        let (result, metrics) =
            futures::executor::block_on(db.query_with_metrics(sql_input(sql.clone()))).unwrap();
        let QueryResult::Select(rows) = result else {
            panic!("select result")
        };
        assert_eq!(metrics.rows_emitted, rows.len() as u64);
        let explain =
            futures::executor::block_on(db.explain_analyze(sql_input(sql.clone()))).unwrap();
        let analysis = explain.analyze.unwrap();
        assert!(!analysis.operator_stats.is_empty());
        assert_eq!(
            metrics.without_timings(),
            analysis.metrics.without_timings(),
            "{sql}"
        );
    }
}

#[test]
fn sql_explain_prefixes_select_plain_or_analyzed_explains() {
    let db = backend_db(DbConfig::default());
    let query = format!("SELECT * FROM {PLAIN} WHERE n < 3");
    let plain =
        futures::executor::block_on(db.explain(sql_input(format!("EXPLAIN {query}")))).unwrap();
    assert!(plain.analyze.is_none());
    let analyzed =
        futures::executor::block_on(db.explain(sql_input(format!("explain analyze {query}"))))
            .unwrap();
    assert_eq!(analyzed.analyze.unwrap().metrics.rows_emitted, 3);
    let analyzed =
        futures::executor::block_on(db.explain_analyze(sql_input(query.clone()))).unwrap();
    assert_eq!(analyzed.analyze.unwrap().metrics.rows_emitted, 3);
    assert!(futures::executor::block_on(db.query(sql_input(format!("EXPLAIN {query}")))).is_err());
}

#[test]
fn batch_replies_carry_write_metrics() {
    let db = backend_db(DbConfig::default());
    let upsert = |id: &str| {
        let mut object = item(1000);
        object.insert("id", Value::String(id.to_string()));
        semantic_data::query::BatchOperation::Upsert {
            collection: ITEMS.into(),
            id: id.into(),
            object,
        }
    };
    let reply = futures::executor::block_on(db.execute_batch_returning(
        semantic_data::query::Batch {
            operations: vec![upsert("new-1"), upsert("new-2")],
        },
        crate::BatchReturn::Changes,
    ))
    .unwrap();
    let metrics = *reply.metrics();
    assert_eq!(metrics.fallback_scans, 0);
    assert_eq!(metrics.point_reads, 2);
    assert!(metrics.storage_writes >= 2, "{metrics:?}");

    let outcome = futures::executor::block_on(db.execute_batch(semantic_data::query::Batch {
        operations: vec![upsert("new-3")],
    }))
    .unwrap();
    assert!(outcome.metrics.storage_writes >= 1, "{:?}", outcome.metrics);
    // Dataset replies hold the written rows only; no collection is loaded.
    assert!(
        outcome.metrics.visited_rows < ROWS as u64,
        "{:?}",
        outcome.metrics
    );
    assert_eq!(outcome.dataset[ITEMS].len(), 1);

    let commit = futures::executor::block_on(async {
        let tx = db
            .begin_transaction(crate::TransactionOptions::default())
            .await?;
        let mut object = item(1001);
        object.insert("id", Value::String("tx-1".into()));
        tx.upsert(ITEMS.into(), "tx-1".into(), object).await?;
        tx.commit().await
    })
    .unwrap();
    assert!(commit.metrics.storage_writes >= 1, "{:?}", commit.metrics);
}

/// Subscriber recording the `warn` events it receives as rendered fields.
#[derive(Clone, Default)]
struct WarnCapture {
    events: Arc<Mutex<Vec<String>>>,
}

impl tracing::Subscriber for WarnCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::WARN
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{}={:?} ", field.name(), value));
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.events.lock().unwrap().push(fields.0);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn slow_queries_are_logged_with_plan_and_metrics() {
    let capture = WarnCapture::default();
    let query = format!("SELECT * FROM {ITEMS} WHERE n >= 5 AND n < 20");

    let fast = backend_db(DbConfig::default());
    tracing::subscriber::with_default(capture.clone(), || {
        futures::executor::block_on(fast.query(sql_input(query.clone()))).unwrap();
    });
    assert!(capture.events.lock().unwrap().is_empty());

    let slow = backend_db(DbConfig {
        slow_query_threshold: Some(Duration::ZERO),
        ..DbConfig::default()
    });
    tracing::subscriber::with_default(capture.clone(), || {
        futures::executor::block_on(slow.query(sql_input(query.clone()))).unwrap();
    });
    let events = capture.events.lock().unwrap();
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    assert!(event.contains("message=Slow query"), "{event}");
    assert!(
        event.contains("plan=Project -> IndexRange(metric_items.by_n [5, 20))")
            || event.contains("IndexRange(metric_items.by_n [5, 20))"),
        "{event}"
    );
    assert!(event.contains("rows_emitted=15"), "{event}");
    assert!(event.contains("point_reads=15"), "{event}");
}
