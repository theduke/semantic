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

use semantic_data::value::{FromValue, IntoValue, SemanticType};
use semantic_db_core::embedded::StorageStats;
use semantic_db_core::{
    DEFAULT_REWRITE_BATCH_SIZE, ReindexReport, ReindexTarget, VerifyOptions, VerifyReport,
};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{AttrScopeId, RpcCommand, RpcCommandSpec};

use crate::command::ScopeParams;
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
    (
        $ty:ty,
        $name:literal,
        $payload_ty:ty => $output:ty,
        |$db:ident, $payload:ident, $ctx:ident| $body:expr
    ) => {
        impl RpcCommandSpec for $ty {
            type Payload = $payload_ty;
            type Output = $output;
            type Error = AppError;

            const NAME: &'static str = $name;
        }

        impl RpcCommand<AppRequestContext> for $ty {
            fn call<'a>(
                &'a self,
                ctx: &'a AppRequestContext,
                payload: $payload_ty,
            ) -> Pin<Box<dyn Future<Output = Result<$output, AppError>> + Send + 'a>> {
                Box::pin(async move {
                    let $ctx = ctx;
                    let $db = ctx
                        .resolve_db(payload.scope_id.clone().map(DbScopeId::new))
                        .await?;
                    let $db: &dyn SemanticDb = $db.as_ref();
                    let $payload = payload;
                    $body
                })
            }
        }
    };
}

maintenance_command!(
    Reindex,
    "semantic.db.maintenance.reindex",
    ReindexPayload => ReindexOutput,
    |db, payload, _ctx| {
        let target = payload.target()?;
        Ok(db.reindex(target).await?.into())
    }
);
maintenance_command!(
    Verify,
    "semantic.db.maintenance.verify",
    VerifyPayload => VerifyOutput,
    |db, payload, _ctx| Ok(db.verify(payload.options()).await?.into())
);
maintenance_command!(
    Repair,
    "semantic.db.maintenance.repair",
    VerifyPayload => RepairOutput,
    |db, payload, _ctx| {
        let report = db.repair(payload.options()).await?;
        Ok(RepairOutput {
            before: report.before.into(),
            rebuilt: report.rebuilt.into(),
            after: report.after.into(),
        })
    }
);
maintenance_command!(
    Compact,
    "semantic.db.maintenance.compact",
    ScopeParams => CompactOutput,
    |db, _payload, _ctx| {
        let report = db.compact_storage().await?;
        Ok(CompactOutput {
            before: report.before.into(),
            after: report.after.into(),
            compacted: report.compacted,
            freed_bytes: report.freed_bytes,
            duration_ms: duration_ms(report.duration),
        })
    }
);
maintenance_command!(
    Stats,
    "semantic.db.maintenance.stats",
    ScopeParams => StatsOutput,
    |db, _payload, _ctx| Ok(db.storage_stats().await?.into())
);
maintenance_command!(
    RewritePayloads,
    "semantic.db.maintenance.rewrite_payloads",
    RewritePayloadsPayload => RewritePayloadsOutput,
    |db, payload, _ctx| {
        let batch_size = match payload.batch_size {
            None => DEFAULT_REWRITE_BATCH_SIZE,
            Some(0) => {
                return Err(AppError::InvalidRequest(
                    "field 'batch_size' must be a positive integer".into(),
                ));
            }
            Some(batch_size) => batch_size,
        };
        let report = db.rewrite_payloads(batch_size).await?;
        Ok(RewritePayloadsOutput {
            scanned: report.scanned,
            rewritten: report.rewritten,
            batches: report.batches,
            duration_ms: duration_ms(report.duration),
        })
    }
);
maintenance_command!(
    Backup,
    "semantic.db.maintenance.backup",
    BackupPayload => BackupOutput,
    |db, payload, ctx| {
        if ctx.principal.kind != PrincipalKind::System {
            return Err(AppError::InvalidRequest(
                "database backups write server files and require a system principal".into(),
            ));
        }
        let report = db.backup(PathBuf::from(payload.path)).await?;
        Ok(BackupOutput {
            path: report.path.display().to_string(),
            revision: report.revision,
            entries: report.entries,
            bytes: report.bytes,
            duration_ms: duration_ms(report.duration),
        })
    }
);

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct ReindexPayload {
    #[semantic(attr = AttrScopeId)]
    scope_id: Option<String>,
    /// Rebuild only the indexes of this collection.
    collection: Option<String>,
    /// Rebuild only this index of `collection`.
    index: Option<String>,
}

impl ReindexPayload {
    /// Without `collection` and `index` every index is rebuilt.
    fn target(self) -> Result<ReindexTarget, AppError> {
        match (self.collection, self.index) {
            (None, None) => Ok(ReindexTarget::All),
            (Some(collection), None) => Ok(ReindexTarget::Collection(collection)),
            (Some(collection), Some(name)) => Ok(ReindexTarget::Index { collection, name }),
            (None, Some(_)) => Err(AppError::InvalidRequest(
                "field 'index' requires 'collection'".into(),
            )),
        }
    }
}

/// Every check runs unless disabled by its `check_*` field.
#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct VerifyPayload {
    #[semantic(attr = AttrScopeId)]
    scope_id: Option<String>,
    check_indexes: Option<bool>,
    check_reverse_references: Option<bool>,
    check_relationship_edges: Option<bool>,
    check_stats: Option<bool>,
    check_storage_integrity: Option<bool>,
    check_payloads: Option<bool>,
    max_problems: Option<usize>,
}

