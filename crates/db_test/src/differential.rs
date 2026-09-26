//! Randomised differential testing of database backends.
//!
//! [`run_differential`] generates a random schema (see `schema`) and a
//! random sequence of operations (see `ops`) — by-id and predicate writes,
//! class rows with validated references, interactive transactions with
//! savepoints and rollbacks, index DDL, invalid writes and reopens — and
//! applies each to every [`DiffTarget`] and to a trivial reference model
//! (see `model`). After every operation all targets must report the same
//! outcome (the same result or the same error kind) and the model's
//! prediction where it makes one; every `check_every` operations, a fixed
//! set of queries (full scans, every index shape, text matches, counts,
//! joins) must return identical rows on every target and match the model,
//! and `verify` must report no problems.
//!
//! The seed is taken from `SEMANTIC_TEST_SEED` when set and printed when a
//! run fails; see `docs/testing.md`.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;

use semantic_data::query::{
    BatchOperation, Expr, OrderBy, QueryInput, SelectQuery, SortDirection, TextQueryFormat,
};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::{Db, DbError, DdlBatch, QueryResult, TransactionOptions, VerifyOptions};

use crate::rng::{SeedGuard, TestRng};

mod model;
mod ops;
mod schema;

use model::Model;
use ops::{Op, TxStep};
use schema::{AUTHOR, CLASSES, LABEL, PLAIN, Schema, binary, field, literal};

/// A database under test.
pub struct DiffTarget {
    name: String,
    db: Option<Db>,
    open: Option<Box<dyn FnMut() -> Db + Send>>,
}

impl DiffTarget {
    /// A target that cannot be reopened (for example in-memory storage).
    pub fn new(name: impl Into<String>, db: Db) -> Self {
        Self {
            name: name.into(),
            db: Some(db),
            open: None,
        }
    }

