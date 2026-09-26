//! Seeded databases and the benchmark workloads run against them.
//!
//! A [`Fixture`] is seeded once per (engine, size) and shared by all
//! workloads. Read workloads leave the data untouched; mutating workloads
//! either rewrite rows in place (the data set keeps its shape) or undo their
//! effect outside the measured section (the `*_timed` methods), so the
//! collection size stays constant across benchmarks.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use semantic_data::query::{
    AggregateOp, Batch, BatchOperation, BinaryOp, DeleteQuery, Expr, FunctionArg, JoinCondition,
    JoinQuery, JoinSource, JoinType, Operand, OrderBy, Query, QueryField, QueryInput, SelectQuery,
    SortDirection, TextAnalyzer, TextMatchMode, UpdateQuery,
};
use semantic_data::schema::DbOpenMode;
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::embedded::EmbeddedBackend;
use semantic_db_core::{
    BatchReturn, Db, DbConfig, DbError, QueryExplain, QueryPlan, TransactionOptions,
};
use semantic_db_redb::{RedbDurability, RedbOptions};

use crate::data::{self, KINDS};
use crate::schema::{self, COLLECTION, attr};

/// Rows per write batch while seeding.
const SEED_BATCH: u64 = 1000;
/// Parameter variants prepared per parameterised query.
const POOL: u64 = 64;
/// Rows per page of the ordered and keyset pagination queries.
pub const PAGE: usize = 50;
/// Reads and writes of the interactive transaction workload.
pub const TX_STATEMENTS: u64 = 10;
/// Rows per batch of the writer of the concurrent workload.
pub const CONCURRENT_WRITE_BATCH: usize = 100;

/// Storage engine behind a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// In-memory KV engine.
    Memory,
    /// redb in a temporary file.
    Redb,
}

impl Engine {
    pub const ALL: [Engine; 2] = [Engine::Memory, Engine::Redb];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Memory => "memory",
            Engine::Redb => "redb",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|engine| engine.name().eq_ignore_ascii_case(name.trim()))
    }
}

/// Queries built once per fixture, as ASTs so that the measured sections
/// exclude SQL parsing. Parameterised queries come in [`POOL`] variants.
/// The SQL equivalent of each query is noted on its field.
struct Queries {
    /// `SELECT * FROM entities WHERE code = $code`
    eq_select: Vec<SelectQuery>,
    /// `SELECT * FROM entities WHERE score >= $low AND score < $low + size / 10 LIMIT 50`
    range_limit: Vec<SelectQuery>,
    /// `SELECT * FROM entities WHERE score >= 0 ORDER BY score LIMIT 50`
    ///
    /// All scores are non-negative; the bound tells the planner that rows
    /// without a score (owners), which a bare `ORDER BY score` must return
    /// first and the index does not hold, are excluded.
    ordered_scan: SelectQuery,
    /// `SELECT * FROM entities WHERE score > $cursor ORDER BY score LIMIT 50`
    keyset_page: Vec<SelectQuery>,
    /// `SELECT id, title FROM entities WHERE text_match(title, body, '$a $b')`
    full_text: Vec<SelectQuery>,
    /// `SELECT * FROM entities WHERE price > 990.0`
    full_scan: SelectQuery,
    /// `SELECT COUNT(*) AS n FROM entities`
    count: SelectQuery,
    /// `SELECT * FROM entities ORDER BY price DESC LIMIT 10`
    top_n: SelectQuery,
    /// `SELECT i.id AS item, o.name AS owner FROM entities AS i
    ///  INNER JOIN entities AS o ON i.owner_ref = o.id WHERE i.kind = $kind`
    join: Vec<SelectQuery>,
    /// `SELECT * FROM entities WHERE kind = $kind ORDER BY created_at DESC LIMIT 20`
    feed: Vec<SelectQuery>,
}

