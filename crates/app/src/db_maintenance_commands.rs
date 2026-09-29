//! `semantic.db.maintenance.*` commands: database maintenance operations of
//! [`SemanticDb`](crate::SemanticDb) for RPC clients.
//!
//! Every command accepts an optional `scope_id` (like the
//! `semantic.db.validation.*` commands) and returns its report as an
//! object. `backup` writes a file on the server and is limited to system
//! principals.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use semantic_data::schema::FunctionType;
use semantic_data::value::{Object, Value};
use semantic_db_core::embedded::StorageStats;
use semantic_db_core::{
    DEFAULT_REWRITE_BATCH_SIZE, ReindexReport, ReindexTarget, VerifyOptions, VerifyReport,
};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{RpcCommand, RpcCommandSpec};

use crate::command::{expect_object, optional_bool, optional_string, required_string};
use crate::{AppError, AppRequestContext, DbScopeId, PrincipalKind, SemanticDb};

pub(crate) fn register(
    registry: &mut RpcRegistry<AppRequestContext, AppError>,
) -> Result<(), AppError> {
    registry.register(Reindex)?;
    registry.register(Verify)?;
    registry.register(Repair)?;
    registry.register(Compact)?;
    registry.register(Stats)?;
    registry.register(RewritePayloads)?;
    registry.register(Backup)?;
    Ok(())
}

struct Reindex;
struct Verify;
struct Repair;
struct Compact;
struct Stats;
struct RewritePayloads;
struct Backup;

macro_rules! maintenance_command {
    ($ty:ty, $name:literal, |$db:ident, $payload:ident, $ctx:ident| $body:expr) => {
        impl RpcCommandSpec for $ty {
            type Payload = Value;
            type Output = Value;
            type Error = AppError;

            const NAME: &'static str = $name;

            fn signature(&self) -> FunctionType {
                FunctionType {
                    params: Vec::new(),
                    results: Vec::new(),
                    throws: None,
                    async_fn: true,
                }
            }
        }

        impl RpcCommand<AppRequestContext> for $ty {
            fn call<'a>(
                &'a self,
                ctx: &'a AppRequestContext,
                payload: Value,
            ) -> Pin<Box<dyn Future<Output = Result<Value, AppError>> + Send + 'a>> {
                Box::pin(async move {
                    let $payload = expect_object(payload)?;
                    let $ctx = ctx;
                    let $db = ctx
                        .resolve_db(optional_string(&$payload, "scope_id")?.map(DbScopeId::new))
                        .await?;
                    let $db: &dyn SemanticDb = $db.as_ref();
                    $body
                })
            }
        }
    };
}

maintenance_command!(
    Reindex,
    "semantic.db.maintenance.reindex",
    |db, payload, _ctx| {
        let target = reindex_target(&payload)?;
        Ok(Value::Object(reindex_object(db.reindex(target).await?)))
    }
);
maintenance_command!(
    Verify,
    "semantic.db.maintenance.verify",
    |db, payload, _ctx| {
        let options = verify_options(&payload)?;
        Ok(Value::Object(verify_object(db.verify(options).await?)))
    }
);
maintenance_command!(
    Repair,
    "semantic.db.maintenance.repair",
    |db, payload, _ctx| {
        let report = db.repair(verify_options(&payload)?).await?;
        let mut out = Object::new();
        out.insert("before", Value::Object(verify_object(report.before)));
        out.insert("rebuilt", Value::Object(reindex_object(report.rebuilt)));
        out.insert("after", Value::Object(verify_object(report.after)));
        Ok(Value::Object(out))
    }
);
maintenance_command!(
    Compact,
    "semantic.db.maintenance.compact",
    |db, _payload, _ctx| {
        let report = db.compact_storage().await?;
        let mut out = Object::new();
        out.insert("before", Value::Object(stats_object(report.before)));
        out.insert("after", Value::Object(stats_object(report.after)));
        out.insert("compacted", Value::Bool(report.compacted));
        out.insert("freed_bytes", optional_u64(report.freed_bytes));
        out.insert("duration_ms", duration_ms(report.duration));
        Ok(Value::Object(out))
    }
);
maintenance_command!(
    Stats,
    "semantic.db.maintenance.stats",
    |db, _payload, _ctx| Ok(Value::Object(stats_object(db.storage_stats().await?)))
);
maintenance_command!(
    RewritePayloads,
    "semantic.db.maintenance.rewrite_payloads",
    |db, payload, _ctx| {
        let batch_size = match payload.get("batch_size") {
            None | Some(Value::Null) | Some(Value::Void) => DEFAULT_REWRITE_BATCH_SIZE,
            Some(value) => value
                .as_i64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    AppError::InvalidRequest("field 'batch_size' must be a positive integer".into())
                })?,
        };
        let report = db.rewrite_payloads(batch_size).await?;
        let mut out = Object::new();
        out.insert("scanned", Value::U64(report.scanned));
        out.insert("rewritten", Value::U64(report.rewritten));
        out.insert("batches", Value::U64(report.batches));
        out.insert("duration_ms", duration_ms(report.duration));
        Ok(Value::Object(out))
    }
);
maintenance_command!(
    Backup,
    "semantic.db.maintenance.backup",
    |db, payload, ctx| {
        if ctx.principal.kind != PrincipalKind::System {
            return Err(AppError::InvalidRequest(
                "database backups write server files and require a system principal".into(),
            ));
        }
        let path = PathBuf::from(required_string(&payload, "path")?);
        let report = db.backup(path).await?;
        let mut out = Object::new();
        out.insert("path", Value::String(report.path.display().to_string()));
        out.insert("revision", optional_u64(report.revision));
        out.insert("entries", Value::U64(report.entries));
        out.insert("bytes", optional_u64(report.bytes));
        out.insert("duration_ms", duration_ms(report.duration));
        Ok(Value::Object(out))
    }
);

