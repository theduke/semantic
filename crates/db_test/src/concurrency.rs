//! Concurrency invariants of a backend under concurrent writers, readers
//! and a change-feed subscriber.
//!
//! [`run_concurrency`] runs writer tasks committing interactive
//! transactions (random upserts and deletes of overlapping ids plus
//! read-modify-write increments of shared counters), retrying conflicts,
//! next to reader tasks running full scans and index queries and one
//! change-feed subscriber. Afterwards:
//!
//! - replaying the committed transactions in revision order yields exactly
//!   the final state, and no increment was lost;
//! - every read observed the state after some prefix of that history, and
//!   the prefixes observed by one reader never go backwards (no torn or
//!   stale-after-newer reads);
//! - the change feed delivered every commit in revision order with the
//!   rows of the replayed states (gaps only where it reported `Lagged`);
//! - `verify` reports no problems.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt as _;
use semantic_data::query::{BinaryOp, Expr, OrderBy, SelectQuery, SortDirection};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::catalog::IntegrityMode;
use semantic_db_core::{
    ChangeFeedItem, ChangeKind, ChangeSubscriptionOptions, Db, DbError, DdlBatch,
    DdlCollectionKind, DdlOperation, TransactionOptions, VerifyOptions, evaluate_filter_expr,
};

use crate::rng::{SeedGuard, TestRng};

const ITEMS: &str = "concurrency_items";
const KEYS: u64 = 12;
const COUNTERS: u64 = 3;

/// Parameters of [`run_concurrency`].
#[derive(Debug, Clone, Copy)]
pub struct ConcurrencyConfig {
    pub seed: u64,
    pub writers: usize,
    pub transactions_per_writer: usize,
    pub readers: usize,
    pub reads_per_reader: usize,
}

impl ConcurrencyConfig {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            writers: 4,
            transactions_per_writer: crate::rng::env_usize("SEMANTIC_TEST_CONCURRENCY_TXS", 25),
            readers: 3,
            reads_per_reader: 30,
        }
    }
}

type State = BTreeMap<String, Object>;

/// The effects of one committed transaction.
#[derive(Debug, Clone)]
struct Commit {
    revision: u64,
    /// Final write per id: the new row, or `None` for a delete.
    writes: BTreeMap<String, Option<Object>>,
    increments: Vec<String>,
}

#[derive(Debug, Clone)]
enum Read {
    Scan,
    Name(String),
    Above(i64),
}

impl Read {
    fn query(&self) -> SelectQuery {
        let field = |name: &str| {
            Expr::Operand(semantic_data::query::Operand::Field(
                FieldPath::from_fields([name]),
            ))
        };
        let literal = |value: Value| Expr::Operand(semantic_data::query::Operand::Literal(value));
        let order = |name: &str| OrderBy {
            expr: field(name),
            direction: SortDirection::Asc,
        };
        let query = SelectQuery::new().with_collection(ITEMS);
        match self {
            Read::Scan => query.with_order_by(vec![order("id")]),
            Read::Name(name) => query
                .with_predicate(binary(
                    BinaryOp::Eq,
                    field("nick"),
                    literal(Value::String(name.clone())),
                ))
                .with_order_by(vec![order("id")]),
            Read::Above(n) => query
                .with_predicate(binary(BinaryOp::Gt, field("n"), literal(Value::I64(*n))))
                .with_order_by(vec![order("n"), order("id")]),
        }
    }

