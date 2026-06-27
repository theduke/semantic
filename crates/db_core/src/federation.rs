use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures::{StreamExt, TryStreamExt, stream};
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::{FieldPath, Object, Value};

use crate::catalog::{Catalog, CollectionKind, IntegrityMode, LocalCollectionId, SharedCatalog};
use crate::{
    AccessPath, AsyncPhysicalDataSource, Backend, Batch, BatchOperation, BatchOutcome, BatchStats,
    CoreResult, DEFAULT_EXECUTION_BATCH_SIZE, DbError, DdlBatch, DdlOutcome, DeleteQuery,
    DeleteResult, DynObject, EntityRecord, ExecutionOptions, Expr, FieldRef, InsertQuery,
    InsertResult, InsertSource, JoinSource, LogicalJoinPlan, LogicalPlan, Operand, OrderBy,
    PackageRegistrationOutcome, Query, QueryExplain, QueryField, QueryPlan, QueryResult,
    SelectQuery, SendableRecordBatchStream, SourceRef, TextQueryInput, UpdateQuery, UpdateResult,
    evaluate_filter_expr, execute_physical_plan_collect,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceNamespace {
    PrefixCollections,
    PolymorphicCollection { collection: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceCapabilities {
    pub supports_filter_pushdown: bool,
    pub supports_projection_pushdown: bool,
    pub supports_limit_pushdown: bool,
    pub supports_order_pushdown: bool,
    pub supports_aggregate_pushdown: bool,
    pub supports_join_pushdown: bool,
    pub supports_index_lookup: bool,
    pub supports_writes: bool,
    pub supports_transactions: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceScanRequest {
    pub source: String,
    pub collection: String,
    pub predicate: Option<Expr>,
    pub projection: Vec<QueryField>,
    pub order_by: Vec<OrderBy>,
    pub group_by: Vec<Expr>,
    pub having: Option<Expr>,
    pub offset: Expr,
    pub limit: Option<Expr>,
}

impl SourceScanRequest {
    fn full_scan(source: String, collection: String) -> Self {
        Self {
            source,
            collection,
            predicate: None,
            projection: Vec::new(),
            order_by: Vec::new(),
            group_by: Vec::new(),
            having: None,
            offset: Expr::from(0usize),
            limit: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceInsertRequest {
    pub source: String,
    pub collection: String,
    pub query: InsertQuery,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceUpdateRequest {
    pub source: String,
    pub collection: String,
    pub query: UpdateQuery,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceDeleteRequest {
    pub source: String,
    pub collection: String,
    pub query: DeleteQuery,
}

#[async_trait]
pub trait FederatedSource: Send + Sync + 'static {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError>;

    async fn scan(&self, request: SourceScanRequest) -> std::result::Result<Vec<Object>, DbError>;

    fn scan_stream(self: Arc<Self>, request: SourceScanRequest) -> SendableRecordBatchStream {
        stream::once(async move {
            let rows = self
                .scan(request)
                .await
                .map_err(|err| crate::CoreError::new(err.to_string()))?;
            Ok(rows
                .into_iter()
                .map(|row| Box::new(row) as DynObject)
                .collect::<Vec<_>>())
        })
        .map_ok(chunk_federated_rows)
        .try_flatten()
        .boxed()
    }

    async fn insert(
        &self,
        request: SourceInsertRequest,
    ) -> std::result::Result<InsertResult, DbError>;

    async fn update(
        &self,
        request: SourceUpdateRequest,
    ) -> std::result::Result<UpdateResult, DbError>;

    async fn delete(
        &self,
        request: SourceDeleteRequest,
    ) -> std::result::Result<DeleteResult, DbError>;
}

#[derive(Clone)]
pub struct SourceRegistration {
    pub name: String,
    pub backend: Arc<dyn FederatedSource>,
    pub namespace: SourceNamespace,
    pub capabilities: SourceCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederatedCollectionRef {
    pub source: String,
    pub source_collection: String,
    pub federated_collection: String,
}

#[derive(Clone)]
struct RegisteredSource {
    backend: Arc<dyn FederatedSource>,
    #[allow(dead_code)]
    namespace: SourceNamespace,
    capabilities: SourceCapabilities,
}

#[derive(Clone, Default)]
pub struct SourceRegistry {
    sources: BTreeMap<String, RegisteredSource>,
    collections: BTreeMap<String, FederatedCollectionRef>,
}

impl SourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains_source(&self, name: &str) -> bool {
        self.sources.contains_key(name)
    }

    pub fn source_names(&self) -> impl Iterator<Item = &str> {
        self.sources.keys().map(String::as_str)
    }

    pub fn collection_ref(&self, federated_collection: &str) -> Option<&FederatedCollectionRef> {
        self.collections.get(federated_collection)
    }

    fn source(&self, name: &str) -> std::result::Result<&RegisteredSource, DbError> {
        self.sources
            .get(name)
            .ok_or_else(|| DbError::InvalidQuery(format!("unknown federated source '{name}'")))
    }

    fn insert_source(
        &mut self,
        registration: SourceRegistration,
    ) -> std::result::Result<(), DbError> {
        validate_source_name(&registration.name)?;
        if self.sources.contains_key(&registration.name) {
            return Err(DbError::InvalidQuery(format!(
                "federated source '{}' already exists",
                registration.name
            )));
        }
        self.sources.insert(
            registration.name,
            RegisteredSource {
                backend: registration.backend,
                namespace: registration.namespace,
                capabilities: registration.capabilities,
            },
        );
        Ok(())
    }
}

pub struct FederatedBackend {
    registry: Arc<RwLock<SourceRegistry>>,
    default_source: Option<String>,
    catalog: SharedCatalog,
}

impl FederatedBackend {
    pub fn new(default_source: Option<String>) -> Self {
        Self {
            registry: Arc::new(RwLock::new(SourceRegistry::new())),
            default_source,
            catalog: SharedCatalog::new(Catalog::new()),
        }
    }

    pub async fn register_source(
        &self,
        registration: SourceRegistration,
    ) -> std::result::Result<(), DbError> {
        let source_catalog = registration.backend.catalog().await?;
        let source_name = registration.name.clone();
        let namespace = registration.namespace.clone();

        let mut registry = self.registry.write().map_err(|_| {
            DbError::Storage("federated source registry rwlock poisoned".to_string())
        })?;
        registry.insert_source(registration)?;
        rebuild_catalog(
            &mut registry,
            self.catalog.catalog_arc().as_ref(),
            &source_name,
            &namespace,
            source_catalog.as_ref(),
        )?;
        self.catalog
            .compare_and_swap(self.catalog.snapshot().version, build_catalog(&registry)?)
            .map_err(|mismatch| {
                DbError::TransactionConflict(format!(
                    "federated catalog version changed: expected {}, actual {}",
                    mismatch.expected, mismatch.actual
                ))
            })?;
        Ok(())
    }

    pub fn registry_snapshot(&self) -> std::result::Result<SourceRegistry, DbError> {
        self.registry
            .read()
            .map_err(|_| DbError::Storage("federated source registry rwlock poisoned".to_string()))
            .map(|registry| registry.clone())
    }

    fn parse_query(
        &self,
        input: TextQueryInput,
    ) -> impl std::future::Future<Output = std::result::Result<Query, DbError>> + '_ {
        async move {
            match input {
                TextQueryInput::Ast(query) => Ok(query),
                TextQueryInput::Text { format, query } => {
                    self.parse_text_query(format, &query).await
                }
            }
        }
    }

    fn resolve_collection(
        &self,
        registry: &SourceRegistry,
        collection: Option<&str>,
    ) -> std::result::Result<ResolvedCollection, DbError> {
        let Some(collection) = collection else {
            let source = self.default_source.clone().ok_or_else(|| {
                DbError::InvalidQuery(
                    "query has no source and federated backend has no default source".to_string(),
                )
            })?;
            return Ok(ResolvedCollection {
                source,
                collection: crate::DEFAULT_COLLECTION.to_string(),
                federated_collection: None,
            });
        };

        if let Some((source, local_collection)) = split_source_collection(registry, collection) {
            return Ok(ResolvedCollection {
                source,
                collection: local_collection,
                federated_collection: Some(collection.to_string()),
            });
        }

        let source = self.default_source.clone().ok_or_else(|| {
            DbError::InvalidQuery(format!(
                "collection '{collection}' has no source prefix and federated backend has no default source"
            ))
        })?;
        Ok(ResolvedCollection {
            source,
            collection: collection.to_string(),
            federated_collection: None,
        })
    }

    fn normalize_select(
        &self,
        registry: &SourceRegistry,
        query: SelectQuery,
    ) -> std::result::Result<(SelectQuery, SourceRef), DbError> {
        let base = self.resolve_collection(registry, query.collection.as_deref())?;
        let mut query = query;
        query.joins = query
            .joins
            .into_iter()
            .map(|mut join| {
                if let Some(collection) = federated_join_collection(registry, &join.source) {
                    join.source = JoinSource {
                        collection: Some(collection),
                        class: None,
                    };
                }
                join
            })
            .collect();
        let source = SourceRef {
            source_name: Some(base.collection),
            collection_id: None,
            binding: query.source_alias.clone(),
            backend_tag: Some(base.source),
        };
        Ok((query, source))
    }

    fn plan_select(
        &self,
        registry: &SourceRegistry,
        query: &SelectQuery,
    ) -> std::result::Result<crate::PlanPair, DbError> {
        let (query, source) = self.normalize_select(registry, query.clone())?;
        let optimizer = crate::Optimizer::core();
        let context = crate::QueryContext::new(self.catalog.catalog_arc());
        let mut pair = optimizer.optimize_query_with_source(&query, source, None, &context);
        pair.logical =
            resolve_logical_sources(registry, self.default_source.as_deref(), pair.logical)?;
        pair.physical = optimizer.lower_to_physical(&pair.logical, None, &context);
        Ok(pair)
    }

    async fn query_select(&self, query: SelectQuery) -> std::result::Result<Vec<Object>, DbError> {
        let registry = self.registry_snapshot()?;
        let pair = self.plan_select(&registry, &query)?;
        let source = Arc::new(FederatedAsyncPhysicalDataSource { registry });
        let context = crate::QueryContext::new(self.catalog.catalog_arc());
        execute_physical_plan_collect(pair.physical, source, context, ExecutionOptions::default())
            .await
            .map_err(|err| DbError::InvalidQuery(err.to_string()))
    }

    async fn query_insert(&self, query: InsertQuery) -> std::result::Result<InsertResult, DbError> {
        let registry = self.registry_snapshot()?;
        reject_cross_source_insert(&registry, self.default_source.as_deref(), &query)?;
        let target = self.resolve_collection(&registry, query.collection.as_deref())?;
        let source = registry.source(&target.source)?;
        if !source.capabilities.supports_writes {
            return Err(DbError::InvalidQuery(format!(
                "federated source '{}' does not support writes",
                target.source
            )));
        }
        source
            .backend
            .insert(SourceInsertRequest {
                source: target.source,
                collection: target.collection,
                query,
            })
            .await
    }

    async fn query_update(&self, query: UpdateQuery) -> std::result::Result<UpdateResult, DbError> {
        let registry = self.registry_snapshot()?;
        let target = self.resolve_collection(&registry, query.collection.as_deref())?;
        reject_cross_source_expr(
            registry.clone(),
            self.default_source.as_deref(),
            query.predicate.as_ref(),
            &target.source,
        )?;
        let source = registry.source(&target.source)?;
        if !source.capabilities.supports_writes {
            return Err(DbError::InvalidQuery(format!(
                "federated source '{}' does not support writes",
                target.source
            )));
        }
        source
            .backend
            .update(SourceUpdateRequest {
                source: target.source,
                collection: target.collection,
                query,
            })
            .await
    }

    async fn query_delete(&self, query: DeleteQuery) -> std::result::Result<DeleteResult, DbError> {
        let registry = self.registry_snapshot()?;
        let target = self.resolve_collection(&registry, query.collection.as_deref())?;
        reject_cross_source_expr(
            registry.clone(),
            self.default_source.as_deref(),
            query.predicate.as_ref(),
            &target.source,
        )?;
        let source = registry.source(&target.source)?;
        if !source.capabilities.supports_writes {
            return Err(DbError::InvalidQuery(format!(
                "federated source '{}' does not support writes",
                target.source
            )));
        }
        source
            .backend
            .delete(SourceDeleteRequest {
                source: target.source,
                collection: target.collection,
                query,
            })
            .await
    }
}

#[async_trait]
impl Backend for FederatedBackend {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        Ok(self.catalog.catalog_arc())
    }

    async fn create_collection(
        &self,
        _name: String,
        _kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        Err(DbError::InvalidQuery(
            "create_collection is not supported on federated backend; create collections on a concrete source".to_string(),
        ))
    }

    async fn execute_ddl(&self, _ddl: DdlBatch) -> std::result::Result<DdlOutcome, DbError> {
        Err(DbError::InvalidQuery(
            "DDL is not supported on federated backend".to_string(),
        ))
    }

    async fn upsert_package(
        &self,
        _package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
        Err(DbError::InvalidQuery(
            "package registration is not supported on federated backend".to_string(),
        ))
    }

    async fn upsert_relationship(
        &self,
        _relationship: RelationType,
    ) -> std::result::Result<(), DbError> {
        Err(DbError::InvalidQuery(
            "relationship registration is not supported on federated backend".to_string(),
        ))
    }

    async fn delete_relationship(&self, _id: String) -> std::result::Result<(), DbError> {
        Err(DbError::InvalidQuery(
            "relationship deletion is not supported on federated backend".to_string(),
        ))
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        mut object: Object,
    ) -> std::result::Result<(), DbError> {
        object.insert(
            crate::catalog::PRIMARY_ID_FIELD.to_string(),
            Value::String(id),
        );
        self.query_insert(InsertQuery {
            collection: Some(collection),
            columns: Vec::new(),
            source: InsertSource::Objects(vec![object]),
            returning: Vec::new(),
            field_format: semantic_data::query::FieldFormat::Plain,
        })
        .await
        .map(|_| ())
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let registry = self.registry_snapshot()?;
        let target = self.resolve_collection(&registry, Some(&collection))?;
        let source = registry.source(&target.source)?;
        let predicate = Expr::Binary {
            op: semantic_data::query::BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                crate::catalog::PRIMARY_ID_FIELD,
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(id.clone())))),
        };
        let mut rows = source
            .backend
            .scan(SourceScanRequest {
                predicate: Some(predicate),
                limit: Some(Expr::from(1usize)),
                ..SourceScanRequest::full_scan(target.source.clone(), target.collection.clone())
            })
            .await?;
        Ok(rows.pop().map(|object| EntityRecord {
            id,
            collection: target
                .federated_collection
                .unwrap_or_else(|| collection.to_string()),
            object,
        }))
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        let predicate = Expr::Binary {
            op: semantic_data::query::BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                crate::catalog::PRIMARY_ID_FIELD,
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(id)))),
        };
        self.query_delete(
            DeleteQuery::new()
                .with_collection(collection)
                .with_predicate(predicate),
        )
        .await
        .map(|_| ())
    }

    async fn query(&self, input: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
        match self.parse_query(input).await? {
            Query::Select(query) => self.query_select(query).await.map(QueryResult::Select),
            Query::Insert(query) => self.query_insert(query).await.map(QueryResult::Insert),
            Query::Update(query) => self.query_update(query).await.map(QueryResult::Update),
            Query::Delete(query) => self.query_delete(query).await.map(QueryResult::Delete),
            Query::Ddl(_) => Err(DbError::InvalidQuery(
                "DDL is not supported on federated backend".to_string(),
            )),
        }
    }

    async fn explain(&self, input: TextQueryInput) -> std::result::Result<QueryExplain, DbError> {
        let query = self.parse_query(input).await?;
        let Query::Select(select) = query else {
            return Err(DbError::InvalidQuery(
                "federated explain currently supports SELECT only".to_string(),
            ));
        };
        let registry = self.registry_snapshot()?;
        let pair = self.plan_select(&registry, &select)?;
        Ok(QueryExplain {
            logical: pair.logical,
            physical: pair.physical,
            access_path: AccessPath::FullScan,
        })
    }

    async fn plan(&self, input: TextQueryInput) -> std::result::Result<QueryPlan, DbError> {
        let query = self.parse_query(input).await?;
        let collection = query.collection_or_default().to_string();
        let _ = self.explain(TextQueryInput::Ast(query)).await?;
        Ok(QueryPlan::FullScan { collection })
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        if batch.operations.is_empty() {
            return Ok(BatchOutcome {
                stats: BatchStats {
                    upserted: 0,
                    deleted: 0,
                    updated: 0,
                },
                dataset: BTreeMap::new(),
            });
        }

        let registry = self.registry_snapshot()?;
        let mut source_name = None::<String>;
        for operation in &batch.operations {
            let collection = match operation {
                BatchOperation::Upsert { collection, .. }
                | BatchOperation::DeleteById { collection, .. }
                | BatchOperation::DeleteByIds { collection, .. }
                | BatchOperation::Update { collection, .. }
                | BatchOperation::Delete { collection, .. } => collection.as_str(),
            };
            let resolved = self.resolve_collection(&registry, Some(collection))?;
            if let Some(existing) = &source_name {
                if existing != &resolved.source {
                    return Err(DbError::InvalidQuery(
                        "federated batches may target only one source".to_string(),
                    ));
                }
            } else {
                source_name = Some(resolved.source);
            }
        }

        Err(DbError::InvalidQuery(
            "federated batch routing is reserved for source-native batch support".to_string(),
        ))
    }
}

