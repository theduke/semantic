use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_trait::async_trait;
use futures_util::StreamExt;
use semantic_data::{
    Object, Value,
    query::{self as public, BinaryOp, Expr, FieldFormat, Query, QueryInput, SelectQuery},
};
use semantic_db_core::catalog::Catalog;
use semantic_db_core::{
    Batch, BatchOperation, CoreError, DEFAULT_COLLECTION, DEFAULT_EXECUTION_BATCH_SIZE, DbError,
    DynObject, EntityRecord, FederatedEngine, FederatedError, FederatedExplain, FederationSources,
    QueryResult, QuerySource, SendableRecordBatchStream, SourceScan, TextQueryInput,
    normalize_virtual_joins, referenced_collections, unresolved_join_collections,
};
use semantic_vdb::{AcceptedScan, FilterSupport, ScanPlan, ScanRequest, ScopeVdbs, VdbSet};

use crate::{DbScopeId, Principal, SemanticApp, SemanticDb};

/// Local leaves execute through the asynchronous database API, retaining the
/// local optimizer and indexes without entering the embedded LocalPool path.
pub(crate) struct LocalSource {
    inner: Arc<dyn SemanticDb>,
}

#[async_trait]
impl QuerySource for LocalSource {
    async fn negotiate(
        &self,
        _collection: &str,
        request: &ScanRequest,
    ) -> Result<ScanPlan, DbError> {
        Ok(ScanPlan::Accepted {
            plan: AcceptedScan {
                filters: vec![FilterSupport::Exact; request.filters.len()],
                ordered_prefix: request.order_by.len() as u64,
                limit_applied: request.limit.is_some(),
                offset_applied: request.offset > 0,
                estimated_rows: None,
                token: None,
                schema_revision: "local".into(),
            },
        })
    }

    fn scan(self: Arc<Self>, scan: SourceScan) -> SendableRecordBatchStream {
        let query = SelectQuery {
            collection: Some(scan.collection),
            predicate: scan
                .request
                .filters
                .into_iter()
                .reduce(|left, right| Expr::Binary {
                    op: BinaryOp::And,
                    left: Box::new(left),
                    right: Box::new(right),
                }),
            order_by: scan.request.order_by,
            limit: scan
                .request
                .limit
                .map(|value| Expr::Operand(public::Operand::Literal(Value::U64(value)))),
            offset: Expr::Operand(public::Operand::Literal(Value::U64(scan.request.offset))),
            // Projection is a hint. Whole entities retain id/type and computed
            // dependencies, as required by the shared row model.
            field_format: FieldFormat::Qualified,
            ..Default::default()
        };
        let stream = futures_util::stream::once(async move {
            match self
                .inner
                .query_data(QueryInput::ast_with_params(query, scan.bindings))
                .await
            {
                Ok(QueryResult::Select(rows)) => {
                    let mut rows = rows.into_iter();
                    let mut batches = Vec::new();
                    loop {
                        let batch: Vec<DynObject> = rows
                            .by_ref()
                            .take(DEFAULT_EXECUTION_BATCH_SIZE)
                            .map(|row| Box::new(row) as DynObject)
                            .collect();
                        if batch.is_empty() {
                            break;
                        }
                        batches.push(Ok(batch));
                    }
                    batches
                }
                Ok(_) => vec![Err(CoreError::new(
                    "local federation scan must return SELECT rows",
                ))],
                Err(error) => vec![Err(CoreError::new(error.to_string()))],
            }
        });
        Box::pin(stream.flat_map(futures_util::stream::iter))
    }
}

#[derive(Clone)]
pub(crate) struct VdbAccess {
    app: SemanticApp,
    principal: Principal,
    scope: DbScopeId,
}

impl VdbAccess {
    fn may_have_virtual_databases(&self, local: &Catalog) -> bool {
        // Persisted host-provider activations require this schema. Native
        // registrations may also be used with a custom database catalog.
        local
            .collection_by_name(semantic_data::plugin::COLLECTION)
            .is_some()
            || local.class_id(semantic_data::plugin::CLASS_ID).is_some()
            || self.app.has_native_virtual_databases()
    }

    pub(crate) fn new(app: SemanticApp, principal: Principal, scope: DbScopeId) -> Self {
        Self {
            app,
            principal,
            scope,
        }
    }

    pub(crate) async fn snapshot(
        &self,
        local: Arc<Catalog>,
    ) -> Result<(Arc<ScopeVdbs>, VdbSet), DbError> {
        let plugins = self
            .app
            .plugins(&self.principal, self.scope.clone())
            .await
            .map_err(app_db_error)?;
        let set = plugins
            .vdbs
            .snapshot(plugins.runtime.bindings().await, local)
            .await;
        Ok((plugins.vdbs.clone(), set))
    }