fn field(path: &[&str]) -> Expr {
    Expr::Operand(Operand::Field(FieldPath::from_fields(path.iter().copied())))
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn items() -> SelectQuery {
    SelectQuery::new().with_collection(COLLECTION)
}

fn order_by(path: &[&str], direction: SortDirection) -> Vec<OrderBy> {
    vec![OrderBy {
        expr: field(path),
        direction,
    }]
}

fn project(fields: &[(&[&str], &str)]) -> Vec<QueryField> {
    fields
        .iter()
        .map(|(path, alias)| QueryField {
            expr: Box::new(field(path)),
            alias: Some(alias.to_string()),
        })
        .collect()
}

fn pool(build: impl Fn(u64) -> SelectQuery) -> Vec<SelectQuery> {
    (0..POOL).map(build).collect()
}

/// Spreads iteration or variant `n` over `0..bound`.
fn spread(n: u64, bound: u64) -> u64 {
    n.wrapping_mul(0x9E37_79B9) % bound.max(1)
}

impl Queries {
    fn new(size: u64) -> Self {
        let score = || field(&[attr::SCORE]);
        let int = |value: u64| literal(Value::I64(value as i64));
        let range_width = (size / 10).max(1);
        Self {
            eq_select: pool(|v| {
                items().with_predicate(eq(attr::CODE, Value::String(data::code(spread(v, size)))))
            }),
            range_limit: pool(|v| {
                let low = spread(v, size.saturating_sub(range_width).max(1));
                items()
                    .with_predicate(binary(
                        BinaryOp::And,
                        binary(BinaryOp::Gte, score(), int(low)),
                        binary(BinaryOp::Lt, score(), int(low + range_width)),
                    ))
                    .with_limit(PAGE)
            }),
            ordered_scan: items()
                .with_predicate(binary(BinaryOp::Gte, score(), int(0)))
                .with_order_by(order_by(&[attr::SCORE], SortDirection::Asc))
                .with_limit(PAGE),
            keyset_page: pool(|v| {
                let cursor = spread(v, size.saturating_sub(PAGE as u64).max(1));
                items()
                    .with_predicate(binary(BinaryOp::Gt, score(), int(cursor)))
                    .with_order_by(order_by(&[attr::SCORE], SortDirection::Asc))
                    .with_limit(PAGE)
            }),
            full_text: pool(|v| {
                let first = 8 + (v % 16) * 2;
                let terms = format!("{} {}", data::word(first), data::word(first + 1 + v / 16));
                items()
                    .with_predicate(Expr::TextMatch {
                        exprs: vec![field(&[attr::TITLE]), field(&[attr::BODY])],
                        query: Box::new(literal(Value::String(terms))),
                        mode: TextMatchMode::All,
                        analyzer: TextAnalyzer::default(),
                    })
                    .with_projection(project(&[(&["id"], "id"), (&[attr::TITLE], attr::TITLE)]))
            }),
            full_scan: items().with_predicate(binary(
                BinaryOp::Gt,
                field(&[attr::PRICE]),
                literal(Value::F64(990.0.into())),
            )),
            count: items().with_projection(vec![QueryField {
                expr: Box::new(Expr::Aggregate {
                    op: AggregateOp::Count,
                    distinct: false,
                    arg: Box::new(FunctionArg::Wildcard),
                }),
                alias: Some("n".to_string()),
            }]),
            top_n: items()
                .with_order_by(order_by(&[attr::PRICE], SortDirection::Desc))
                .with_limit(10usize),
            join: pool(|v| {
                items()
                    .with_source_alias("i")
                    .with_joins(vec![JoinQuery {
                        source: JoinSource {
                            collection: Some(COLLECTION.to_string()),
                            class: None,
                        },
                        alias: Some("o".to_string()),
                        join_type: JoinType::Inner,
                        condition: JoinCondition::OnExpr(binary(
                            BinaryOp::Eq,
                            field(&["i", attr::OWNER]),
                            field(&["o", "id"]),
                        )),
                        predicate: None,
                    }])
                    .with_predicate(binary(
                        BinaryOp::Eq,
                        field(&["i", attr::KIND]),
                        literal(Value::String(data::kind(v))),
                    ))
                    .with_projection(project(&[
                        (&["i", "id"], "item"),
                        (&["o", attr::NAME], "owner"),
                    ]))
            }),
            feed: pool(|v| {
                items()
                    .with_predicate(eq(attr::KIND, Value::String(data::kind(v))))
                    .with_order_by(order_by(&[attr::CREATED_AT], SortDirection::Desc))
                    .with_limit(20usize)
            }),
        }
    }
}

fn eq(field: &str, value: Value) -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            field,
        ])))),
        right: Box::new(Expr::Operand(Operand::Literal(value))),
    }
}

fn literal(value: Value) -> Expr {
    Expr::Operand(Operand::Literal(value))
}

fn pick<T>(items: &[T], n: u64) -> &T {
    &items[(n % items.len() as u64) as usize]
}