#[derive(Debug, Clone)]
struct ResolvedCollection {
    source: String,
    collection: String,
    federated_collection: Option<String>,
}

struct FederatedAsyncPhysicalDataSource {
    registry: SourceRegistry,
}

impl FederatedAsyncPhysicalDataSource {
    fn request_for_source(&self, source: &SourceRef) -> CoreResult<SourceScanRequest> {
        let source_name = source.backend_tag.clone().ok_or_else(|| {
            crate::CoreError::new("federated physical source is missing backend tag")
        })?;
        let collection = source.source_name.clone().ok_or_else(|| {
            crate::CoreError::new("federated physical source is missing collection name")
        })?;
        Ok(SourceScanRequest::full_scan(source_name, collection))
    }

    fn stream_request(&self, request: SourceScanRequest) -> SendableRecordBatchStream {
        let registered = match self.registry.source(&request.source) {
            Ok(source) => source.clone(),
            Err(err) => {
                return stream::once(async move { Err(crate::CoreError::new(err.to_string())) })
                    .boxed();
            }
        };
        registered.backend.scan_stream(request)
    }
}

impl AsyncPhysicalDataSource for FederatedAsyncPhysicalDataSource {
    fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
        match self.request_for_source(&source) {
            Ok(request) => self.stream_request(request),
            Err(err) => stream::once(async move { Err(err) }).boxed(),
        }
    }

    fn scan_filtered_stream(
        &self,
        source: SourceRef,
        predicate: Expr,
    ) -> SendableRecordBatchStream {
        let mut request = match self.request_for_source(&source) {
            Ok(request) => request,
            Err(err) => return stream::once(async move { Err(err) }).boxed(),
        };
        let registered = match self.registry.source(&request.source) {
            Ok(source) => source.clone(),
            Err(err) => {
                return stream::once(async move { Err(crate::CoreError::new(err.to_string())) })
                    .boxed();
            }
        };
        if registered.capabilities.supports_filter_pushdown {
            request.predicate = Some(predicate);
            return self.stream_request(request);
        }

        self.scan_stream(source)
            .map_ok(move |batch| {
                batch
                    .into_iter()
                    .filter(|row| evaluate_filter_expr(row.as_ref(), &predicate))
                    .collect::<Vec<_>>()
            })
            .boxed()
    }

    fn index_lookup_stream(
        &self,
        source: SourceRef,
        field: FieldRef,
        value: Value,
    ) -> SendableRecordBatchStream {
        let mut request = match self.request_for_source(&source) {
            Ok(request) => request,
            Err(err) => return stream::once(async move { Err(err) }).boxed(),
        };
        let registered = match self.registry.source(&request.source) {
            Ok(source) => source.clone(),
            Err(err) => {
                return stream::once(async move { Err(crate::CoreError::new(err.to_string())) })
                    .boxed();
            }
        };
        if registered.capabilities.supports_index_lookup {
            if let Some(path) = field_path_for_ref(&field) {
                request.predicate = Some(Expr::Binary {
                    op: semantic_data::query::BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(path))),
                    right: Box::new(Expr::Operand(Operand::Literal(value))),
                });
                return self.stream_request(request);
            }
        }

        self.scan_filtered_stream(
            source,
            Expr::Binary {
                op: semantic_data::query::BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(
                    field_path_for_ref(&field).unwrap_or_else(|| FieldPath::from_fields(["id"])),
                ))),
                right: Box::new(Expr::Operand(Operand::Literal(value))),
            },
        )
    }
}