impl VerifyPayload {
    fn options(&self) -> VerifyOptions {
        let check = |value: Option<bool>| value.unwrap_or(true);
        let all = VerifyOptions::all();
        VerifyOptions {
            check_indexes: check(self.check_indexes),
            check_reverse_references: check(self.check_reverse_references),
            check_relationship_edges: check(self.check_relationship_edges),
            check_stats: check(self.check_stats),
            check_storage_integrity: check(self.check_storage_integrity),
            check_payloads: check(self.check_payloads),
            max_problems: self.max_problems.unwrap_or(all.max_problems),
            ..all
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct RewritePayloadsPayload {
    #[semantic(attr = AttrScopeId)]
    scope_id: Option<String>,
    /// A positive batch size.
    batch_size: Option<usize>,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct BackupPayload {
    #[semantic(attr = AttrScopeId)]
    scope_id: Option<String>,
    /// The server file path to write the backup to.
    path: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct VerifyOutput {
    ok: bool,
    #[semantic(required)]
    revision: Option<u64>,
    problem_count: u64,
    problems: Vec<VerifyProblemOutput>,
    checked: VerifyCountsOutput,
    skipped: Vec<String>,
    duration_ms: u64,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct VerifyProblemOutput {
    kind: String,
    #[semantic(required)]
    collection: Option<String>,
    #[semantic(required)]
    index: Option<String>,
    #[semantic(required)]
    entity_id: Option<String>,
    detail: String,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance:verify")]
struct VerifyCountsOutput {
    collections: u64,
    rows: u64,
    indexes: u64,
    index_entries: u64,
    reverse_references: u64,
    relationship_edges: u64,
    counters: u64,
}

impl From<VerifyReport> for VerifyOutput {
    fn from(report: VerifyReport) -> Self {
        let checked = report.checked;
        Self {
            ok: report.is_ok(),
            revision: report.revision,
            problem_count: report.problem_count,
            problems: report
                .problems
                .into_iter()
                .map(|problem| VerifyProblemOutput {
                    kind: problem.kind.as_str().into(),
                    collection: problem.collection,
                    index: problem.index,
                    entity_id: problem.entity_id,
                    detail: problem.detail,
                })
                .collect(),
            checked: VerifyCountsOutput {
                collections: checked.collections,
                rows: checked.rows,
                indexes: checked.indexes,
                index_entries: checked.index_entries,
                reverse_references: checked.reverse_references,
                relationship_edges: checked.relationship_edges,
                counters: checked.counters,
            },
            skipped: report.skipped,
            duration_ms: duration_ms(report.duration),
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct ReindexOutput {
    indexes: Vec<ReindexedIndexOutput>,
    derived: Vec<String>,
    duration_ms: u64,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct ReindexedIndexOutput {
    collection: String,
    index: String,
    rows: u64,
    #[semantic(required)]
    entries: Option<u64>,
}

impl From<ReindexReport> for ReindexOutput {
    fn from(report: ReindexReport) -> Self {
        Self {
            indexes: report
                .indexes
                .into_iter()
                .map(|index| ReindexedIndexOutput {
                    collection: index.collection,
                    index: index.index,
                    rows: index.rows,
                    entries: index.entries,
                })
                .collect(),
            derived: report
                .derived
                .into_iter()
                .map(|derived| derived.as_str().into())
                .collect(),
            duration_ms: duration_ms(report.duration),
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct RepairOutput {
    before: VerifyOutput,
    rebuilt: ReindexOutput,
    after: VerifyOutput,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct StatsOutput {
    #[semantic(required)]
    file_size_bytes: Option<u64>,
    #[semantic(required)]
    entries: Option<u64>,
    #[semantic(required)]
    allocated_bytes: Option<u64>,
    #[semantic(required)]
    stored_bytes: Option<u64>,
    #[semantic(required)]
    fragmented_bytes: Option<u64>,
    tables: Vec<TableStatsOutput>,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct TableStatsOutput {
    name: String,
    entries: u64,
}

impl From<StorageStats> for StatsOutput {
    fn from(stats: StorageStats) -> Self {
        Self {
            file_size_bytes: stats.file_size_bytes,
            entries: stats.entries,
            allocated_bytes: stats.allocated_bytes,
            stored_bytes: stats.stored_bytes,
            fragmented_bytes: stats.fragmented_bytes,
            tables: stats
                .tables
                .into_iter()
                .map(|table| TableStatsOutput {
                    name: table.name,
                    entries: table.entries,
                })
                .collect(),
        }
    }
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance:compact")]
struct CompactOutput {
    before: StatsOutput,
    after: StatsOutput,
    compacted: bool,
    #[semantic(required)]
    freed_bytes: Option<u64>,
    duration_ms: u64,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct RewritePayloadsOutput {
    scanned: u64,
    rewritten: u64,
    batches: u64,
    duration_ms: u64,
}

#[derive(SemanticType, IntoValue, FromValue)]
#[semantic(namespace = "semantic:maintenance")]
struct BackupOutput {
    path: String,
    #[semantic(required)]
    revision: Option<u64>,
    entries: u64,
    #[semantic(required)]
    bytes: Option<u64>,
    duration_ms: u64,
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