fn upserts(rows: impl IntoIterator<Item = Object>) -> Batch {
    rows.into_iter().fold(Batch::new(), |batch, row| {
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .expect("generated rows have an id")
            .to_string();
        batch.with_op(BatchOperation::Upsert {
            collection: COLLECTION.to_string(),
            id,
            object: row,
        })
    })
}

/// Commit `batch` replying with statistics only: the bounded write path,
/// whose cost does not depend on the collection size.
async fn write_stats(db: &Db, batch: Batch) -> Result<(), DbError> {
    db.execute_batch_returning(batch, BatchReturn::Stats)
        .await
        .map(drop)
}

/// How a write workload commits its batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePath {
    /// `Db::execute_batch` / `Db::delete`, which reply with a `Dataset`.
    Dataset,
    /// `Db::execute_batch_returning(batch, BatchReturn::Stats)`.
    Stats,
}

impl WritePath {
    pub fn name(self) -> &'static str {
        match self {
            WritePath::Dataset => "execute_batch",
            WritePath::Stats => "returning_stats",
        }
    }
}

/// A seeded database of one engine and size.
pub struct Fixture {
    engine: Engine,
    size: u64,
    db: Arc<Db>,
    queries: Arc<Queries>,
    /// Temporary directory of the redb file; removed on drop.
    dir: Option<tempfile::TempDir>,
    /// Source of ids for rows inserted by the benchmarks.
    next_insert: AtomicU64,
}

impl Fixture {
    /// Open an empty database, create the schema and seed `size` items and
    /// their owners. redb commits with [`RedbDurability::Eventual`], so write
    /// benchmarks measure engine cost rather than fsync latency.
    pub async fn seeded(engine: Engine, size: u64) -> Result<Self, anyhow::Error> {
        let (db, dir) = match engine {
            Engine::Memory => {
                let db = semantic_db_kv::open_memory()?;
                (Db::new(EmbeddedBackend::new(db)), None)
            }
            Engine::Redb => {
                let dir = tempfile::Builder::new()
                    .prefix("semantic-db-bench-")
                    .tempdir()?;
                let db = open_redb(&redb_path(&dir), RedbDurability::Eventual)?;
                (db, Some(dir))
            }
        };
        db.execute_ddl(schema::ddl()).await?;
        let owners = data::owner_count(size);
        for start in (0..owners).step_by(SEED_BATCH as usize) {
            let end = (start + SEED_BATCH).min(owners);
            write_stats(&db, upserts((start..end).map(data::owner))).await?;
        }
        for start in (0..size).step_by(SEED_BATCH as usize) {
            let end = (start + SEED_BATCH).min(size);
            write_stats(&db, upserts((start..end).map(|i| data::item(i, size)))).await?;
        }
        Ok(Self {
            engine,
            size,
            db: Arc::new(db),
            queries: Arc::new(Queries::new(size)),
            dir,
            next_insert: AtomicU64::new(0),
        })
    }

    /// Reopen a redb fixture with another commit durability, keeping its data.
    pub async fn reopen_redb(self, durability: RedbDurability) -> Result<Self, anyhow::Error> {
        let Self {
            engine,
            size,
            db,
            queries,
            dir,
            next_insert,
        } = self;
        let dir = dir.ok_or_else(|| anyhow::anyhow!("only redb fixtures can be reopened"))?;
        drop(db);
        let path = redb_path(&dir);
        // Background work of the old handle may briefly keep the file open.
        let mut attempts = 0;
        let db = loop {
            match open_redb(&path, durability) {
                Ok(db) => break db,
                Err(err) if attempts < 50 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    if attempts == 50 {
                        return Err(err.into());
                    }
                }
                Err(err) => return Err(err.into()),
            }
        };
        Ok(Self {
            engine,
            size,
            db: Arc::new(db),
            queries,
            dir: Some(dir),
            next_insert,
        })
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    async fn select(&self, query: &SelectQuery) -> Result<Vec<Object>, DbError> {
        self.db.select(query.clone()).await
    }

    /// Plan of the query a read workload runs for iteration `n`.
    pub async fn plan(&self, workload: ReadWorkload, n: u64) -> Result<QueryPlan, DbError> {
        let query = self
            .read_query(workload, n)
            .ok_or_else(|| DbError::InvalidQuery(format!("{workload:?} does not run a query")))?;
        self.db
            .plan(QueryInput::Ast(Query::Select(query.clone())))
            .await
    }