fn chunk_federated_rows(rows: Vec<DynObject>) -> SendableRecordBatchStream {
    let batch_size = DEFAULT_EXECUTION_BATCH_SIZE;
    stream::unfold(rows.into_iter(), move |mut iter| async move {
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            let Some(row) = iter.next() else {
                break;
            };
            batch.push(row);
        }
        if batch.is_empty() {
            None
        } else {
            Some((Ok(batch), iter))
        }
    })
    .boxed()
}

fn validate_source_name(name: &str) -> std::result::Result<(), DbError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(DbError::InvalidQuery(format!(
            "invalid federated source name '{name}'"
        )));
    }
    Ok(())
}

fn rebuild_catalog(
    registry: &mut SourceRegistry,
    _previous_catalog: &Catalog,
    source_name: &str,
    namespace: &SourceNamespace,
    source_catalog: &Catalog,
) -> std::result::Result<(), DbError> {
    for (_, collection) in source_catalog.collections() {
        let source_collection = match namespace {
            SourceNamespace::PrefixCollections => collection.name.clone(),
            SourceNamespace::PolymorphicCollection { collection } => collection.clone(),
        };
        let federated_collection = format!("{source_name}.{source_collection}");
        if registry.collections.contains_key(&federated_collection) {
            return Err(DbError::InvalidQuery(format!(
                "federated collection '{federated_collection}' already exists"
            )));
        }
        registry.collections.insert(
            federated_collection.clone(),
            FederatedCollectionRef {
                source: source_name.to_string(),
                source_collection,
                federated_collection,
            },
        );
    }
    Ok(())
}