    /// The rows the query returns on `state`.
    fn evaluate(&self, state: &State) -> Vec<Object> {
        let predicate = self.query().predicate.map(semantic_db_core::Expr::from);
        let mut rows = state
            .values()
            .filter(|row| {
                predicate
                    .as_ref()
                    .is_none_or(|predicate| evaluate_filter_expr(*row, predicate))
            })
            .cloned()
            .collect::<Vec<_>>();
        if matches!(self, Read::Above(_)) {
            rows.sort_by(|a, b| {
                a.get("n")
                    .cmp(&b.get("n"))
                    .then(a.get("id").cmp(&b.get("id")))
            });
        }
        rows
    }
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

async fn setup(db: &Db) {
    let index = |name: &str, field: &str, kind| DdlOperation::UpsertIndex {
        name: name.into(),
        collection: ITEMS.into(),
        field: field.into(),
        unique: false,
        kind,
        extra_fields: Vec::new(),
        predicate: None,
        analyzer: Default::default(),
    };
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertCollection {
                name: ITEMS.into(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            })
            .with_op(index(
                "concurrency_name",
                "nick",
                semantic_data::schema::IndexKind::Equality,
            ))
            .with_op(index(
                "concurrency_n",
                "n",
                semantic_data::schema::IndexKind::Range,
            )),
    )
    .await
    .unwrap();
}