    /// Explanation of the query a read workload runs for iteration `n`.
    pub async fn explain(&self, workload: ReadWorkload, n: u64) -> Result<QueryExplain, DbError> {
        let query = self
            .read_query(workload, n)
            .ok_or_else(|| DbError::InvalidQuery(format!("{workload:?} does not run a query")))?;
        self.db
            .explain(QueryInput::Ast(Query::Select(query.clone())))
            .await
    }

    fn read_query(&self, workload: ReadWorkload, n: u64) -> Option<&SelectQuery> {
        let q = &self.queries;
        Some(match workload {
            ReadWorkload::PointGet => return None,
            ReadWorkload::EqSelect => pick(&q.eq_select, n),
            ReadWorkload::RangeLimit => pick(&q.range_limit, n),
            ReadWorkload::OrderedScan => &q.ordered_scan,
            ReadWorkload::KeysetPage => pick(&q.keyset_page, n),
            ReadWorkload::FullText => pick(&q.full_text, n),
            ReadWorkload::FullScan => &q.full_scan,
            ReadWorkload::Count => &q.count,
            ReadWorkload::TopN => &q.top_n,
            ReadWorkload::Join => pick(&q.join, n),
            ReadWorkload::Feed => pick(&q.feed, n),
        })
    }

    /// Run read workload `workload` for iteration `n`; returns the number of
    /// rows produced (for `Count`, the counted value).
    pub async fn read(&self, workload: ReadWorkload, n: u64) -> Result<usize, DbError> {
        match workload {
            ReadWorkload::PointGet => {
                let id = data::item_id(spread(n, self.size));
                let row = self.db.get(COLLECTION, id.clone()).await?;
                row.map(|_| 1)
                    .ok_or_else(|| DbError::InvalidQuery(format!("item {id} is missing")))
            }
            ReadWorkload::Count => {
                let rows = self.select(&self.queries.count).await?;
                let count = rows
                    .first()
                    .and_then(|row| row.get("n"))
                    .and_then(|n| match n {
                        Value::I64(n) => usize::try_from(*n).ok(),
                        Value::U64(n) => usize::try_from(*n).ok(),
                        _ => None,
                    });
                count.ok_or_else(|| DbError::InvalidQuery(format!("bad count result {rows:?}")))
            }
            other => {
                let query = self.read_query(other, n).expect("query workload");
                Ok(self.select(query).await?.len())
            }
        }
    }

    /// Insert `len` new items in one batch committed through `path`;
    /// returns the time of the commit. The rows are deleted again outside
    /// the measured section.
    pub async fn insert_batch_timed(
        &self,
        len: usize,
        path: WritePath,
    ) -> Result<Duration, DbError> {
        let (batch, ids) = self.new_items(len, "new");
        let started = Instant::now();
        match path {
            WritePath::Dataset => self.db.execute_batch(batch).await.map(drop)?,
            WritePath::Stats => write_stats(&self.db, batch).await?,
        }
        let elapsed = started.elapsed();
        self.remove_items(ids).await?;
        Ok(elapsed)
    }

    fn new_items(&self, len: usize, prefix: &str) -> (Batch, Vec<String>) {
        let first = self.next_insert.fetch_add(len as u64, Ordering::Relaxed);
        let mut ids = Vec::with_capacity(len);
        let batch = (first..first + len as u64).fold(Batch::new(), |batch, seq| {
            let id = format!("{prefix}-{seq:010}");
            let mut row = data::item(seq % self.size, self.size);
            row.insert("id", Value::String(id.clone()));
            ids.push(id.clone());
            batch.with_op(BatchOperation::Create {
                collection: COLLECTION.to_string(),
                id,
                object: row,
            })
        });
        (batch, ids)
    }

    async fn remove_items(&self, ids: Vec<String>) -> Result<(), DbError> {
        for chunk in ids.chunks(SEED_BATCH as usize) {
            let batch = Batch::new().with_op(BatchOperation::DeleteByIds {
                collection: COLLECTION.to_string(),
                ids: chunk.to_vec(),
            });
            write_stats(&self.db, batch).await?;
        }
        Ok(())
    }

    /// Restore seeded items `ordinals` to their generated state.
    async fn restore_items(&self, ordinals: impl Iterator<Item = u64>) -> Result<(), DbError> {
        let rows = ordinals
            .map(|i| data::item(i, self.size))
            .collect::<Vec<_>>();
        for chunk in rows.chunks(SEED_BATCH as usize) {
            write_stats(&self.db, upserts(chunk.iter().cloned())).await?;
        }
        Ok(())
    }