fn build_catalog(registry: &SourceRegistry) -> std::result::Result<Catalog, DbError> {
    let mut catalog = Catalog::new();
    for collection_ref in registry.collections.values() {
        let _ = catalog.upsert_collection(
            collection_ref.federated_collection.clone(),
            CollectionKind::Polymorphic,
            IntegrityMode::Permissive,
        )?;
    }
    Ok(catalog)
}

fn split_source_collection(
    registry: &SourceRegistry,
    collection: &str,
) -> Option<(String, String)> {
    let (source, rest) = collection.split_once('.')?;
    if rest.is_empty() || !registry.contains_source(source) {
        return None;
    }
    Some((source.to_string(), rest.to_string()))
}

fn federated_join_collection(registry: &SourceRegistry, source: &JoinSource) -> Option<String> {
    match (&source.collection, &source.class) {
        (Some(collection), Some(class)) if registry.contains_source(collection) => {
            Some(format!("{collection}.{class}"))
        }
        _ => None,
    }
}

fn resolve_logical_sources(
    registry: &SourceRegistry,
    default_source: Option<&str>,
    plan: LogicalPlan,
) -> std::result::Result<LogicalPlan, DbError> {
    match plan {
        LogicalPlan::Source {
            source,
            pushed_predicate,
        } => Ok(LogicalPlan::Source {
            source: resolve_source_ref(registry, default_source, source)?,
            pushed_predicate,
        }),
        LogicalPlan::Values { .. } => Ok(plan),
        LogicalPlan::Filter { input, predicate } => Ok(LogicalPlan::Filter {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            predicate,
        }),
        LogicalPlan::Sort { input, order_by } => Ok(LogicalPlan::Sort {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            order_by,
        }),
        LogicalPlan::Project { input, projection } => Ok(LogicalPlan::Project {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            projection,
        }),
        LogicalPlan::Aggregate {
            input,
            group_by,
            projection,
            having,
        } => Ok(LogicalPlan::Aggregate {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            group_by,
            projection,
            having,
        }),
        LogicalPlan::Limit {
            input,
            offset,
            limit,
        } => Ok(LogicalPlan::Limit {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            offset,
            limit,
        }),
        LogicalPlan::Distinct { input } => Ok(LogicalPlan::Distinct {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
        }),
        LogicalPlan::Union { inputs, all } => Ok(LogicalPlan::Union {
            inputs: inputs
                .into_iter()
                .map(|input| resolve_logical_sources(registry, default_source, input))
                .collect::<std::result::Result<Vec<_>, _>>()?,
            all,
        }),
        LogicalPlan::Join(join) => Ok(LogicalPlan::Join(LogicalJoinPlan {
            left: Box::new(resolve_logical_sources(
                registry,
                default_source,
                *join.left,
            )?),
            right: Box::new(resolve_logical_sources(
                registry,
                default_source,
                *join.right,
            )?),
            join_type: join.join_type,
            condition: join.condition,
            left_binding: join.left_binding,
            right_binding: join.right_binding,
        })),
        LogicalPlan::ApplyExists {
            input,
            subquery,
            negated,
        } => Ok(LogicalPlan::ApplyExists {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            subquery: Box::new(resolve_logical_sources(
                registry,
                default_source,
                *subquery,
            )?),
            negated,
        }),
        LogicalPlan::ApplyInSubquery {
            input,
            left,
            subquery,
            negated,
        } => Ok(LogicalPlan::ApplyInSubquery {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            left,
            subquery: Box::new(resolve_logical_sources(
                registry,
                default_source,
                *subquery,
            )?),
            negated,
        }),
        LogicalPlan::Exchange {
            input,
            partition_count,
        } => Ok(LogicalPlan::Exchange {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            partition_count,
        }),
        LogicalPlan::RepartitionHash {
            input,
            partition_count,
            partition_keys,
        } => Ok(LogicalPlan::RepartitionHash {
            input: Box::new(resolve_logical_sources(registry, default_source, *input)?),
            partition_count,
            partition_keys,
        }),
    }
}