/// Run one attempt of a writer transaction; `Ok(None)` on a conflict.
async fn attempt(
    db: &Db,
    rng: &mut TestRng,
    writer: usize,
    sequence: usize,
) -> Result<Option<Commit>, DbError> {
    let tx = db.begin_transaction(TransactionOptions::default()).await?;
    let mut writes = BTreeMap::new();
    let mut increments = Vec::new();
    for _ in 0..1 + rng.below(3) {
        match rng.below(6) {
            0..=2 => {
                let id = format!("k{}", rng.below(KEYS));
                let mut row = Object::new();
                row.insert("id", Value::String(id.clone()));
                row.insert("nick", Value::String(format!("n{}", rng.below(4))));
                row.insert("n", Value::I64(rng.below(10) as i64));
                row.insert("writer", Value::I64(writer as i64));
                row.insert("sequence", Value::I64(sequence as i64));
                tx.upsert(ITEMS.into(), id.clone(), row.clone()).await?;
                writes.insert(id, Some(row));
            }
            3 => {
                let id = format!("k{}", rng.below(KEYS));
                tx.delete(ITEMS.into(), id.clone()).await?;
                writes.insert(id, None);
            }
            _ => {
                let id = format!("c{}", rng.below(COUNTERS));
                let count = match tx.get(ITEMS.into(), id.clone()).await? {
                    Some(record) => record.object.get("count").and_then(|v| match v {
                        Value::I64(count) => Some(*count),
                        _ => None,
                    }),
                    None => Some(0),
                }
                .expect("counter rows hold a count");
                let mut row = Object::new();
                row.insert("id", Value::String(id.clone()));
                row.insert("count", Value::I64(count + 1));
                tx.upsert(ITEMS.into(), id.clone(), row.clone()).await?;
                writes.insert(id.clone(), Some(row));
                increments.push(id);
            }
        }
    }
    match tx.commit().await {
        Ok(commit) => Ok(Some(Commit {
            revision: commit.revision.expect("embedded commits report revisions"),
            writes,
            increments,
        })),
        Err(DbError::TransactionConflict(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Commit `transactions` transactions; returns them with the number of
/// conflicting attempts.
async fn writer(
    db: &Db,
    mut rng: TestRng,
    index: usize,
    transactions: usize,
) -> (Vec<Commit>, usize) {
    let mut commits = Vec::new();
    let mut conflicts = 0;
    for sequence in 0..transactions {
        let mut attempts = 0;
        loop {
            attempts += 1;
            assert!(
                attempts < 1_000,
                "writer {index}: transaction never committed"
            );
            // Each attempt re-runs the whole transaction body.
            let mut attempt_rng = rng.clone();
            match attempt(db, &mut attempt_rng, index, sequence).await {
                Ok(Some(commit)) => {
                    rng = attempt_rng;
                    commits.push(commit);
                    break;
                }
                Ok(None) | Err(DbError::TransactionConflict(_)) => conflicts += 1,
                Err(err) => panic!("writer {index}: {err}"),
            }
        }
    }
    (commits, conflicts)
}

async fn reader(db: &Db, mut rng: TestRng, reads: usize) -> Vec<(Read, Vec<Object>)> {
    let mut observed = Vec::new();
    for _ in 0..reads {
        let read = match rng.below(3) {
            0 => Read::Scan,
            1 => Read::Name(format!("n{}", rng.below(4))),
            _ => Read::Above(rng.below(10) as i64 - 1),
        };
        let rows = db.select(read.query()).await.unwrap();
        observed.push((read, rows));
    }
    observed
}

/// Wait for `duration` on a helper thread.
async fn sleep(duration: Duration) {
    let (sender, receiver) = futures::channel::oneshot::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        let _ = sender.send(());
    });
    let _ = receiver.await;
}

/// Replay `commits` in revision order; returns the committed revisions with
/// the state after each (starting with the empty state at revision 0).
fn replay(mut commits: Vec<Commit>) -> Vec<(u64, State)> {
    commits.sort_by_key(|commit| commit.revision);
    let mut history = vec![(0, State::new())];
    let apply = |state: &State, commit: &Commit| {
        let mut next = state.clone();
        for (id, row) in &commit.writes {
            match row {
                Some(row) => next.insert(id.clone(), row.clone()),
                None => next.remove(id),
            };
        }
        next
    };
    let mut index = 0;
    while index < commits.len() {
        // Commits that wrote nothing report their read revision, which they
        // share with the commit that created it; at most one commit of a
        // revision changes the state.
        let revision = commits[index].revision;
        let group = commits[index..]
            .iter()
            .take_while(|commit| commit.revision == revision)
            .collect::<Vec<_>>();
        index += group.len();
        let before = history.last().unwrap().1.clone();
        // The commit that created the revision (if any; the others read at
        // it and wrote nothing): applying it to the previous state must
        // leave every other commit of the group without effect.
        let consistent = |after: &State| group.iter().all(|commit| apply(after, commit) == *after);
        let creator = group
            .iter()
            .map(|commit| apply(&before, commit))
            .filter(|after| *after != before)
            .find(|after| consistent(after));
        let Some(after) = creator else {
            assert!(
                consistent(&before),
                "commits at revision {revision} are not serializable: {group:?}"
            );
            continue;
        };
        history.push((revision, after));
    }
    history
}

/// Run the concurrency scenario on a fresh database.
pub async fn run_concurrency(db: &Db, config: ConcurrencyConfig) {
    let _guard = SeedGuard::new("concurrency test", config.seed);
    setup(db).await;
    let mut rng = TestRng::new(config.seed);
    let options = ChangeSubscriptionOptions {
        include_payloads: true,
        include_internal: false,
        collections: vec![ITEMS.into()],
    };
    let feed = match db.subscribe_changes(options) {
        Ok(feed) => Some(feed),
        Err(err) if err.storage_kind() == Some(semantic_db_core::StorageErrorKind::Unsupported) => {
            None
        }
        Err(err) => panic!("subscribe: {err}"),
    };
    let done = Arc::new(Mutex::new(None::<u64>));

    let writers = (0..config.writers)
        .map(|index| writer(db, rng.fork(), index, config.transactions_per_writer))
        .collect::<Vec<_>>();
    let readers = (0..config.readers)
        .map(|_| reader(db, rng.fork(), config.reads_per_reader))
        .collect::<Vec<_>>();
    let subscriber = {
        let done = Arc::clone(&done);
        async move {
            let mut feed = feed?;
            let mut items = Vec::new();
            loop {
                let last = match items.last() {
                    Some(ChangeFeedItem::Event(event)) => event.revision,
                    Some(ChangeFeedItem::Lagged {
                        resume_revision, ..
                    }) => resume_revision.saturating_sub(1),
                    None => 0,
                };
                if done.lock().unwrap().is_some_and(|target| last >= target) {
                    return Some(items);
                }
                // Poll with a timeout so the loop notices when the writers
                // are done.
                let next = feed.next();
                let timeout = sleep(Duration::from_millis(100));
                futures::pin_mut!(next, timeout);
                match futures::future::select(next, timeout).await {
                    futures::future::Either::Left((Some(item), _)) => items.push(item),
                    futures::future::Either::Left((None, _)) => return Some(items),
                    futures::future::Either::Right(_) => {}
                }
            }
        }
    };
    let writers_done = {
        let done = Arc::clone(&done);
        async move {
            let (commits, conflicts): (Vec<_>, Vec<_>) =
                futures::future::join_all(writers).await.into_iter().unzip();
            let commits = commits.into_iter().flatten().collect::<Vec<_>>();
            let conflicts = conflicts.into_iter().sum::<usize>();
            let last = commits
                .iter()
                .map(|commit| commit.revision)
                .max()
                .unwrap_or(0);
            *done.lock().unwrap() = Some(last);
            (commits, conflicts)
        }
    };
    let deadline = async {
        // The subscriber stops once it saw the last commit; bound the wait.
        sleep(Duration::from_secs(120)).await;
    };
    let work = futures::future::join3(writers_done, futures::future::join_all(readers), subscriber);
    futures::pin_mut!(work, deadline);
    let ((commits, conflicts), reads, feed_items) =
        match futures::future::select(work, deadline).await {
            futures::future::Either::Left((result, _)) => result,
            futures::future::Either::Right(_) => panic!("concurrency test timed out"),
        };

    let increments = commits
        .iter()
        .flat_map(|commit| commit.increments.clone())
        .fold(BTreeMap::<String, i64>::new(), |mut counts, id| {
            *counts.entry(id).or_default() += 1;
            counts
        });
    let committed = commits.len();
    let history = replay(commits);
    eprintln!(
        "concurrency: {committed} commits ({} changed the state), {conflicts} conflicting \
         attempts, {} feed items",
        history.len() - 1,
        feed_items.as_ref().map_or(0, Vec::len)
    );
    let (_, final_state) = history.last().unwrap();

    // Final state: the replay, without lost increments.
    let rows = db.select(Read::Scan.query()).await.unwrap();
    assert_eq!(
        rows,
        Read::Scan.evaluate(final_state),
        "final state differs from the replay"
    );
    for (id, count) in &increments {
        assert_eq!(
            final_state.get(id).and_then(|row| row.get("count")),
            Some(&Value::I64(*count)),
            "lost increments of {id}"
        );
    }

    // Reads: consistent prefixes, monotonic per reader.
    for (reader, observed) in reads.iter().enumerate() {
        let mut lowest = 0;
        for (index, (read, rows)) in observed.iter().enumerate() {
            let Some(position) = (lowest..history.len())
                .find(|position| read.evaluate(&history[*position].1) == *rows)
            else {
                panic!(
                    "reader {reader} read {index} ({read:?}) observed a state that is no \
                     prefix of the history at or after revision {}: {rows:?}",
                    history[lowest].0
                );
            };
            lowest = position;
        }
    }

    // Change feed: every commit in order with the replayed rows.
    if let Some(feed_items) = feed_items {
        let states = history.iter().cloned().collect::<BTreeMap<_, _>>();
        let mut expected = history
            .iter()
            .skip(1)
            .map(|(revision, _)| *revision)
            .peekable();
        let mut previous = 0;
        for item in &feed_items {
            match item {
                ChangeFeedItem::Event(event) => {
                    assert!(event.revision > previous, "feed revisions out of order");
                    previous = event.revision;
                    assert_eq!(
                        expected.next(),
                        Some(event.revision),
                        "feed skipped or invented a commit"
                    );
                    let state = &states[&event.revision];
                    for change in &event.changes {
                        assert_eq!(change.collection, ITEMS);
                        let row = state.get(&change.id);
                        match change.kind {
                            ChangeKind::Deleted => assert!(row.is_none(), "{change:?}"),
                            ChangeKind::Created | ChangeKind::Updated => {
                                assert_eq!(change.after.as_ref(), row, "{change:?}")
                            }
                        }
                    }
                }
                ChangeFeedItem::Lagged {
                    resume_revision, ..
                } => {
                    while expected
                        .peek()
                        .is_some_and(|revision| revision < resume_revision)
                    {
                        expected.next();
                    }
                }
            }
        }
        assert_eq!(expected.next(), None, "feed missed commits");
    }

    let report = db.verify(VerifyOptions::all()).await.unwrap();
    assert!(report.is_ok(), "verify found problems:\n{report}");
}