    /// Rating written by iteration `n` of the workload using `base`: unique
    /// per iteration and workload, so that every write really changes the
    /// row (unchanged writes are skipped by the database).
    fn rating(base: i64, n: u64) -> Value {
        Value::I64(base + n as i64)
    }

    /// `UPDATE items SET rating = .. WHERE kind = ..` (1 % of the rows,
    /// served by the composite index); returns the rows updated.
    pub async fn update_by_kind(&self, n: u64) -> Result<usize, DbError> {
        let query = UpdateQuery::new()
            .with_collection(COLLECTION)
            .with_predicate(eq(attr::KIND, Value::String(data::kind(n))))
            .set(
                FieldPath::from_fields([attr::RATING]),
                literal(Self::rating(1_000_000_000, n)),
            );
        Ok(self.db.update_where(query).await?.affected)
    }

    /// `UPDATE items SET rating = .. WHERE id = ..`; returns the rows updated.
    pub async fn update_by_id(&self, n: u64) -> Result<usize, DbError> {
        let query = UpdateQuery::new()
            .with_collection(COLLECTION)
            .with_predicate(eq("id", Value::String(data::item_id(n % self.size))))
            .set(
                FieldPath::from_fields([attr::RATING]),
                literal(Self::rating(2_000_000_000, n)),
            );
        Ok(self.db.update_where(query).await?.affected)
    }

    /// Delete one item by id through `path` (`Db::delete` or a `DeleteById`
    /// batch); returns the time of the delete. The item is restored outside
    /// the measured section.
    pub async fn delete_by_id_timed(&self, n: u64, path: WritePath) -> Result<Duration, DbError> {
        let ordinal = spread(n, self.size);
        let id = data::item_id(ordinal);
        let started = Instant::now();
        match path {
            WritePath::Dataset => self.db.delete(COLLECTION, id).await?,
            WritePath::Stats => {
                let batch = Batch::new().with_op(BatchOperation::DeleteById {
                    collection: COLLECTION.to_string(),
                    id,
                });
                write_stats(&self.db, batch).await?
            }
        }
        let elapsed = started.elapsed();
        self.restore_items(std::iter::once(ordinal)).await?;
        Ok(elapsed)
    }

    /// `DELETE FROM entities WHERE kind = ..` (1 % of the rows); returns the
    /// time of the delete. The rows are restored outside the measured
    /// section.
    pub async fn delete_by_kind_timed(&self, n: u64) -> Result<Duration, DbError> {
        let kind = n % KINDS;
        let query = DeleteQuery::new()
            .with_collection(COLLECTION)
            .with_predicate(eq(attr::KIND, Value::String(data::kind(kind))));
        let started = Instant::now();
        let deleted = self.db.delete_where(query).await?;
        let elapsed = started.elapsed();
        let expected = (kind..self.size).step_by(KINDS as usize);
        if deleted != expected.clone().count() {
            return Err(DbError::InvalidQuery(format!(
                "deleted {deleted} rows of {}, expected {}",
                data::kind(kind),
                expected.count()
            )));
        }
        self.restore_items(expected).await?;
        Ok(elapsed)
    }