fn resolve_source_ref(
    registry: &SourceRegistry,
    default_source: Option<&str>,
    source: SourceRef,
) -> std::result::Result<SourceRef, DbError> {
    if source.backend_tag.is_some() {
        return Ok(source);
    }
    let Some(source_name) = source.source_name.as_deref() else {
        return Ok(source);
    };
    if crate::is_all_collection_alias(source_name) {
        return Err(DbError::InvalidQuery(
            "the 'all' collection alias is not supported by federated queries".to_string(),
        ));
    }
    if let Some((backend_tag, local_collection)) = split_source_collection(registry, source_name) {
        return Ok(SourceRef {
            source_name: Some(local_collection),
            backend_tag: Some(backend_tag),
            ..source
        });
    }
    let Some(default_source) = default_source else {
        return Err(DbError::InvalidQuery(format!(
            "source '{source_name}' is not qualified and no default source is configured"
        )));
    };
    Ok(SourceRef {
        backend_tag: Some(default_source.to_string()),
        ..source
    })
}

fn field_path_for_ref(field: &FieldRef) -> Option<FieldPath> {
    match field {
        FieldRef::CanonicalName(name) => Some(FieldPath::from_fields([name])),
        FieldRef::Path(path) => Some(path.clone()),
        FieldRef::AttrId(_) | FieldRef::FieldId(_) => None,
    }
}

fn reject_cross_source_insert(
    registry: &SourceRegistry,
    default_source: Option<&str>,
    query: &InsertQuery,
) -> std::result::Result<(), DbError> {
    if let InsertSource::Select(select) = &query.source {
        let target_source =
            resolve_collection_for_check(registry, default_source, query.collection.as_deref())?
                .source;
        reject_select_sources(registry, default_source, select, &target_source)?;
    }
    Ok(())
}

fn reject_cross_source_expr(
    registry: SourceRegistry,
    default_source: Option<&str>,
    expr: Option<&Expr>,
    target_source: &str,
) -> std::result::Result<(), DbError> {
    if let Some(expr) = expr {
        reject_expr_sources(&registry, default_source, expr, target_source)?;
    }
    Ok(())
}

fn reject_select_sources(
    registry: &SourceRegistry,
    default_source: Option<&str>,
    select: &SelectQuery,
    target_source: &str,
) -> std::result::Result<(), DbError> {
    let base =
        resolve_collection_for_check(registry, default_source, select.collection.as_deref())?;
    if base.source != target_source {
        return Err(DbError::InvalidQuery(
            "federated mutations may not read from another source".to_string(),
        ));
    }
    for join in &select.joins {
        if let Some(collection) = federated_join_collection(registry, &join.source)
            && resolve_collection_for_check(registry, default_source, Some(&collection))?.source
                != target_source
        {
            return Err(DbError::InvalidQuery(
                "federated mutations may not join another source".to_string(),
            ));
        }
    }
    if let Some(predicate) = &select.predicate {
        reject_expr_sources(registry, default_source, predicate, target_source)?;
    }
    Ok(())
}

fn reject_expr_sources(
    registry: &SourceRegistry,
    default_source: Option<&str>,
    expr: &Expr,
    target_source: &str,
) -> std::result::Result<(), DbError> {
    match expr {
        Expr::Subquery(query) => {
            reject_select_sources(registry, default_source, query, target_source)
        }
        Expr::Exists { query, .. } => {
            reject_select_sources(registry, default_source, query, target_source)
        }
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => {
            reject_expr_sources(registry, default_source, expr, target_source)
        }
        Expr::Binary { left, right, .. } => {
            reject_expr_sources(registry, default_source, left, target_source)?;
            reject_expr_sources(registry, default_source, right, target_source)
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            reject_expr_sources(registry, default_source, cond, target_source)?;
            reject_expr_sources(registry, default_source, then_expr, target_source)?;
            reject_expr_sources(registry, default_source, else_expr, target_source)
        }
        Expr::Coalesce(items) | Expr::InList { list: items, .. } => {
            for item in items {
                reject_expr_sources(registry, default_source, item, target_source)?;
            }
            if let Expr::InList { expr, .. } = expr {
                reject_expr_sources(registry, default_source, expr, target_source)?;
            }
            Ok(())
        }
        Expr::Function { args, .. } => {
            for arg in args {
                if let semantic_data::query::FunctionArg::Expr(expr) = arg {
                    reject_expr_sources(registry, default_source, expr, target_source)?;
                }
            }
            Ok(())
        }
        Expr::Aggregate { arg, .. } => {
            if let semantic_data::query::FunctionArg::Expr(expr) = arg.as_ref() {
                reject_expr_sources(registry, default_source, expr, target_source)?;
            }
            Ok(())
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            reject_expr_sources(registry, default_source, expr, target_source)?;
            reject_expr_sources(registry, default_source, low, target_source)?;
            reject_expr_sources(registry, default_source, high, target_source)
        }
        Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
            reject_expr_sources(registry, default_source, expr, target_source)?;
            reject_expr_sources(registry, default_source, pattern, target_source)
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            reject_expr_sources(registry, default_source, relation, target_source)?;
            reject_expr_sources(registry, default_source, source, target_source)?;
            reject_expr_sources(registry, default_source, target, target_source)?;
            if let Some(max_depth) = max_depth {
                reject_expr_sources(registry, default_source, max_depth, target_source)?;
            }
            Ok(())
        }
        Expr::Operand(_) => Ok(()),
    }
}