    pub(crate) async fn unavailable(
        &self,
        names: &BTreeSet<String>,
        set: &VdbSet,
    ) -> Result<Option<DbError>, DbError> {
        for name in names {
            if set.get_available(name).is_some() {
                continue;
            }
            if let Some(entry) = set.entry(name) {
                if let semantic_vdb::VdbStatus::Unavailable { reason } = &entry.status {
                    return Ok(Some(DbError::InvalidQuery(format!(
                        "virtual database '{name}' is unavailable: {reason}"
                    ))));
                }
            }
        }
        if names.iter().all(|name| set.get_available(name).is_some()) {
            return Ok(None);
        }
        // Disabled/failed generations have no live binding and disappear from
        // VDB listings. Their persisted activations still explain unavailable
        // query targets, and reserve the name against accidental writes.
        let plugins = self
            .app
            .plugins(&self.principal, self.scope.clone())
            .await
            .map_err(app_db_error)?;
        for activation in plugins.list().await.map_err(app_db_error)? {
            for name in activation_names(&activation) {
                if names.contains(&name) && set.get_available(&name).is_none() {
                    return Ok(Some(DbError::InvalidQuery(format!(
                        "virtual database '{name}' is unavailable: plugin is disabled or failed"
                    ))));
                }
            }
        }
        Ok(None)
    }

    async fn reserved_names(&self) -> Result<BTreeSet<String>, DbError> {
        let plugins = self
            .app
            .plugins(&self.principal, self.scope.clone())
            .await
            .map_err(app_db_error)?;
        let mut names = BTreeSet::new();
        for activation in plugins.list().await.map_err(app_db_error)? {
            names.extend(activation_names(&activation));
        }
        Ok(names)
    }
}

fn app_db_error(error: crate::AppError) -> DbError {
    match error {
        crate::AppError::Db(error) => error,
        error => DbError::InvalidQuery(error.to_string()),
    }
}

fn activation_names(activation: &semantic_data::plugin::PluginActivation) -> Vec<String> {
    let exports: Vec<_> = activation
        .exports
        .iter()
        .filter(|export| {
            export.package == semantic_vdb::PACKAGE_NAME
                && export.module == semantic_vdb::MODULE_NAME
                && export.contract.is_none()
                && export.interface == semantic_vdb::INTERFACE_NAME
        })
        .collect();
    exports
        .iter()
        .map(|export| {
            if exports.len() == 1 {
                activation.id.clone()
            } else {
                format!("{}.{}", activation.id, export.export)
            }
        })
        .collect()
}

pub(crate) struct FederatedScopeDb {
    inner: Arc<dyn SemanticDb>,
    vdbs: VdbAccess,
}

impl FederatedScopeDb {
    pub(crate) fn new(inner: Arc<dyn SemanticDb>, vdbs: VdbAccess) -> Self {
        Self { inner, vdbs }
    }

    fn is_local(catalog: &Catalog, name: &str) -> bool {
        name == DEFAULT_COLLECTION || name == "all" || catalog.collection_by_name(name).is_some()
    }

    async fn parse_for_routing(&self, sql: &str) -> Option<Query> {
        match self.inner.parse_sql(sql.to_owned()).await {
            Ok(query) => Some(query),
            Err(_) => semantic_db_core::sql::parse_sql_query_unbound(
                sql,
                semantic_db_core::sql::SqlDialectKind::Generic,
            )
            .ok()
            .map(Into::into),
        }
    }

    async fn route(
        &self,
        query: Query,
        params: &BTreeMap<String, Value>,
    ) -> Result<Option<QueryResult>, DbError> {
        let local = self.inner.catalog().await?;
        let mut names = referenced_collections(&query);
        names.extend(write_collections(&query));
        names.extend(unresolved_join_collections(&query, &local));
        if names.iter().all(|name| Self::is_local(&local, name))
            || !self.vdbs.may_have_virtual_databases(&local)
        {
            return Ok(None);
        }
        self.check_writes(write_collections(&query)).await?;
        if !matches!(query, Query::Select(_)) {
            let reserved = self.vdbs.reserved_names().await?;
            if names
                .iter()
                .any(|name| !Self::is_local(&local, name) && reserved.contains(name))
            {
                return Err(DbError::InvalidQuery(
                    "only SELECT queries can read virtual databases".into(),
                ));
            }
            return Ok(None);
        }
        let (cache, set) = self.vdbs.snapshot(local.clone()).await?;
        let unknown: BTreeSet<_> = names
            .into_iter()
            .filter(|name| !Self::is_local(&local, name))
            .collect();
        if let Some(error) = self.vdbs.unavailable(&unknown, &set).await? {
            return Err(error);
        }
        let available: BTreeSet<_> = set.sources.keys().cloned().collect();
        let mut query = query;
        normalize_virtual_joins(&mut query, &local, &available);
        let references = referenced_collections(&query);
        if !references
            .iter()
            .any(|name| set.get_available(name).is_some())
        {
            return Ok(None);
        }
        let Query::Select(query) = query else {
            return Err(DbError::InvalidQuery(
                "only SELECT queries can read virtual databases".into(),
            ));
        };
        self.execute(query, params, local, cache, set, false)
            .await
            .map(|output| match output {
                FederatedOutput::Rows(rows) => Some(QueryResult::Select(rows)),
                FederatedOutput::Explain(_) => unreachable!(),
            })
    }