    /// A target opened by `open`, which the test calls again to reopen the
    /// database after dropping the previous handle.
    pub fn reopenable(
        name: impl Into<String>,
        mut open: impl FnMut() -> Db + Send + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            db: Some(open()),
            open: Some(Box::new(open)),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn db(&self) -> &Db {
        self.db.as_ref().expect("database is open")
    }

    fn reopen(&mut self) {
        if let Some(open) = &mut self.open {
            // Release the storage (file locks) before opening it again.
            self.db = None;
            self.db = Some(open());
        }
    }
}

/// Parameters of one differential run.
#[derive(Debug, Clone, Copy)]
pub struct DiffConfig {
    pub seed: u64,
    /// Number of random operations.
    pub steps: usize,
    /// Run the query checks and `verify` after every `check_every`
    /// operations (and at the end).
    pub check_every: usize,
}

impl DiffConfig {
    /// `steps` operations with seed `seed`; `SEMANTIC_TEST_STEPS` and
    /// `SEMANTIC_TEST_CHECK_EVERY` override the defaults.
    pub fn new(seed: u64, steps: usize) -> Self {
        Self {
            seed,
            steps: crate::rng::env_usize("SEMANTIC_TEST_STEPS", steps),
            check_every: crate::rng::env_usize("SEMANTIC_TEST_CHECK_EVERY", 10).max(1),
        }
    }
}

/// Run one differential test over `targets` (at least one; a single target
/// is still checked against the model and `verify`).
pub async fn run_differential(config: DiffConfig, mut targets: Vec<DiffTarget>) {
    assert!(!targets.is_empty());
    let _guard = SeedGuard::new("differential test", config.seed);
    let mut rng = TestRng::new(config.seed);
    let mut run = Run {
        schema: Schema::random(&mut rng),
        model: Model::default(),
        step: 0,
        outcomes: Default::default(),
    };
    for target in &targets {
        target
            .db()
            .execute_ddl(run.schema.setup())
            .await
            .unwrap_or_else(|err| panic!("{}: schema setup: {err}", target.name));
        target
            .db()
            .activate_validation()
            .await
            .unwrap_or_else(|err| panic!("{}: activate validation: {err}", target.name));
    }
    for step in 0..config.steps {
        run.step = step;
        let op = ops::random_op(&mut rng, &run.model, &run.schema);
        run.execute(&op, &mut targets).await;
        if (step + 1) % config.check_every == 0 {
            run.check(&targets).await;
        }
    }
    run.check(&targets).await;
    eprintln!(
        "differential seed {}: outcomes (ok, error) per operation: {:?}",
        config.seed,
        run.outcomes.borrow()
    );
}

/// The kind of an error: its `DbError` variant name.
fn error_kind(error: &DbError) -> String {
    let debug = format!("{error:?}");
    debug
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Outcome of an operation on one target: a comparable result, or the
/// error kind (with the message for diagnostics).
type Outcome<T> = Result<T, (String, String)>;

fn outcome<T>(result: Result<T, DbError>) -> Outcome<T> {
    result.map_err(|err| (error_kind(&err), err.to_string()))
}

struct Run {
    schema: Schema,
    model: Model,
    step: usize,
    /// Successful and failed outcomes per operation kind, reported at the
    /// end so runs that stopped exercising something are noticed.
    outcomes: std::cell::RefCell<BTreeMap<String, (usize, usize)>>,
}

impl Run {
    /// Run `f` on every target and assert all outcomes agree; returns the
    /// common outcome.
    async fn on_all<'a, T, F, Fut>(
        &self,
        what: &str,
        targets: &'a [DiffTarget],
        f: F,
    ) -> Result<T, String>
    where
        T: Debug + PartialEq,
        F: Fn(&'a Db) -> Fut,
        Fut: Future<Output = Result<T, DbError>> + 'a,
    {
        let mut outcomes = Vec::with_capacity(targets.len());
        for target in targets {
            outcomes.push(outcome(f(target.db()).await));
        }
        self.agree(what, targets, outcomes)
    }

    fn agree<T: Debug + PartialEq>(
        &self,
        what: &str,
        targets: &[DiffTarget],
        mut outcomes: Vec<Outcome<T>>,
    ) -> Result<T, String> {
        let first = &outcomes[0];
        for (target, other) in targets.iter().zip(&outcomes).skip(1) {
            let same = match (first, other) {
                (Ok(a), Ok(b)) => a == b,
                (Err((a, _)), Err((b, _))) => a == b,
                _ => false,
            };
            assert!(
                same,
                "step {}: {what}: targets disagree\n  {}: {first:?}\n  {}: {other:?}",
                self.step, targets[0].name, target.name
            );
        }
        let label = match what.split_once(", tx step ") {
            // `<index> <step>`
            Some((_, step)) => format!(
                "tx {}",
                kind_label(step.split_once(' ').map_or(step, |(_, step)| step))
            ),
            None if what.ends_with(", commit") => "tx commit".to_string(),
            None => kind_label(what),
        };
        let mut counts = self.outcomes.borrow_mut();
        let entry = counts.entry(label).or_default();
        if outcomes[0].is_ok() {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
        outcomes.swap_remove(0).map_err(|(kind, _)| kind)
    }

    /// Assert the model's prediction matches the common engine outcome.
    fn expect<T: Debug, U: Debug + PartialEq>(
        &self,
        what: &str,
        engines: &Result<T, String>,
        model: &Result<U, &str>,
        engine_value: impl Fn(&T) -> U,
    ) {
        let matches = match (engines, model) {
            (Ok(engine), Ok(model)) => engine_value(engine) == *model,
            (Err(engine), Err(model)) => engine == model,
            _ => false,
        };
        assert!(
            matches,
            "step {}: {what}: engines and model disagree\n  engines: {engines:?}\n  model: {model:?}",
            self.step
        );
    }

    async fn execute(&mut self, op: &Op, targets: &mut [DiffTarget]) {
        let what = format!("{op:?}");
        match op {
            Op::Batch(operations) => {
                let batch = batch_of(operations);
                let engines = self
                    .on_all(&what, targets, |db| async {
                        db.execute_batch(batch.clone()).await.map(|out| out.stats)
                    })
                    .await;
                let mut next = self.model.clone();
                let model = next.apply_batch(operations, &self.schema);
                self.expect(&what, &engines, &model, |_| ());
                self.model = next;
            }
            Op::ClassBatch(operations) => {
                let batch = batch_of(operations);
                let engines = self
                    .on_all(&what, targets, |db| async {
                        db.execute_batch(batch.clone()).await.map(|out| out.stats)
                    })
                    .await;
                // Reference validation is not modelled: follow the engines.
                if engines.is_ok() {
                    self.model
                        .apply_batch(operations, &self.schema)
                        .expect("class rows have no unique indexes");
                }
            }
            Op::Invalid(operations) => {
                let batch = batch_of(operations);
                let engines = self
                    .on_all(&what, targets, |db| async {
                        db.execute_batch(batch.clone()).await.map(|out| out.stats)
                    })
                    .await;
                assert!(engines.is_err(), "step {}: {what}: accepted", self.step);
            }
            Op::Update(query) => {
                let engines = self
                    .on_all(&what, targets, |db| db.update_where(query.clone()))
                    .await;
                let mut next = self.model.clone();
                let model = next.update_where(query, &self.schema);
                self.expect(&what, &engines, &model, |stats| {
                    (stats.matched, stats.affected)
                });
                if model.is_ok() {
                    self.model = next;
                }
            }
            Op::Delete(query) => {
                let engines = self
                    .on_all(&what, targets, |db| db.delete_where(query.clone()))
                    .await;
                let model = Ok(self.model.delete_where(query));
                self.expect(&what, &engines, &model, |deleted| *deleted);
            }
            Op::Transaction { steps, commit } => {
                self.transaction(&what, targets, steps, *commit).await;
            }
            Op::AddIndex(index) => {
                let mut schema = self.schema.clone();
                schema.active.push(index.clone());
                let engines = self
                    .on_all(&what, targets, |db| async {
                        db.execute_ddl(DdlBatch::new().with_op(index.upsert()))
                            .await
                            .map(|_| ())
                    })
                    .await;
                // A unique index over existing duplicate values is rejected
                // and leaves the schema unchanged.
                let model = self.model.check_unique(&schema);
                self.expect(&what, &engines, &model, |_| ());
                if model.is_ok() {
                    self.schema = schema;
                }
            }
            Op::DropIndex(index) => {
                let engines = self
                    .on_all(&what, targets, |db| async {
                        db.execute_ddl(DdlBatch::new().with_op(index.delete()))
                            .await
                            .map(|_| ())
                    })
                    .await;
                assert!(engines.is_ok(), "step {}: {what}: {engines:?}", self.step);
                self.schema.active.retain(|active| active != index);
            }
            Op::Reopen => {
                for target in targets.iter_mut() {
                    target.reopen();
                }
            }
        }
    }

    async fn transaction(
        &mut self,
        what: &str,
        targets: &[DiffTarget],
        steps: &[TxStep],
        commit: bool,
    ) {
        let mut handles = Vec::new();
        for target in targets {
            handles.push(
                target
                    .db()
                    .begin_transaction(TransactionOptions::default())
                    .await
                    .unwrap_or_else(|err| panic!("{}: begin: {err}", target.name)),
            );
        }
        let mut model = self.model.clone();
        // Savepoints: the model state and every target's savepoint id.
        let mut savepoints = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            let what = format!("{what}, tx step {index} {step:?}");
            let mut outcomes = Vec::new();
            match step {
                TxStep::Upsert {
                    collection,
                    id,
                    object,
                } => {
                    for tx in &handles {
                        let result = tx
                            .upsert(collection.clone(), id.clone(), object.clone())
                            .await;
                        outcomes.push(outcome(result.map(|()| String::new())));
                    }
                    if self.agree(&what, targets, outcomes).is_ok() {
                        model
                            .collections
                            .entry(collection.clone())
                            .or_default()
                            .insert(id.clone(), object.clone());
                    }
                }
                TxStep::Delete { collection, id } => {
                    for tx in &handles {
                        let result = tx.delete(collection.clone(), id.clone()).await;
                        outcomes.push(outcome(result.map(|existed| format!("{existed}"))));
                    }
                    let engines = self.agree(&what, targets, outcomes);
                    let existed = model
                        .collections
                        .entry(collection.clone())
                        .or_default()
                        .remove(id)
                        .is_some();
                    self.expect(&what, &engines, &Ok::<_, &str>(existed.to_string()), |e| {
                        e.clone()
                    });
                }
                TxStep::Update(query) => {
                    for tx in &handles {
                        let result = tx.update_where(query.clone().into()).await;
                        outcomes.push(outcome(result.map(|result| format!("{:?}", result.stats))));
                    }
                    let engines = self.agree(&what, targets, outcomes);
                    // Unique indexes are checked when the transaction
                    // commits.
                    let mut next = model.clone();
                    let predicted = next.update_where(query, &Schema::without_indexes()).map(
                        |(matched, affected)| {
                            format!(
                                "{:?}",
                                semantic_db_core::MutationStats { matched, affected }
                            )
                        },
                    );
                    self.expect(&what, &engines, &predicted, |e| e.clone());
                    if predicted.is_ok() {
                        model = next;
                    }
                }
                TxStep::Select(collection) => {
                    let query = SelectQuery::new()
                        .with_collection(collection.clone())
                        .with_order_by(vec![ascending("id")]);
                    let mut selected = Vec::new();
                    for tx in &handles {
                        selected.push(outcome(tx.select(query.clone().into()).await));
                    }
                    let engines = self.agree(&what, targets, selected);
                    self.expect(
                        &what,
                        &engines,
                        &Ok::<_, &str>(model.select(collection, None, &[])),
                        |rows| rows.clone(),
                    );
                }
                TxStep::Savepoint => {
                    let mut ids = Vec::new();
                    for (target, tx) in targets.iter().zip(&handles) {
                        ids.push(
                            tx.savepoint()
                                .await
                                .unwrap_or_else(|err| panic!("{}: savepoint: {err}", target.name)),
                        );
                    }
                    savepoints.push((model.clone(), ids));
                }
                TxStep::RollbackTo(position) => {
                    if savepoints.is_empty() {
                        continue;
                    }
                    let position = position % savepoints.len();
                    savepoints.truncate(position + 1);
                    let (state, ids) = &savepoints[position];
                    for ((target, tx), id) in targets.iter().zip(&handles).zip(ids) {
                        tx.rollback_to(*id)
                            .await
                            .unwrap_or_else(|err| panic!("{}: rollback_to: {err}", target.name));
                    }
                    model = state.clone();
                }
            }
        }
        if !commit {
            for (target, tx) in targets.iter().zip(handles) {
                tx.rollback()
                    .await
                    .unwrap_or_else(|err| panic!("{}: rollback: {err}", target.name));
            }
            return;
        }
        let mut outcomes = Vec::new();
        for tx in handles {
            outcomes.push(outcome(tx.commit().await.map(|commit| commit.stats)));
        }
        let engines = self.agree(&format!("{what}, commit"), targets, outcomes);
        let predicted = model.check_unique(&self.schema);
        self.expect(&format!("{what}, commit"), &engines, &predicted, |_| ());
        if engines.is_ok() {
            self.model = model;
        }
    }

    /// Compare the query results of every target with each other and with
    /// the model, and verify every target.
    async fn check(&self, targets: &[DiffTarget]) {
        for collection in PLAIN.iter().chain([&CLASSES]) {
            self.check_select(targets, collection, None, &[]).await;
        }
        for collection in PLAIN {
            for (predicate, order) in check_predicates() {
                self.check_select(targets, collection, Some(predicate), &order)
                    .await;
            }
            let count = format!("SELECT COUNT(*) AS c FROM {collection}");
            let rows = self.check_sql(targets, &count).await;
            assert_eq!(
                rows[0].get("c").and_then(Value::as_u64_lossy),
                Some(self.model.rows(collection).count() as u64),
                "step {}: {count}",
                self.step
            );
            self.check_sql(
                targets,
                &format!("SELECT grp, COUNT(*) AS c FROM {collection} GROUP BY grp ORDER BY grp"),
            )
            .await;
        }
        self.check_sql(
            targets,
            "SELECT a.id AS aid, b.id AS bid, b.n AS bn FROM diff_a a JOIN diff_b b \
             ON a.ref = b.id ORDER BY a.id, b.id",
        )
        .await;
        for author in ["p0", "p1", "p2"] {
            let predicate = binary(
                semantic_data::query::BinaryOp::Eq,
                field(AUTHOR),
                literal(author.to_string()),
            );
            self.check_select(targets, CLASSES, Some(predicate), &[])
                .await;
        }
        let labels = binary(
            semantic_data::query::BinaryOp::Gte,
            field(LABEL),
            literal("L3".to_string()),
        );
        self.check_select(targets, CLASSES, Some(labels), &[LABEL])
            .await;

        for target in targets {
            let report = target
                .db()
                .verify(VerifyOptions::all())
                .await
                .unwrap_or_else(|err| panic!("step {}: {}: verify: {err}", self.step, target.name));
            assert!(
                report.is_ok(),
                "step {}: {}: verify found problems:\n{report}",
                self.step,
                target.name
            );
        }
    }

    async fn check_select(
        &self,
        targets: &[DiffTarget],
        collection: &str,
        predicate: Option<Expr>,
        order: &[&str],
    ) {
        let mut query = SelectQuery::new()
            .with_collection(collection)
            .with_order_by(
                order
                    .iter()
                    .chain(["id"].iter())
                    .map(|name| ascending(name))
                    .collect(),
            );
        if let Some(predicate) = &predicate {
            query = query.with_predicate(predicate.clone());
        }
        let what = format!("check {query:?}");
        let rows = self
            .on_all(&what, targets, |db| db.select(query.clone()))
            .await
            .unwrap_or_else(|kind| panic!("step {}: {what}: {kind}", self.step));
        let order = order
            .iter()
            .map(|name| FieldPath::from_fields([*name]))
            .collect::<Vec<_>>();
        let expected = self.model.select(collection, predicate.as_ref(), &order);
        if rows != expected {
            let ids = |rows: &[Object]| {
                rows.iter()
                    .map(|row| row.get("id").cloned().unwrap_or(Value::Null))
                    .collect::<Vec<_>>()
            };
            panic!(
                "step {}: {what}: engines and model disagree\n  engines: {:?}\n  model: {:?}\n  \
                 first differing engine row: {:?}\n  model row: {:?}",
                self.step,
                ids(&rows),
                ids(&expected),
                rows.iter()
                    .zip(&expected)
                    .find(|(a, b)| a != b)
                    .map(|(a, _)| a),
                rows.iter()
                    .zip(&expected)
                    .find(|(a, b)| a != b)
                    .map(|(_, b)| b),
            );
        }
    }

    async fn check_sql(&self, targets: &[DiffTarget], sql: &str) -> Vec<Object> {
        let what = format!("check {sql}");
        let result = self
            .on_all(&what, targets, |db| {
                db.query(QueryInput::Text {
                    format: TextQueryFormat::Sql,
                    query: sql.to_string(),
                    params: Default::default(),
                })
            })
            .await
            .unwrap_or_else(|kind| panic!("step {}: {what}: {kind}", self.step));
        match result {
            QueryResult::Select(rows) => rows,
            other => panic!("{what}: unexpected result {other:?}"),
        }
    }
}

/// The leading identifier of a debug-formatted operation.
fn kind_label(what: &str) -> String {
    what.split(|c: char| !c.is_alphanumeric())
        .find(|word| !word.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn batch_of(operations: &[BatchOperation]) -> semantic_data::query::Batch {
    operations
        .iter()
        .cloned()
        .fold(semantic_data::query::Batch::new(), |batch, op| {
            batch.with_op(op)
        })
}

fn ascending(name: &str) -> OrderBy {
    OrderBy {
        expr: field(name),
        direction: SortDirection::Asc,
    }
}

/// Predicates (with their order keys) of the query checks, covering every
/// index shape of the schema.
fn check_predicates() -> Vec<(Expr, Vec<&'static str>)> {
    use semantic_data::query::{BinaryOp::*, TextMatchMode};

    let text = |query: &str, mode| Expr::TextMatch {
        exprs: vec![field("body")],
        query: Box::new(literal(query.to_string())),
        mode,
        analyzer: Default::default(),
    };
    let nick = |n: u32| literal(format!("n{n}"));
    vec![
        // Equality (possibly unique) index.
        (binary(Eq, field("nick"), nick(1)), vec![]),
        (
            Expr::InList {
                expr: Box::new(field("nick")),
                list: vec![nick(2), nick(5), nick(7)],
                negated: false,
            },
            vec![],
        ),
        // Range index, ordered and bounded.
        (binary(Gt, field("n"), literal(3i64)), vec!["n"]),
        (
            binary(
                And,
                binary(Gte, field("n"), literal(-2i64)),
                binary(Lt, field("n"), literal(4i64)),
            ),
            vec!["n"],
        ),
        // Composite (grp, n) index: equality prefix plus range.
        (
            binary(
                And,
                binary(Eq, field("grp"), literal(1i64)),
                binary(Gte, field("n"), literal(0i64)),
            ),
            vec!["n"],
        ),
        // Partial index on tag where n > 0.
        (
            binary(
                And,
                binary(Eq, field("tag"), literal("red".to_string())),
                binary(Gt, field("n"), literal(0i64)),
            ),
            vec![],
        ),
        // Full-text index.
        (text("fox", TextMatchMode::All), vec![]),
        (text("quick dog", TextMatchMode::All), vec![]),
        (text("lazy brown", TextMatchMode::Any), vec![]),
        // Equality index on references to the other collection.
        (binary(Eq, field("ref"), literal("a3".to_string())), vec![]),
        (binary(Eq, field("ref"), literal("b3".to_string())), vec![]),
        // Not index-shaped.
        (
            binary(
                Or,
                binary(Eq, field("tag"), literal("green".to_string())),
                binary(Lt, field("n"), literal(0i64)),
            ),
            vec![],
        ),
    ]
}

trait LossyU64 {
    fn as_u64_lossy(&self) -> Option<u64>;
}

impl LossyU64 for Value {
    fn as_u64_lossy(&self) -> Option<u64> {
        match self {
            Value::I64(value) => u64::try_from(*value).ok(),
            Value::U64(value) => Some(*value),
            Value::I32(value) => u64::try_from(*value).ok(),
            Value::U32(value) => Some(u64::from(*value)),
            _ => None,
        }
    }
}