fn resolve_collection_for_check(
    registry: &SourceRegistry,
    default_source: Option<&str>,
    collection: Option<&str>,
) -> std::result::Result<ResolvedCollection, DbError> {
    if let Some(collection) = collection
        && let Some((source, local_collection)) = split_source_collection(registry, collection)
    {
        return Ok(ResolvedCollection {
            source,
            collection: local_collection,
            federated_collection: Some(collection.to_string()),
        });
    }
    let source = default_source.ok_or_else(|| {
        DbError::InvalidQuery(
            "mutation has no source and no default source is configured".to_string(),
        )
    })?;
    Ok(ResolvedCollection {
        source: source.to_string(),
        collection: collection.unwrap_or(crate::DEFAULT_COLLECTION).to_string(),
        federated_collection: None,
    })
}

pub struct BackendFederatedSource {
    backend: Arc<dyn Backend>,
}

impl BackendFederatedSource {
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Self { backend }
    }
}

#[cfg(test)]
mod tests {
    use futures::executor;
    use std::sync::Mutex;

    use super::*;
    use crate::{evaluate_usize_expr, project_object};

    struct MockSource {
        catalog: Arc<Catalog>,
        rows: BTreeMap<String, Vec<Object>>,
        scans: Mutex<Vec<SourceScanRequest>>,
        writes: Mutex<Vec<SourceInsertRequest>>,
        read_only: bool,
    }

    impl MockSource {
        fn new(collections: &[&str], rows: BTreeMap<String, Vec<Object>>) -> Self {
            let mut catalog = Catalog::new();
            for collection in collections {
                catalog
                    .upsert_collection(
                        (*collection).to_string(),
                        CollectionKind::Polymorphic,
                        IntegrityMode::Permissive,
                    )
                    .unwrap();
            }
            Self {
                catalog: Arc::new(catalog),
                rows,
                scans: Mutex::new(Vec::new()),
                writes: Mutex::new(Vec::new()),
                read_only: false,
            }
        }

        fn read_only(collections: &[&str], rows: BTreeMap<String, Vec<Object>>) -> Self {
            let mut source = Self::new(collections, rows);
            source.read_only = true;
            source
        }

        fn last_scan(&self) -> Option<SourceScanRequest> {
            self.scans.lock().unwrap().last().cloned()
        }

        fn write_count(&self) -> usize {
            self.writes.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl FederatedSource for MockSource {
        async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
            Ok(self.catalog.clone())
        }

        async fn scan(
            &self,
            request: SourceScanRequest,
        ) -> std::result::Result<Vec<Object>, DbError> {
            self.scans.lock().unwrap().push(request.clone());
            let mut rows = self
                .rows
                .get(&request.collection)
                .cloned()
                .unwrap_or_default();
            if let Some(predicate) = &request.predicate {
                rows.retain(|row| evaluate_filter_expr(row, predicate));
            }
            if !request.order_by.is_empty() {
                rows = crate::execute_query(
                    &SelectQuery::new().with_order_by(request.order_by.clone()),
                    rows,
                );
            }
            if let Some(limit) = request.limit.as_ref().and_then(evaluate_usize_expr) {
                rows.truncate(limit);
            }
            if !request.projection.is_empty() {
                rows = rows
                    .into_iter()
                    .map(|row| project_object(&row, &request.projection))
                    .collect();
            }
            Ok(rows)
        }

        async fn insert(
            &self,
            request: SourceInsertRequest,
        ) -> std::result::Result<InsertResult, DbError> {
            if self.read_only {
                return Err(DbError::InvalidQuery("read-only source".to_string()));
            }
            let inserted = match &request.query.source {
                InsertSource::Objects(rows) => rows.len(),
                InsertSource::Values(rows) => rows.len(),
                InsertSource::Select(_) => 0,
            };
            self.writes.lock().unwrap().push(request);
            Ok(InsertResult {
                inserted,
                returning: Vec::new(),
            })
        }

        async fn update(
            &self,
            _request: SourceUpdateRequest,
        ) -> std::result::Result<UpdateResult, DbError> {
            Err(DbError::InvalidQuery("not implemented".to_string()))
        }

        async fn delete(
            &self,
            _request: SourceDeleteRequest,
        ) -> std::result::Result<DeleteResult, DbError> {
            Err(DbError::InvalidQuery("not implemented".to_string()))
        }
    }

    fn row(fields: &[(&str, Value)]) -> Object {
        let mut out = Object::new();
        for (key, value) in fields {
            out.insert((*key).to_string(), value.clone());
        }
        out
    }

    fn rows(collection: &str, rows: Vec<Object>) -> BTreeMap<String, Vec<Object>> {
        BTreeMap::from([(collection.to_string(), rows)])
    }

    fn register(
        db: &FederatedBackend,
        name: &str,
        source: Arc<MockSource>,
        capabilities: SourceCapabilities,
    ) {
        executor::block_on(db.register_source(SourceRegistration {
            name: name.to_string(),
            backend: source,
            namespace: SourceNamespace::PrefixCollections,
            capabilities,
        }))
        .unwrap();
    }

    #[test]
    fn query_without_source_uses_default_source() {
        let source = Arc::new(MockSource::new(
            &["users"],
            rows(
                "users",
                vec![row(&[("id", Value::String("u1".to_string()))])],
            ),
        ));
        let db = FederatedBackend::new(Some("local".to_string()));
        register(&db, "local", source.clone(), SourceCapabilities::default());

        let out =
            executor::block_on(db.query(TextQueryInput::sql("SELECT id FROM users"))).unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(source.last_scan().unwrap().collection, "users");
    }