    pub(crate) async fn explain(
        &self,
        mut query: SelectQuery,
        params: &BTreeMap<String, Value>,
    ) -> Result<FederatedExplain, DbError> {
        let local = self.inner.catalog().await?;
        let (cache, set) = self.vdbs.snapshot(local.clone()).await?;
        let mut ast = Query::Select(query);
        normalize_virtual_joins(&mut ast, &local, &set.sources.keys().cloned().collect());
        let names = referenced_collections(&ast);
        let unknown: BTreeSet<_> = names
            .iter()
            .filter(|name| !Self::is_local(&local, name))
            .cloned()
            .collect();
        if let Some(error) = self.vdbs.unavailable(&unknown, &set).await? {
            return Err(error);
        }
        if !names.iter().any(|name| set.get_available(name).is_some()) {
            return Err(DbError::InvalidQuery(
                "query references no virtual database".into(),
            ));
        }
        let Query::Select(normalized) = ast else {
            unreachable!()
        };
        query = normalized;
        match self.execute(query, params, local, cache, set, true).await? {
            FederatedOutput::Explain(explain) => Ok(explain),
            FederatedOutput::Rows(_) => unreachable!(),
        }
    }

    async fn execute(
        &self,
        query: SelectQuery,
        params: &BTreeMap<String, Value>,
        mut local: Arc<Catalog>,
        mut cache: Arc<ScopeVdbs>,
        mut set: VdbSet,
        explain: bool,
    ) -> Result<FederatedOutput, DbError> {
        for attempt in 0..2 {
            let engine = FederatedEngine::new(
                local.clone(),
                FederationSources {
                    local: Arc::new(LocalSource {
                        inner: self.inner.clone(),
                    }),
                    virtual_sources: set.sources.clone(),
                },
            );
            let output = if explain {
                engine
                    .explain(query.clone(), params)
                    .await
                    .map(FederatedOutput::Explain)
            } else {
                engine
                    .select(query.clone(), params)
                    .await
                    .map(FederatedOutput::Rows)
            };
            match output {
                Ok(output) => return Ok(output),
                Err(FederatedError::Db(error)) => return Err(error),
                Err(FederatedError::SchemaChanged { collection }) if attempt == 0 => {
                    if let Some(schema) = set.get_available(&collection) {
                        cache.invalidate(&collection, &schema.schema_revision);
                    }
                    local = self.inner.catalog().await?;
                    (cache, set) = self.vdbs.snapshot(local.clone()).await?;
                    let names = referenced_collections(&Query::Select(query.clone()))
                        .into_iter()
                        .filter(|name| !Self::is_local(&local, name))
                        .collect();
                    if let Some(error) = self.vdbs.unavailable(&names, &set).await? {
                        return Err(error);
                    }
                }
                Err(FederatedError::SchemaChanged { collection }) => {
                    return Err(DbError::InvalidQuery(format!(
                        "schema_changed: {collection}"
                    )));
                }
            }
        }
        unreachable!("bounded retry returns on second attempt")
    }

    async fn check_writes(&self, targets: BTreeSet<String>) -> Result<(), DbError> {
        let local = self.inner.catalog().await?;
        let targets: BTreeSet<_> = targets
            .into_iter()
            .filter(|name| !Self::is_local(&local, name))
            .collect();
        if targets.is_empty() || !self.vdbs.may_have_virtual_databases(&local) {
            return Ok(());
        }
        // Reject writes using persisted export metadata before describe,
        // negotiate, or scan can be invoked on a virtual database.
        let reserved = self.vdbs.reserved_names().await?;
        for name in &targets {
            if reserved.contains(name) {
                return Err(read_only(name));
            }
        }
        Ok(())
    }
}

enum FederatedOutput {
    Rows(Vec<Object>),
    Explain(FederatedExplain),
}

fn read_only(name: &str) -> DbError {
    DbError::InvalidQuery(format!(
        "collection '{name}' is a read-only virtual database"
    ))
}

fn write_collections(query: &Query) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    match query {
        Query::Insert(query) => {
            targets.insert(
                query
                    .collection
                    .as_deref()
                    .unwrap_or(DEFAULT_COLLECTION)
                    .into(),
            );
        }
        Query::Update(query) => {
            targets.insert(
                query
                    .collection
                    .as_deref()
                    .unwrap_or(DEFAULT_COLLECTION)
                    .into(),
            );
        }
        Query::Delete(query) => {
            targets.insert(
                query
                    .collection
                    .as_deref()
                    .unwrap_or(DEFAULT_COLLECTION)
                    .into(),
            );
        }
        Query::Ddl(query) => {
            for operation in &query.batch.operations {
                match operation {
                    public::DdlOperation::UpsertCollection { name, .. }
                    | public::DdlOperation::DeleteCollection { name } => {
                        targets.insert(name.clone());
                    }
                    public::DdlOperation::UpsertIndex { collection, .. }
                    | public::DdlOperation::DeleteIndex { collection, .. } => {
                        targets.insert(collection.clone());
                    }
                    public::DdlOperation::UpsertRelationship { relationship } => {
                        targets.insert(relationship.source_collection.clone());
                    }
                    _ => {}
                }
            }
        }
        Query::Select(_) => {}
    }
    targets
}