/// `collection` and `index` select the reindexed indexes; without either
/// every index is rebuilt.
fn reindex_target(payload: &Object) -> Result<ReindexTarget, AppError> {
    match (
        optional_string(payload, "collection")?,
        optional_string(payload, "index")?,
    ) {
        (None, None) => Ok(ReindexTarget::All),
        (Some(collection), None) => Ok(ReindexTarget::Collection(collection)),
        (Some(collection), Some(name)) => Ok(ReindexTarget::Index { collection, name }),
        (None, Some(_)) => Err(AppError::InvalidRequest(
            "field 'index' requires 'collection'".into(),
        )),
    }
}

/// Every check runs unless disabled by its `check_*` field.
fn verify_options(payload: &Object) -> Result<VerifyOptions, AppError> {
    let check = |field| Ok::<_, AppError>(optional_bool(payload, field)?.unwrap_or(true));
    let mut options = VerifyOptions {
        check_indexes: check("check_indexes")?,
        check_reverse_references: check("check_reverse_references")?,
        check_relationship_edges: check("check_relationship_edges")?,
        check_stats: check("check_stats")?,
        check_storage_integrity: check("check_storage_integrity")?,
        check_payloads: check("check_payloads")?,
        ..VerifyOptions::all()
    };
    if let Some(value) = payload.get("max_problems") {
        options.max_problems = value
            .as_i64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                AppError::InvalidRequest("field 'max_problems' must be a count".into())
            })?;
    }
    Ok(options)
}

fn verify_object(report: VerifyReport) -> Object {
    let mut out = Object::new();
    out.insert("ok", Value::Bool(report.is_ok()));
    out.insert("revision", optional_u64(report.revision));
    out.insert("problem_count", Value::U64(report.problem_count));
    out.insert(
        "problems",
        Value::List(
            report
                .problems
                .into_iter()
                .map(|problem| {
                    let mut row = Object::new();
                    row.insert("kind", Value::String(problem.kind.as_str().into()));
                    row.insert("collection", optional_string_value(problem.collection));
                    row.insert("index", optional_string_value(problem.index));
                    row.insert("entity_id", optional_string_value(problem.entity_id));
                    row.insert("detail", Value::String(problem.detail));
                    Value::Object(row)
                })
                .collect(),
        ),
    );
    let checked = report.checked;
    let mut counts = Object::new();
    for (name, count) in [
        ("collections", checked.collections),
        ("rows", checked.rows),
        ("indexes", checked.indexes),
        ("index_entries", checked.index_entries),
        ("reverse_references", checked.reverse_references),
        ("relationship_edges", checked.relationship_edges),
        ("counters", checked.counters),
    ] {
        counts.insert(name, Value::U64(count));
    }
    out.insert("checked", Value::Object(counts));
    out.insert(
        "skipped",
        Value::List(report.skipped.into_iter().map(Value::String).collect()),
    );
    out.insert("duration_ms", duration_ms(report.duration));
    out
}

fn reindex_object(report: ReindexReport) -> Object {
    let mut out = Object::new();
    out.insert(
        "indexes",
        Value::List(
            report
                .indexes
                .into_iter()
                .map(|index| {
                    let mut row = Object::new();
                    row.insert("collection", Value::String(index.collection));
                    row.insert("index", Value::String(index.index));
                    row.insert("rows", Value::U64(index.rows));
                    row.insert("entries", optional_u64(index.entries));
                    Value::Object(row)
                })
                .collect(),
        ),
    );
    out.insert(
        "derived",
        Value::List(
            report
                .derived
                .into_iter()
                .map(|derived| Value::String(derived.as_str().into()))
                .collect(),
        ),
    );
    out.insert("duration_ms", duration_ms(report.duration));
    out
}

fn stats_object(stats: StorageStats) -> Object {
    let mut out = Object::new();
    out.insert("file_size_bytes", optional_u64(stats.file_size_bytes));
    out.insert("entries", optional_u64(stats.entries));
    out.insert("allocated_bytes", optional_u64(stats.allocated_bytes));
    out.insert("stored_bytes", optional_u64(stats.stored_bytes));
    out.insert("fragmented_bytes", optional_u64(stats.fragmented_bytes));
    out.insert(
        "tables",
        Value::List(
            stats
                .tables
                .into_iter()
                .map(|table| {
                    let mut row = Object::new();
                    row.insert("name", Value::String(table.name));
                    row.insert("entries", Value::U64(table.entries));
                    Value::Object(row)
                })
                .collect(),
        ),
    );
    out
}

fn optional_u64(value: Option<u64>) -> Value {
    value.map_or(Value::Null, Value::U64)
}

fn optional_string_value(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::String)
}

fn duration_ms(duration: Duration) -> Value {
    Value::U64(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}