    #[test]
    fn query_without_source_fails_without_default_source() {
        let source = Arc::new(MockSource::new(&["users"], BTreeMap::new()));
        let db = FederatedBackend::new(None);
        register(&db, "local", source, SourceCapabilities::default());

        let err =
            executor::block_on(db.query(TextQueryInput::sql("SELECT id FROM users"))).unwrap_err();
        assert!(matches!(err, DbError::InvalidQuery(_)));
    }

    #[test]
    fn source_prefixed_query_routes_to_registered_source() {
        let source = Arc::new(MockSource::new(
            &["User"],
            rows(
                "User",
                vec![row(&[("login", Value::String("theduke".to_string()))])],
            ),
        ));
        let db = FederatedBackend::new(None);
        register(&db, "github", source.clone(), SourceCapabilities::default());

        let out =
            executor::block_on(db.query(TextQueryInput::sql("SELECT login FROM github.User")))
                .unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(
            rows[0].get("login"),
            Some(&Value::String("theduke".to_string()))
        );
        assert_eq!(source.last_scan().unwrap().source, "github");
    }

    #[test]
    fn cross_source_inner_join_executes_locally() {
        let github = Arc::new(MockSource::new(
            &["User"],
            rows(
                "User",
                vec![row(&[
                    ("id", Value::String("u1".to_string())),
                    ("login", Value::String("theduke".to_string())),
                ])],
            ),
        ));
        let local = Arc::new(MockSource::new(
            &["issues"],
            rows(
                "issues",
                vec![row(&[
                    ("author_id", Value::String("u1".to_string())),
                    ("title", Value::String("federation".to_string())),
                ])],
            ),
        ));
        let db = FederatedBackend::new(None);
        register(&db, "github", github, SourceCapabilities::default());
        register(&db, "local", local, SourceCapabilities::default());

        let sql = "SELECT u.login AS login, i.title AS title \
                   FROM github.User AS u \
                   JOIN local.issues AS i ON u.id = i.author_id";
        let out = executor::block_on(db.query(TextQueryInput::sql(sql))).unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("title"),
            Some(&Value::String("federation".to_string()))
        );
    }

    #[test]
    fn cross_source_left_join_preserves_unmatched_left_row() {
        let github = Arc::new(MockSource::new(
            &["User"],
            rows(
                "User",
                vec![row(&[
                    ("id", Value::String("u1".to_string())),
                    ("login", Value::String("theduke".to_string())),
                ])],
            ),
        ));
        let local = Arc::new(MockSource::new(&["issues"], rows("issues", Vec::new())));
        let db = FederatedBackend::new(None);
        register(&db, "github", github, SourceCapabilities::default());
        register(&db, "local", local, SourceCapabilities::default());

        let sql = "SELECT u.login AS login \
                   FROM github.User AS u \
                   LEFT JOIN local.issues AS i ON u.id = i.author_id";
        let out = executor::block_on(db.query(TextQueryInput::sql(sql))).unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("login"),
            Some(&Value::String("theduke".to_string()))
        );
    }

    #[test]
    fn cross_source_aggregation_after_join_executes_locally() {
        let github = Arc::new(MockSource::new(
            &["User"],
            rows(
                "User",
                vec![row(&[("id", Value::String("u1".to_string()))])],
            ),
        ));
        let local = Arc::new(MockSource::new(
            &["issues"],
            rows(
                "issues",
                vec![
                    row(&[("author_id", Value::String("u1".to_string()))]),
                    row(&[("author_id", Value::String("u1".to_string()))]),
                ],
            ),
        ));
        let db = FederatedBackend::new(None);
        register(&db, "github", github, SourceCapabilities::default());
        register(&db, "local", local, SourceCapabilities::default());

        let sql = "SELECT COUNT(*) AS issue_count \
                   FROM github.User AS u \
                   JOIN local.issues AS i ON u.id = i.author_id";
        let out = executor::block_on(db.query(TextQueryInput::sql(sql))).unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(rows[0].get("issue_count"), Some(&Value::I64(2)));
    }

    #[test]
    fn predicate_pushdown_is_used_when_supported() {
        let source = Arc::new(MockSource::new(
            &["users"],
            rows(
                "users",
                vec![
                    row(&[("login", Value::String("theduke".to_string()))]),
                    row(&[("login", Value::String("other".to_string()))]),
                ],
            ),
        ));
        let db = FederatedBackend::new(Some("local".to_string()));
        register(
            &db,
            "local",
            source.clone(),
            SourceCapabilities {
                supports_filter_pushdown: true,
                ..SourceCapabilities::default()
            },
        );

        let out = executor::block_on(db.query(TextQueryInput::sql(
            "SELECT login FROM users WHERE login = 'theduke'",
        )))
        .unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(rows.len(), 1);
        assert!(source.last_scan().unwrap().predicate.is_some());
    }

    #[test]
    fn unsupported_predicate_pushdown_falls_back_to_local_filtering() {
        let source = Arc::new(MockSource::new(
            &["users"],
            rows(
                "users",
                vec![
                    row(&[("login", Value::String("theduke".to_string()))]),
                    row(&[("login", Value::String("other".to_string()))]),
                ],
            ),
        ));
        let db = FederatedBackend::new(Some("local".to_string()));
        register(&db, "local", source.clone(), SourceCapabilities::default());

        let out = executor::block_on(db.query(TextQueryInput::sql(
            "SELECT login FROM users WHERE login = 'theduke'",
        )))
        .unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(rows.len(), 1);
        assert!(source.last_scan().unwrap().predicate.is_none());
    }

    #[test]
    fn source_local_catalog_collision_is_rejected() {
        let db = FederatedBackend::new(None);
        let first = Arc::new(MockSource::new(&["users"], BTreeMap::new()));
        let second = Arc::new(MockSource::new(&["users"], BTreeMap::new()));
        register(&db, "local", first, SourceCapabilities::default());

        let err = executor::block_on(db.register_source(SourceRegistration {
            name: "local".to_string(),
            backend: second,
            namespace: SourceNamespace::PrefixCollections,
            capabilities: SourceCapabilities::default(),
        }))
        .unwrap_err();
        assert!(matches!(err, DbError::InvalidQuery(_)));
    }

    #[test]
    fn single_source_insert_routes_to_source() {
        let source = Arc::new(MockSource::new(&["users"], BTreeMap::new()));
        let db = FederatedBackend::new(None);
        register(
            &db,
            "local",
            source.clone(),
            SourceCapabilities {
                supports_writes: true,
                ..SourceCapabilities::default()
            },
        );

        let query = InsertQuery::new()
            .with_collection("local.users")
            .with_source(InsertSource::Objects(vec![row(&[(
                "id",
                Value::String("u1".to_string()),
            )])]));
        let out = executor::block_on(db.query(TextQueryInput::Ast(Query::Insert(query)))).unwrap();
        assert!(matches!(out, QueryResult::Insert(_)));
        assert_eq!(source.write_count(), 1);
    }

    #[test]
    fn multi_source_batch_is_rejected() {
        let db = FederatedBackend::new(None);
        register(
            &db,
            "left",
            Arc::new(MockSource::new(&["items"], BTreeMap::new())),
            SourceCapabilities {
                supports_writes: true,
                ..SourceCapabilities::default()
            },
        );
        register(
            &db,
            "right",
            Arc::new(MockSource::new(&["items"], BTreeMap::new())),
            SourceCapabilities {
                supports_writes: true,
                ..SourceCapabilities::default()
            },
        );

        let batch = Batch::new()
            .with_op(BatchOperation::Upsert {
                collection: "left.items".to_string(),
                id: "1".to_string(),
                object: Object::new(),
            })
            .with_op(BatchOperation::Upsert {
                collection: "right.items".to_string(),
                id: "2".to_string(),
                object: Object::new(),
            });
        let err = executor::block_on(db.execute_batch(batch)).unwrap_err();
        assert!(matches!(err, DbError::InvalidQuery(_)));
    }

    #[test]
    fn write_to_read_only_source_is_rejected() {
        let source = Arc::new(MockSource::read_only(&["users"], BTreeMap::new()));
        let db = FederatedBackend::new(None);
        register(&db, "github", source, SourceCapabilities::default());

        let query = InsertQuery::new()
            .with_collection("github.users")
            .with_source(InsertSource::Objects(vec![Object::new()]));
        let err =
            executor::block_on(db.query(TextQueryInput::Ast(Query::Insert(query)))).unwrap_err();
        assert!(matches!(err, DbError::InvalidQuery(_)));
    }

    #[test]
    fn explain_contains_backend_tags() {
        let source = Arc::new(MockSource::new(&["users"], BTreeMap::new()));
        let db = FederatedBackend::new(Some("local".to_string()));
        register(&db, "local", source, SourceCapabilities::default());

        let explain =
            executor::block_on(db.explain(TextQueryInput::sql("SELECT * FROM users"))).unwrap();
        let crate::PhysicalPlan::Source(crate::PhysicalSource::Scan { source }) = explain.physical
        else {
            panic!("expected scan source");
        };
        assert_eq!(source.backend_tag.as_deref(), Some("local"));
    }

    #[test]
    fn projection_and_limit_remain_local_when_not_pushed() {
        let source = Arc::new(MockSource::new(
            &["users"],
            rows(
                "users",
                vec![
                    row(&[
                        ("id", Value::String("u1".to_string())),
                        ("login", Value::String("one".to_string())),
                    ]),
                    row(&[
                        ("id", Value::String("u2".to_string())),
                        ("login", Value::String("two".to_string())),
                    ]),
                ],
            ),
        ));
        let db = FederatedBackend::new(Some("local".to_string()));
        register(&db, "local", source.clone(), SourceCapabilities::default());

        let out = executor::block_on(db.query(TextQueryInput::sql(
            "SELECT login FROM users ORDER BY login ASC LIMIT 1",
        )))
        .unwrap();
        let QueryResult::Select(rows) = out else {
            panic!("expected select result");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 1);
        assert_eq!(
            rows[0].get("login"),
            Some(&Value::String("one".to_string()))
        );
        let scan = source.last_scan().unwrap();
        assert!(scan.projection.is_empty());
        assert!(scan.limit.is_none());
        assert!(scan.order_by.is_empty());
    }
}