    /// An interactive transaction reading [`TX_STATEMENTS`] items by id and
    /// rewriting [`TX_STATEMENTS`] others, then committing.
    pub async fn transaction(&self, n: u64) -> Result<(), DbError> {
        let tx = self
            .db
            .begin_transaction(TransactionOptions::default())
            .await?;
        let base = n.wrapping_mul(2 * TX_STATEMENTS);
        for j in 0..TX_STATEMENTS {
            let id = data::item_id(spread(base + j, self.size));
            tx.get(COLLECTION.to_string(), id).await?;
        }
        for j in TX_STATEMENTS..2 * TX_STATEMENTS {
            let ordinal = spread(base + j, self.size);
            let mut row = data::item(ordinal, self.size);
            row.insert(attr::RATING, Self::rating(3_000_000_000, n));
            tx.upsert(COLLECTION.to_string(), data::item_id(ordinal), row)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Run `iters` feed queries split over `readers` concurrent tasks while
    /// one writer task keeps inserting batches of [`CONCURRENT_WRITE_BATCH`]
    /// rows (on the bounded [`WritePath::Stats`] path, so the writer's
    /// critical sections do not grow with the collection); returns the summed reader
    /// latency (so the per-iteration time is the mean query latency). The
    /// writer's rows are deleted afterwards, outside the measured section.
    pub async fn concurrent_reads(
        self: &Arc<Self>,
        iters: u64,
        readers: u64,
    ) -> Result<Duration, anyhow::Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let fixture = Arc::clone(self);
            let stop = Arc::clone(&stop);
            tokio::spawn(async move {
                let mut written = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let (batch, ids) = fixture.new_items(CONCURRENT_WRITE_BATCH, "conc");
                    write_stats(&fixture.db, batch).await?;
                    written.extend(ids);
                    tokio::task::yield_now().await;
                }
                Ok::<_, DbError>(written)
            })
        };
        let tasks = (0..readers)
            .map(|reader| {
                let fixture = Arc::clone(self);
                let count = iters / readers + u64::from(reader < iters % readers);
                tokio::spawn(async move {
                    let mut total = Duration::ZERO;
                    for i in 0..count {
                        let query = pick(&fixture.queries.feed, reader * 7919 + i);
                        let started = Instant::now();
                        fixture.select(query).await?;
                        total += started.elapsed();
                    }
                    Ok::<_, DbError>(total)
                })
            })
            .collect::<Vec<_>>();
        let mut total = Duration::ZERO;
        let mut failure = None;
        for task in tasks {
            match task.await? {
                Ok(elapsed) => total += elapsed,
                Err(err) => failure = Some(err),
            }
        }
        stop.store(true, Ordering::Relaxed);
        let written = writer.await??;
        self.remove_items(written).await?;
        match failure {
            Some(err) => Err(err.into()),
            None => Ok(total),
        }
    }
}

/// Read-only workloads; see [`Fixture::read`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWorkload {
    /// `get` by id.
    PointGet,
    /// Equality index lookup matching a few rows.
    EqSelect,
    /// Unordered range index scan with `LIMIT`.
    RangeLimit,
    /// `ORDER BY indexed LIMIT 50`.
    OrderedScan,
    /// One keyset pagination page: `WHERE indexed > cursor ORDER BY indexed LIMIT 50`.
    KeysetPage,
    /// Full-text match of two terms (all must match).
    FullText,
    /// Full scan with a non-indexed predicate.
    FullScan,
    /// `COUNT(*)`.
    Count,
    /// `ORDER BY non_indexed LIMIT 10` (top-N over a full scan).
    TopN,
    /// Items joined to their owners through the `Ref` attribute.
    Join,
    /// Latest items of one kind (composite index); the concurrent reader query.
    Feed,
}

impl ReadWorkload {
    pub const ALL: [ReadWorkload; 11] = [
        ReadWorkload::PointGet,
        ReadWorkload::EqSelect,
        ReadWorkload::RangeLimit,
        ReadWorkload::OrderedScan,
        ReadWorkload::KeysetPage,
        ReadWorkload::FullText,
        ReadWorkload::FullScan,
        ReadWorkload::Count,
        ReadWorkload::TopN,
        ReadWorkload::Join,
        ReadWorkload::Feed,
    ];

    /// Benchmark group name.
    pub fn name(self) -> &'static str {
        match self {
            ReadWorkload::PointGet => "point_get",
            ReadWorkload::EqSelect => "eq_select",
            ReadWorkload::RangeLimit => "range_limit",
            ReadWorkload::OrderedScan => "ordered_index_scan",
            ReadWorkload::KeysetPage => "keyset_page",
            ReadWorkload::FullText => "full_text",
            ReadWorkload::FullScan => "full_scan_filter",
            ReadWorkload::Count => "count",
            ReadWorkload::TopN => "top_n_unindexed",
            ReadWorkload::Join => "join_ref",
            ReadWorkload::Feed => "feed",
        }
    }

    /// Whether one iteration touches every row (benchmarks take fewer
    /// samples).
    pub fn scans_all_rows(self) -> bool {
        matches!(
            self,
            ReadWorkload::FullScan | ReadWorkload::TopN | ReadWorkload::Count
        )
    }
}

fn redb_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("bench.redb")
}

fn open_redb(path: &std::path::Path, durability: RedbDurability) -> Result<Db, DbError> {
    let backend = semantic_db_redb::open_backend_with_options(
        path,
        DbOpenMode::AutoCreate,
        RedbOptions::default().with_durability(durability),
        DbConfig::default(),
    )?;
    Ok(Db::new(backend))
}