fn batch_collections(batch: &Batch) -> BTreeSet<String> {
    batch
        .operations
        .iter()
        .map(|operation| match operation {
            BatchOperation::Create { collection, .. }
            | BatchOperation::Upsert { collection, .. }
            | BatchOperation::DeleteById { collection, .. }
            | BatchOperation::DeleteByIds { collection, .. }
            | BatchOperation::Update { collection, .. }
            | BatchOperation::Delete { collection, .. } => collection.clone(),
        })
        .collect()
}

#[async_trait]
impl SemanticDb for FederatedScopeDb {
    async fn query(&self, input: TextQueryInput) -> Result<QueryResult, DbError> {
        let (query, params) = match &input {
            TextQueryInput::Ast(query) => (query.clone().into(), BTreeMap::new()),
            TextQueryInput::AstWithParams { query, params } => {
                (query.clone().into(), params.clone())
            }
            TextQueryInput::Text {
                format: semantic_db_core::TextQueryFormat::Prql,
                ..
            } => return self.inner.query(input).await,
            TextQueryInput::Text { query, params, .. } => {
                let Some(query) = self.parse_for_routing(query).await else {
                    return self.inner.query(input).await;
                };
                (query, params.clone())
            }
        };
        match self.route(query, &params).await? {
            Some(result) => Ok(result),
            None => self.inner.query(input).await,
        }
    }