#[async_trait]
impl FederatedSource for BackendFederatedSource {
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        self.backend.catalog().await
    }

    async fn scan(&self, request: SourceScanRequest) -> std::result::Result<Vec<Object>, DbError> {
        let query = SelectQuery {
            collection: Some(request.collection),
            source_alias: None,
            joins: Vec::new(),
            predicate: request.predicate,
            projection: request.projection,
            distinct: false,
            group_by: request.group_by,
            having: request.having,
            order_by: request.order_by,
            offset: request.offset,
            limit: request.limit,
            field_format: semantic_data::query::FieldFormat::Plain,
        };
        match self
            .backend
            .query(TextQueryInput::Ast(Query::Select(query)))
            .await?
        {
            QueryResult::Select(rows) => Ok(rows),
            _ => Err(DbError::InvalidQuery(
                "federated source returned non-select result for scan".to_string(),
            )),
        }
    }

    async fn insert(
        &self,
        request: SourceInsertRequest,
    ) -> std::result::Result<InsertResult, DbError> {
        let mut query = request.query;
        query.collection = Some(request.collection);
        match self
            .backend
            .query(TextQueryInput::Ast(Query::Insert(query)))
            .await?
        {
            QueryResult::Insert(result) => Ok(result),
            _ => Err(DbError::InvalidQuery(
                "federated source returned non-insert result for insert".to_string(),
            )),
        }
    }

    async fn update(
        &self,
        request: SourceUpdateRequest,
    ) -> std::result::Result<UpdateResult, DbError> {
        let mut query = request.query;
        query.collection = Some(request.collection);
        match self
            .backend
            .query(TextQueryInput::Ast(Query::Update(query)))
            .await?
        {
            QueryResult::Update(result) => Ok(result),
            _ => Err(DbError::InvalidQuery(
                "federated source returned non-update result for update".to_string(),
            )),
        }
    }

    async fn delete(
        &self,
        request: SourceDeleteRequest,
    ) -> std::result::Result<DeleteResult, DbError> {
        let mut query = request.query;
        query.collection = Some(request.collection);
        match self
            .backend
            .query(TextQueryInput::Ast(Query::Delete(query)))
            .await?
        {
            QueryResult::Delete(result) => Ok(result),
            _ => Err(DbError::InvalidQuery(
                "federated source returned non-delete result for delete".to_string(),
            )),
        }
    }
}