    async fn query_data(&self, input: QueryInput) -> Result<QueryResult, DbError> {
        let (query, params) = match &input {
            QueryInput::Ast(query) => (query.clone(), BTreeMap::new()),
            QueryInput::AstWithParams { query, params } => (query.clone(), params.clone()),
            QueryInput::Text {
                format: public::TextQueryFormat::Prql,
                ..
            } => return self.inner.query_data(input).await,
            QueryInput::Text { query, params, .. } => {
                let Some(query) = self.parse_for_routing(query).await else {
                    return self.inner.query_data(input).await;
                };
                (query, params.clone())
            }
        };
        match self.route(query, &params).await? {
            Some(result) => Ok(result),
            None => self.inner.query_data(input).await,
        }
    }

    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError> {
        let local = self.inner.catalog().await?;
        if Self::is_local(&local, &collection) {
            return self.inner.get(collection, id).await;
        }
        let query = SelectQuery::new()
            .with_collection(&collection)
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(public::Operand::Field(
                    semantic_data::value::FieldPath::from_fields(["id"]),
                ))),
                right: Box::new(Expr::Operand(public::Operand::Literal(Value::String(
                    id.clone(),
                )))),
            })
            .with_limit(1usize)
            .with_field_format(FieldFormat::Qualified);
        match self.route(Query::Select(query), &BTreeMap::new()).await? {
            Some(QueryResult::Select(rows)) => {
                Ok(rows.into_iter().next().map(|object| EntityRecord {
                    collection,
                    id,
                    object,
                }))
            }
            Some(_) => unreachable!("SELECT route"),
            None => self.inner.get(collection, id).await,
        }
    }

    async fn insert(&self, collection: String, id: String, object: Object) -> Result<(), DbError> {
        self.check_writes(BTreeSet::from([collection.clone()]))
            .await?;
        self.inner.insert(collection, id, object).await
    }

    async fn delete(&self, collection: String, id: String) -> Result<(), DbError> {
        self.check_writes(BTreeSet::from([collection.clone()]))
            .await?;
        self.inner.delete(collection, id).await
    }

    async fn execute_batch(&self, batch: Batch) -> Result<semantic_db_core::BatchOutcome, DbError> {
        self.check_writes(batch_collections(&batch)).await?;
        self.inner.execute_batch(batch).await
    }

    async fn execute_batch_with_settings(
        &self,
        batch: Batch,
        settings: semantic_db_core::WriteSettings,
    ) -> Result<semantic_db_core::BatchOutcome, DbError> {
        self.check_writes(batch_collections(&batch)).await?;
        self.inner
            .execute_batch_with_settings(batch, settings)
            .await
    }

    async fn execute_batch_returning(
        &self,
        batch: Batch,
        returning: semantic_db_core::BatchReturn,
    ) -> Result<semantic_db_core::BatchReply, DbError> {
        self.check_writes(batch_collections(&batch)).await?;
        self.inner.execute_batch_returning(batch, returning).await
    }

    async fn execute_batch_returning_with_settings(
        &self,
        batch: Batch,
        returning: semantic_db_core::BatchReturn,
        settings: semantic_db_core::WriteSettings,
    ) -> Result<semantic_db_core::BatchReply, DbError> {
        self.check_writes(batch_collections(&batch)).await?;
        self.inner
            .execute_batch_returning_with_settings(batch, returning, settings)
            .await
    }

    async fn execute_batch_returning_bounded_with_settings(
        &self,
        batch: Batch,
        returning: semantic_db_core::BatchReturn,
        settings: semantic_db_core::WriteSettings,
    ) -> Result<semantic_db_core::BatchReply, DbError> {
        self.check_writes(batch_collections(&batch)).await?;
        self.inner
            .execute_batch_returning_bounded_with_settings(batch, returning, settings)
            .await
    }

    async fn catalog(&self) -> Result<Arc<Catalog>, DbError> {
        self.inner.catalog().await
    }
    async fn parse_sql(&self, sql: String) -> Result<Query, DbError> {
        self.inner.parse_sql(sql).await
    }
    async fn scan_entities(&self) -> Result<semantic_db_core::EntityStream, DbError> {
        self.inner.scan_entities().await
    }
    async fn validation_preflight(
        &self,
    ) -> Result<Vec<semantic_db_core::ValidationViolation>, DbError> {
        self.inner.validation_preflight().await
    }
    async fn activate_validation(&self) -> Result<(), DbError> {
        self.inner.activate_validation().await
    }
    async fn reindex(
        &self,
        target: semantic_db_core::ReindexTarget,
    ) -> Result<semantic_db_core::ReindexReport, DbError> {
        self.inner.reindex(target).await
    }
    async fn verify(
        &self,
        options: semantic_db_core::VerifyOptions,
    ) -> Result<semantic_db_core::VerifyReport, DbError> {
        self.inner.verify(options).await
    }
    async fn repair(
        &self,
        options: semantic_db_core::VerifyOptions,
    ) -> Result<semantic_db_core::RepairReport, DbError> {
        self.inner.repair(options).await
    }
    async fn compact_storage(&self) -> Result<semantic_db_core::CompactReport, DbError> {
        self.inner.compact_storage().await
    }
    async fn storage_stats(&self) -> Result<semantic_db_core::embedded::StorageStats, DbError> {
        self.inner.storage_stats().await
    }
    async fn rewrite_payloads(
        &self,
        batch_size: usize,
    ) -> Result<semantic_db_core::RewriteReport, DbError> {
        self.inner.rewrite_payloads(batch_size).await
    }
    async fn backup(
        &self,
        path: std::path::PathBuf,
    ) -> Result<semantic_db_core::BackupReport, DbError> {
        self.inner.backup(path).await
    }
    async fn upsert_package(
        &self,
        package: semantic_data::schema::Package,
    ) -> Result<semantic_db_core::PackageRegistrationOutcome, DbError> {
        self.inner.upsert_package(package).await
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::schema::Package;
    use semantic_db_core::{
        BatchOutcome, BatchReturn, WriteSettings,
        catalog::{CollectionKind, IntegrityMode},
    };
    use std::sync::{
        Mutex, RwLock,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    struct CountingDb {
        parse_supported: bool,
        catalog: RwLock<Arc<Catalog>>,
        records: Mutex<BTreeMap<(String, String), Object>>,
        calls: Mutex<Vec<&'static str>>,
        inputs: Mutex<Vec<QueryInput>>,
    }

    impl CountingDb {
        fn new() -> Self {
            let mut catalog = Catalog::new();
            catalog
                .upsert_collection("local", CollectionKind::Untyped, IntegrityMode::Permissive)
                .unwrap();
            Self {
                parse_supported: true,
                catalog: RwLock::new(Arc::new(catalog)),
                records: Mutex::new(BTreeMap::new()),
                calls: Mutex::new(vec![]),
                inputs: Mutex::new(vec![]),
            }
        }

        fn called(&self, name: &'static str) {
            self.calls.lock().unwrap().push(name);
        }
        fn forwarded(&self, name: &'static str) -> DbError {
            self.called(name);
            DbError::InvalidQuery(format!("forwarded {name}"))
        }
    }

    #[async_trait]
    impl SemanticDb for CountingDb {
        async fn catalog(&self) -> Result<Arc<Catalog>, DbError> {
            Ok(self.catalog.read().unwrap().clone())
        }
        async fn query(&self, input: TextQueryInput) -> Result<QueryResult, DbError> {
            self.called("query");
            self.query_data(match input {
                TextQueryInput::Ast(query) => QueryInput::Ast(query.into()),
                TextQueryInput::AstWithParams { query, params } => QueryInput::AstWithParams {
                    query: query.into(),
                    params,
                },
                TextQueryInput::Text {
                    format,
                    query,
                    params,
                } => QueryInput::Text {
                    format: match format {
                        semantic_db_core::TextQueryFormat::Sql => public::TextQueryFormat::Sql,
                        semantic_db_core::TextQueryFormat::Prql => public::TextQueryFormat::Prql,
                    },
                    query,
                    params,
                },
            })
            .await
        }
        async fn parse_sql(&self, sql: String) -> Result<Query, DbError> {
            if !self.parse_supported {
                return Err(DbError::InvalidQuery(
                    "SQL parsing is not implemented by this database".into(),
                ));
            }
            semantic_db_core::sql::parse_sql_query_unbound(
                &sql,
                semantic_db_core::sql::SqlDialectKind::Generic,
            )
            .map(Into::into)
            .map_err(DbError::from)
        }
        async fn query_data(&self, input: QueryInput) -> Result<QueryResult, DbError> {
            self.called("query_data");
            self.inputs.lock().unwrap().push(input.clone());
            let query = match input {
                QueryInput::Ast(query) | QueryInput::AstWithParams { query, .. } => query,
                QueryInput::Text { .. } => return Ok(QueryResult::Select(vec![])),
            };
            match query {
                Query::Select(query) => {
                    let collection = query.collection.as_deref().unwrap_or(DEFAULT_COLLECTION);
                    let rows: Vec<_> = self
                        .records
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|((name, _), _)| name == collection)
                        .map(|(_, row)| row.clone())
                        .collect();
                    if let [field] = query.projection.as_slice() {
                        if matches!(
                            field.expr.as_ref(),
                            Expr::Aggregate {
                                op: public::AggregateOp::Count,
                                ..
                            }
                        ) {
                            let mut count = Object::new();
                            count.insert(
                                field.alias.as_deref().unwrap_or("count"),
                                Value::U64(rows.len() as u64),
                            );
                            return Ok(QueryResult::Select(vec![count]));
                        }
                    }
                    Ok(QueryResult::Select(rows))
                }
                Query::Ddl(_) => Ok(QueryResult::Ddl(())),
                _ => Err(DbError::InvalidQuery("test database read only".into())),
            }
        }
        async fn get(
            &self,
            collection: String,
            id: String,
        ) -> Result<Option<EntityRecord>, DbError> {
            self.called("get");
            Ok(self
                .records
                .lock()
                .unwrap()
                .get(&(collection.clone(), id.clone()))
                .cloned()
                .map(|object| EntityRecord {
                    collection,
                    id,
                    object,
                }))
        }
        async fn insert(
            &self,
            collection: String,
            id: String,
            object: Object,
        ) -> Result<(), DbError> {
            self.called("insert");
            self.records
                .lock()
                .unwrap()
                .insert((collection, id), object);
            Ok(())
        }
        async fn delete(&self, collection: String, id: String) -> Result<(), DbError> {
            self.called("delete");
            self.records.lock().unwrap().remove(&(collection, id));
            Ok(())
        }
        async fn execute_batch(&self, batch: Batch) -> Result<BatchOutcome, DbError> {
            self.called("execute_batch");
            for operation in batch.operations {
                if let BatchOperation::Create {
                    collection,
                    id,
                    object,
                }
                | BatchOperation::Upsert {
                    collection,
                    id,
                    object,
                } = operation
                {
                    self.records
                        .lock()
                        .unwrap()
                        .insert((collection, id), object);
                }
            }
            Ok(BatchOutcome {
                stats: semantic_db_core::BatchStats {
                    upserted: 0,
                    deleted: 0,
                    updated: 0,
                },
                dataset: BTreeMap::new(),
                metrics: Default::default(),
            })
        }
        async fn upsert_package(
            &self,
            package: Package,
        ) -> Result<semantic_db_core::PackageRegistrationOutcome, DbError> {
            let package = semantic_db_core::normalize_package_definition(&package)
                .map_err(|error| DbError::InvalidQuery(error.to_string()))?;
            let mut catalog = self.catalog.write().unwrap();
            Arc::make_mut(&mut catalog).upsert_package(package);
            Ok(semantic_db_core::PackageRegistrationOutcome {
                executed_migrations: vec![],
            })
        }
        async fn validation_preflight(
            &self,
        ) -> Result<Vec<semantic_db_core::ValidationViolation>, DbError> {
            Err(self.forwarded("validation_preflight"))
        }
        async fn activate_validation(&self) -> Result<(), DbError> {
            Err(self.forwarded("activate_validation"))
        }
        async fn scan_entities(&self) -> Result<semantic_db_core::EntityStream, DbError> {
            Err(self.forwarded("scan_entities"))
        }
        async fn execute_batch_returning_bounded_with_settings(
            &self,
            _batch: Batch,
            _returning: BatchReturn,
            _settings: WriteSettings,
        ) -> Result<semantic_db_core::BatchReply, DbError> {
            Err(self.forwarded("bounded_batch"))
        }
    }

    fn wrapper() -> (Arc<CountingDb>, FederatedScopeDb) {
        let inner = Arc::new(CountingDb::new());
        let app = SemanticApp::builder().build().unwrap();
        // This scope deliberately doesn't exist: any attempted activation on a
        // local-only operation makes the test fail immediately.
        let access = VdbAccess::new(app, Principal::system(), DbScopeId::new("missing"));
        (inner.clone(), FederatedScopeDb::new(inner, access))
    }

    #[tokio::test]
    async fn text_only_backend_preserves_sql_text_and_parameters() {
        let mut database = CountingDb::new();
        database.parse_supported = false;
        let inner = Arc::new(database);
        let app = SemanticApp::builder().build().unwrap();
        let wrapper = FederatedScopeDb::new(
            inner.clone(),
            VdbAccess::new(app, Principal::system(), DbScopeId::new("missing")),
        );
        for sql in [
            "SELECT id FROM local WHERE id = :key",
            "select * from _",
            "SELECT id FROM private_backend_table WHERE id = :key",
            "BACKEND QUERY :key",
        ] {
            let params = BTreeMap::from([("key".into(), Value::String("one".into()))]);
            let expected = QueryInput::sql_with_params(sql, params.clone());
            wrapper
                .query(TextQueryInput::Text {
                    format: semantic_db_core::TextQueryFormat::Sql,
                    query: sql.into(),
                    params,
                })
                .await
                .unwrap();
            assert_eq!(inner.inputs.lock().unwrap().last(), Some(&expected));
            wrapper.query_data(expected.clone()).await.unwrap();
            assert_eq!(inner.inputs.lock().unwrap().last(), Some(&expected));
        }
        assert!(
            wrapper
                .parse_sql("SELECT id FROM local".into())
                .await
                .unwrap_err()
                .to_string()
                .contains("SQL parsing is not implemented")
        );
    }

    #[tokio::test]
    async fn forwards_default_methods_to_inner_database() {
        let (inner, wrapper) = wrapper();
        let db: &dyn SemanticDb = &wrapper;
        assert!(
            db.validation_preflight()
                .await
                .unwrap_err()
                .to_string()
                .contains("forwarded")
        );
        assert!(
            db.activate_validation()
                .await
                .unwrap_err()
                .to_string()
                .contains("forwarded")
        );
        assert!(
            matches!(db.scan_entities().await, Err(DbError::InvalidQuery(message)) if message == "forwarded scan_entities")
        );
        assert!(
            db.execute_batch_returning_bounded_with_settings(
                Batch::new(),
                BatchReturn::Stats,
                WriteSettings::default()
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("forwarded")
        );
        assert_eq!(
            *inner.calls.lock().unwrap(),
            vec![
                "validation_preflight",
                "activate_validation",
                "scan_entities",
                "bounded_batch"
            ]
        );
    }

    #[tokio::test]
    async fn local_inputs_and_ddl_forward_without_activating_plugins() {
        let (inner, wrapper) = wrapper();
        let inputs = [
            QueryInput::ast_with_params(
                SelectQuery::new()
                    .with_collection("local")
                    .with_predicate(Expr::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(Expr::Operand(public::Operand::Field(
                            semantic_data::value::FieldPath::from_fields(["id"]),
                        ))),
                        right: Box::new(Expr::parameter("key")),
                    }),
                BTreeMap::from([("key".into(), Value::String("one".into()))]),
            ),
            QueryInput::sql("SELECT id FROM local"),
            QueryInput::prql("from local"),
            public::DdlQuery {
                batch: public::DdlBatch {
                    operations: vec![public::DdlOperation::DeleteCollection {
                        name: "local".into(),
                    }],
                },
            }
            .into(),
        ];
        for input in &inputs {
            wrapper.query_data(input.clone()).await.unwrap();
        }
        assert_eq!(*inner.inputs.lock().unwrap(), inputs);
        wrapper
            .insert("local".into(), "one".into(), Object::new())
            .await
            .unwrap();
        wrapper.delete("local".into(), "one".into()).await.unwrap();
        wrapper
            .query(TextQueryInput::prql("from local"))
            .await
            .unwrap();
    }

    #[derive(Clone)]
    struct NeverInvoked(Arc<AtomicUsize>);

    #[async_trait]
    impl semantic_vdb::VirtualDatabase for NeverInvoked {
        async fn describe(
            &self,
        ) -> Result<semantic_vdb::DatabaseDescriptor, semantic_vdb::VdbError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(semantic_vdb::DatabaseDescriptor {
                title: "never".into(),
                description: None,
                schema: Default::default(),
                schema_revision: "1".into(),
                allow_untyped: true,
            })
        }
        fn schema_revision(&self) -> String {
            "1".into()
        }
        async fn negotiate(
            &self,
            request: &ScanRequest,
        ) -> Result<ScanPlan, semantic_vdb::VdbError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ScanPlan::Accepted {
                plan: AcceptedScan {
                    filters: vec![FilterSupport::Unsupported; request.filters.len()],
                    ordered_prefix: 0,
                    limit_applied: false,
                    offset_applied: false,
                    estimated_rows: None,
                    token: None,
                    schema_revision: "1".into(),
                },
            })
        }
        fn scan(
            &self,
            _request: ScanRequest,
            _plan: AcceptedScan,
            _bindings: Object,
            _cancellation: semantic_vdb::CancellationToken,
        ) -> semantic_vdb::EntityStream {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(futures_util::stream::empty())
        }
    }

    #[tokio::test]
    async fn text_only_backend_routes_virtual_reads_and_rejects_writes() {
        let mut database = CountingDb::new();
        database.parse_supported = false;
        let inner = Arc::new(database);
        let calls = Arc::new(AtomicUsize::new(0));
        let database = NeverInvoked(calls.clone());
        let mut catalog = Catalog::new();
        catalog.upsert_package(semantic_data::bundles::query::package());
        catalog.upsert_package(semantic_vdb::package());
        let plugin = semantic_vdb::VirtualDatabasePlugin::new(
            semantic_plugin::PluginManifest {
                id: "fx".into(),
                revision: "1".into(),
                title: "fixture".into(),
                exports: vec![
                    semantic_vdb::implementation_descriptor(&catalog, "database").unwrap(),
                ],
                configuration_schema: None,
                source_bindings: BTreeMap::new(),
            },
            move |_context| {
                let database = database.clone();
                async move { Ok(database) }
            },
        );
        let app = SemanticApp::builder()
            .with_default_scope(DbScopeId::new("main"), inner.clone())
            .register_plugin(plugin)
            .unwrap()
            .build()
            .unwrap();
        let wrapper = FederatedScopeDb::new(
            inner.clone(),
            VdbAccess::new(app.clone(), Principal::system(), DbScopeId::new("main")),
        );
        for operation in [
            public::DdlOperation::UpsertCollection {
                name: "fx".into(),
                kind: public::DdlCollectionKind::Polymorphic,
                integrity_mode: public::IntegrityMode::Permissive,
            },
            public::DdlOperation::DeleteCollection { name: "fx".into() },
            public::DdlOperation::DeleteIndex {
                name: "some_index".into(),
                collection: "fx".into(),
            },
            public::DdlOperation::UpsertIndex {
                name: "some_index".into(),
                collection: "fx".into(),
                field: "id".into(),
                unique: false,
                kind: semantic_data::schema::IndexKind::Equality,
                extra_fields: vec![],
                predicate: None,
                analyzer: Default::default(),
            },
            public::DdlOperation::UpsertRelationship {
                relationship: semantic_data::schema::RelationType {
                    id: "fx:relation".into(),
                    name: "relation".into(),
                    source_collection: "fx".into(),
                    mode: semantic_data::schema::RelationMode::External,
                    indexing_mode: semantic_data::schema::RelationIndexingMode::Disabled,
                    meta: Default::default(),
                },
            },
        ] {
            let error = wrapper
                .query_data(
                    public::DdlQuery {
                        batch: public::DdlBatch {
                            operations: vec![operation],
                        },
                    }
                    .into(),
                )
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains("read-only virtual database"),
                "{error}"
            );
        }
        for query in [
            Query::Insert(public::InsertQuery::new().with_collection("fx")),
            Query::Update(public::UpdateQuery::new().with_collection("fx")),
            Query::Delete(public::DeleteQuery::new().with_collection("fx")),
        ] {
            assert!(
                wrapper
                    .query_data(query.into())
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("read-only")
            );
        }
        let mixed = public::InsertQuery::new()
            .with_collection("local")
            .with_source(public::InsertSource::Select(
                SelectQuery::new().with_collection("fx"),
            ));
        assert!(
            wrapper
                .query_data(mixed.into())
                .await
                .unwrap_err()
                .to_string()
                .contains("only SELECT")
        );
        assert!(
            wrapper
                .insert("fx".into(), "one".into(), Object::new())
                .await
                .unwrap_err()
                .to_string()
                .contains("read-only")
        );
        assert!(
            wrapper
                .delete("fx".into(), "one".into())
                .await
                .unwrap_err()
                .to_string()
                .contains("read-only")
        );
        assert!(
            wrapper
                .execute_batch(Batch {
                    operations: vec![BatchOperation::DeleteById {
                        collection: "fx".into(),
                        id: "one".into()
                    }]
                })
                .await
                .unwrap_err()
                .to_string()
                .contains("read-only")
        );
        wrapper
            .insert("fresh_local".into(), "one".into(), Object::new())
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            inner
                .catalog()
                .await
                .unwrap()
                .collection_by_name("fx")
                .is_none()
        );
        for sql in ["INSERT INTO fx (id) VALUES ('one')", "DELETE FROM fx"] {
            assert!(
                wrapper
                    .query_data(QueryInput::sql(sql))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("read-only")
            );
            assert!(
                wrapper
                    .query(TextQueryInput::sql(sql))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("read-only")
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let input = QueryInput::sql("SELECT id FROM fx");
        assert_eq!(
            wrapper.query_data(input.clone()).await.unwrap(),
            QueryResult::Select(vec![])
        );
        assert!(calls.load(Ordering::SeqCst) > 0);
        assert!(!inner.inputs.lock().unwrap().contains(&input));
        app.shutdown().await.unwrap();
    }
}
