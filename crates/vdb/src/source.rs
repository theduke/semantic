use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use futures_util::StreamExt;
use semantic_data::{FromValue, IntoValue, Object, Value};
use semantic_db_core::CoreError;
use semantic_db_core::catalog::{Catalog, CollectionKind, IntegrityMode};
use semantic_db_core::{DEFAULT_EXECUTION_BATCH_SIZE, DynObject, SendableRecordBatchStream};
use semantic_db_core::{DbError, QuerySource, SourceScan};
use semantic_plugin::PluginBinding;
use semantic_rpc::interface::{InvocationArgument, InvocationOutput, StreamEvent};
use semantic_rpc_core::interface::InvocationError;

use crate::{DatabaseDescriptor, ScanPlan, ScanRequest, ScanSummary, VdbError, validate_entity};

/// Adapt a live plugin binding into a canonical federated entity source.
pub struct PluginSource {
    binding: PluginBinding,
    descriptor: Arc<DatabaseDescriptor>,
    overlay: Arc<Catalog>,
    name: String,
}

impl PluginSource {
    pub fn new(
        binding: PluginBinding,
        descriptor: Arc<DatabaseDescriptor>,
        overlay: Arc<Catalog>,
        name: impl Into<String>,
    ) -> Self {
        let overlay = if overlay
            .collections()
            .any(|(_, collection)| collection.kind != CollectionKind::Untyped)
        {
            overlay
        } else {
            let mut validation_overlay = (*overlay).clone();
            validation_overlay
                .upsert_collection(
                    "vdb_validation",
                    CollectionKind::Polymorphic,
                    IntegrityMode::Permissive,
                )
                .expect("empty polymorphic collection is valid");
            Arc::new(validation_overlay)
        };
        Self {
            binding,
            descriptor,
            overlay,
            name: name.into(),
        }
    }

    fn error(&self, error: VdbError) -> DbError {
        DbError::InvalidQuery(format!("{}: {}: {}", self.name, error.code, error.message))
    }

    fn invocation_error(&self, error: InvocationError) -> DbError {
        if is_unavailable(&error.code) {
            return DbError::InvalidQuery(format!(
                "virtual database '{}' is unavailable: {}",
                self.name, error.message
            ));
        }
        if let Some(data) = error.data
            && let Ok(error) = VdbError::from_value(data)
        {
            return self.error(error);
        }
        self.error(VdbError {
            code: error.code,
            message: error.message,
        })
    }

    fn protocol_error(&self, message: impl Into<String>) -> DbError {
        self.error(VdbError {
            code: "protocol_violation".into(),
            message: message.into(),
        })
    }
}

#[async_trait]
impl QuerySource for PluginSource {
    async fn negotiate(
        &self,
        _collection: &str,
        request: &ScanRequest,
    ) -> Result<ScanPlan, DbError> {
        let output = self
            .binding
            .invoke(
                "negotiate",
                vec![InvocationArgument::Value(request.clone().into_value())],
            )
            .await
            .map_err(|error| self.invocation_error(error))?;
        let InvocationOutput::Values(mut values) = output else {
            return Err(self.protocol_error("negotiate must return one value"));
        };
        if values.len() != 1 {
            return Err(self.protocol_error("negotiate must return one value"));
        }
        let plan = ScanPlan::from_value(values.remove(0))
            .map_err(|error| self.protocol_error(error.to_string()))?;
        if let ScanPlan::Accepted { plan } = &plan {
            plan.validate(request).map_err(|error| self.error(error))?;
        }
        Ok(plan)
    }

    fn scan(self: Arc<Self>, scan: SourceScan) -> SendableRecordBatchStream {
        Box::pin(async_stream::try_stream! {
            scan.plan.validate(&scan.request).map_err(|error| CoreError::new(self.error(error).to_string()))?;
            let bindings = Object::from_iter(scan.bindings);
            let output = self.binding.invoke("scan", vec![
                InvocationArgument::Value(scan.request.into_value()),
                InvocationArgument::Value(scan.plan.into_value()),
                InvocationArgument::Value(bindings.into_value()),
            ]).await.map_err(|error| CoreError::new(self.invocation_error(error).to_string()))?;
            let InvocationOutput::Stream(mut stream) = output else { Err(CoreError::new(self.protocol_error("scan must return a stream").to_string()))?; unreachable!() };
            let mut seen = HashSet::new();
            let mut rows = 0u64;
            let mut batch = Vec::with_capacity(DEFAULT_EXECUTION_BATCH_SIZE);
            let mut ended = false;
            while let Some(event) = stream.next().await {
                match event.map_err(|error| CoreError::new(self.invocation_error(error).to_string()))? {
                    StreamEvent::Item(Value::Object(entity)) => {
                        validate_entity(&entity, &self.descriptor, &self.overlay).map_err(|error| CoreError::new(self.error(error).to_string()))?;
                        let id = entity.get("id").and_then(Value::as_str).expect("validated id");
                        if !seen.insert(id.to_owned()) { Err(CoreError::new(self.protocol_error(format!("duplicate entity id '{id}'")).to_string()))?; }
                        rows = rows.checked_add(1).ok_or_else(|| CoreError::new(self.protocol_error("row count overflow").to_string()))?;
                        batch.push(Box::new(entity) as DynObject);
                        if batch.len() == DEFAULT_EXECUTION_BATCH_SIZE { yield std::mem::replace(&mut batch, Vec::with_capacity(DEFAULT_EXECUTION_BATCH_SIZE)); }
                    }
                    StreamEvent::Item(_) => Err(CoreError::new(self.error(VdbError { code: "invalid_entity".into(), message: "row must be an object".into() }).to_string()))?,
                    StreamEvent::End(Some(value)) => {
                        let summary = ScanSummary::from_value(value).map_err(|error| CoreError::new(self.protocol_error(error.to_string()).to_string()))?;
                        if summary.rows != rows { Err(CoreError::new(self.protocol_error("scan summary row count does not match stream").to_string()))?; }
                        ended = true;
                        break;
                    }
                    StreamEvent::End(None) => Err(CoreError::new(self.protocol_error("missing scan summary").to_string()))?,
                }
            }
            if !ended { Err(CoreError::new(self.protocol_error("missing scan stream end").to_string()))?; }
            if !batch.is_empty() { yield batch; }
        })
    }
}

pub(crate) fn is_unavailable(code: &str) -> bool {
    matches!(
        code,
        "unavailable"
            | "plugin_changed"
            | "provider_unavailable"
            | "connection_lost"
            | "session_closed"
            | "transport_error"
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, future::Future, pin::Pin};

    use semantic_data::schema::{
        AttributeRef, AttributeType, ClassAttribute, ClassType, Constraint, LengthSpec, Meta,
        StringType, Type, TypeKind,
    };
    use semantic_jobs::{
        JobId, JobListPage, JobListQuery, JobRecord, JobStore, JobStoreError, JobsBuilder,
        ScopeJobs,
    };
    use semantic_plugin::{
        Plugin, PluginActivation, PluginError, PluginInstanceContext, PluginManifest,
        PluginProvider, PluginRegistry, ScopePlugins, portable_export,
    };
    use semantic_rpc::interface::{
        InterfaceImplementation, InvocationContext, OwnedValueStream, ValidatedInvocation,
    };
    use semantic_rpc_core::interface::ImplementationDescriptor;

    use super::*;
    use crate::{
        AcceptedScan, CancellationToken, DatabaseSchema, EntityStream, VirtualDatabase,
        VirtualDatabasePlugin, implementation_descriptor,
    };

    struct EmptyStore;

    #[async_trait]
    impl JobStore for EmptyStore {
        async fn initialize(&self) -> Result<(), JobStoreError> {
            Ok(())
        }
        async fn get(&self, _: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
            Ok(None)
        }
        async fn put(&self, _: &JobRecord) -> Result<(), JobStoreError> {
            Ok(())
        }
        async fn list(&self, _: JobListQuery) -> Result<JobListPage, JobStoreError> {
            Ok(JobListPage {
                records: vec![],
                next_cursor: None,
            })
        }
        async fn count(&self) -> Result<u64, JobStoreError> {
            Ok(0)
        }
        async fn delete_ids(&self, _: &[JobId]) -> Result<u64, JobStoreError> {
            Ok(0)
        }
    }

    fn schema(required: bool) -> DatabaseSchema {
        DatabaseSchema {
            attributes: vec![AttributeType {
                id: "tiny:name".into(),
                name: "name".into(),
                ty: Type::new(TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                })),
                constraints: vec![],
                meta: Meta::default(),
            }],
            classes: vec![ClassType {
                id: "tiny:Item".into(),
                name: "Item".into(),
                inherits: None,
                extends: vec![],
                strict_schema: false,
                creatable_in_ui: None,
                include_in_ui_listings: None,
                attributes: BTreeMap::from([(
                    "name".into(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: "tiny:name".into(),
                        },
                        required,
                        ui_order: None,
                        computed: None,
                        default: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )]),
                constraints: vec![],
                meta: Meta::default(),
            }],
            ..DatabaseSchema::default()
        }
    }

    fn descriptor(allow_untyped: bool) -> DatabaseDescriptor {
        DatabaseDescriptor {
            title: "Tiny".into(),
            description: None,
            schema: schema(false),
            schema_revision: "1".into(),
            allow_untyped,
        }
    }

    fn catalog(descriptor: &DatabaseDescriptor) -> Arc<Catalog> {
        let mut catalog = Catalog::new();
        for attribute in &descriptor.schema.attributes {
            catalog.upsert_attribute(attribute.clone());
        }
        for class in &descriptor.schema.classes {
            catalog
                .upsert_class_with_module(class.clone(), None)
                .unwrap();
        }
        catalog
            .upsert_collection(
                "tiny",
                CollectionKind::Polymorphic,
                IntegrityMode::Permissive,
            )
            .unwrap();
        Arc::new(catalog)
    }

    fn entity(id: &str, class: Option<&str>) -> Object {
        let mut entity = Object::new();
        entity.insert("id", Value::String(id.into()));
        if let Some(class) = class {
            entity.insert("type", Value::String(class.into()));
        }
        entity
    }

    fn manifest() -> PluginManifest {
        let mut catalog = Catalog::new();
        catalog.upsert_package(semantic_data::bundles::query::package());
        catalog.upsert_package(crate::package());
        PluginManifest {
            id: "tiny".into(),
            revision: "1".into(),
            title: "Tiny".into(),
            exports: vec![implementation_descriptor(&catalog, "database").unwrap()],
            configuration_schema: None,
            source_bindings: Default::default(),
        }
    }

    async fn runtime(plugin: impl Plugin) -> (ScopePlugins, ScopeJobs, PluginBinding) {
        let manifest = plugin.manifest().clone();
        let mut registry = PluginRegistry::new();
        registry.register(plugin).unwrap();
        let jobs = ScopeJobs::open(
            Arc::new(EmptyStore),
            JobsBuilder::new().build(),
            Default::default(),
        )
        .await
        .unwrap();
        let runtime = ScopePlugins::new("test", registry, jobs.clone());
        runtime
            .activate(PluginActivation {
                id: "tiny".into(),
                revision: "1".into(),
                provider: PluginProvider::Rust { key: manifest.id },
                enabled: true,
                generation: 1,
                configuration: Value::Null,
                configuration_schema: None,
                priority: None,
                source_bindings: Default::default(),
                exports: manifest.exports.into_iter().map(portable_export).collect(),
            })
            .await
            .unwrap();
        let binding = runtime.bindings().await.remove(0);
        (runtime, jobs, binding)
    }

    #[derive(Clone)]
    struct TinyVdb {
        descriptor: DatabaseDescriptor,
        rows: Vec<Object>,
        error: Option<VdbError>,
    }

    #[async_trait]
    impl VirtualDatabase for TinyVdb {
        async fn describe(&self) -> Result<DatabaseDescriptor, VdbError> {
            Ok(self.descriptor.clone())
        }
        fn schema_revision(&self) -> String {
            "1".into()
        }
        fn scan(
            &self,
            _: ScanRequest,
            _: AcceptedScan,
            _: Object,
            _: CancellationToken,
        ) -> EntityStream {
            if let Some(error) = &self.error {
                return Box::pin(futures_util::stream::iter([Err(error.clone())]));
            }
            Box::pin(futures_util::stream::iter(
                self.rows.clone().into_iter().map(Ok),
            ))
        }
    }

    async fn source(
        rows: Vec<Object>,
        allow_untyped: bool,
        error: Option<VdbError>,
    ) -> (ScopePlugins, ScopeJobs, Arc<PluginSource>) {
        let descriptor = descriptor(allow_untyped);
        let tiny = TinyVdb {
            descriptor: descriptor.clone(),
            rows,
            error,
        };
        let plugin = VirtualDatabasePlugin::new(manifest(), move |_| {
            let tiny = tiny.clone();
            async move { Ok(tiny) }
        });
        let (runtime, jobs, binding) = runtime(plugin).await;
        (
            runtime,
            jobs,
            Arc::new(PluginSource::new(
                binding,
                Arc::new(descriptor.clone()),
                catalog(&descriptor),
                "tiny",
            )),
        )
    }

    async fn collect(source: Arc<PluginSource>) -> Result<Vec<Object>, CoreError> {
        let request = ScanRequest::default();
        let ScanPlan::Accepted { plan } = source.negotiate("tiny", &request).await.unwrap() else {
            panic!("expected acceptance")
        };
        let mut stream = source.scan(SourceScan {
            collection: "tiny".into(),
            request,
            plan,
            bindings: Default::default(),
        });
        let mut rows = vec![];
        while let Some(batch) = stream.next().await {
            rows.extend(batch?.into_iter().map(|row| row.to_object()));
        }
        Ok(rows)
    }

    #[tokio::test]
    async fn real_binding_streams_canonical_entities_in_batches() {
        let rows = (0..DEFAULT_EXECUTION_BATCH_SIZE + 3)
            .map(|id| entity(&id.to_string(), Some("tiny:Item")))
            .collect::<Vec<_>>();
        let (runtime, jobs, source) = source(rows.clone(), false, None).await;
        assert_eq!(collect(source).await.unwrap(), rows);
        runtime.shutdown().await.unwrap();
        jobs.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_entities_and_duplicate_ids_fail_the_scan() {
        for (rows, allow_untyped, message) in [
            (vec![Object::new()], true, "id must be"),
            (vec![entity("", None)], true, "id must be"),
            (
                vec![entity("one", Some("unknown:Class"))],
                true,
                "exposed class",
            ),
            (vec![entity("one", None)], false, "untyped"),
            (
                vec![entity("one", None), entity("one", None)],
                true,
                "duplicate entity id",
            ),
        ] {
            let (runtime, jobs, source) = source(rows, allow_untyped, None).await;
            let error = collect(source).await.unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
            runtime.shutdown().await.unwrap();
            jobs.shutdown().await.unwrap();
        }
    }

    #[test]
    fn validates_class_membership_value_types_required_fields_and_constraints() {
        let mut descriptor = descriptor(false);
        descriptor.schema = schema(true);
        descriptor.schema.attributes[0]
            .constraints
            .push(Constraint::Length(LengthSpec::Range {
                min: Some(2),
                max: None,
            }));
        let catalog = catalog(&descriptor);
        let mut row = entity("one", Some("tiny:Item"));
        assert!(
            validate_entity(&row, &descriptor, &catalog)
                .unwrap_err()
                .message
                .contains("required")
        );
        row.insert("tiny:name", Value::Bool(false));
        assert!(validate_entity(&row, &descriptor, &catalog).is_err());
        row.insert("tiny:name", Value::String("a".into()));
        assert!(
            validate_entity(&row, &descriptor, &catalog)
                .unwrap_err()
                .message
                .contains("length")
        );
        row.insert("tiny:name", Value::String("valid".into()));
        validate_entity(&row, &descriptor, &catalog).unwrap();
        row.insert("other:field", Value::Null);
        assert!(
            validate_entity(&row, &descriptor, &catalog)
                .unwrap_err()
                .message
                .contains("does not belong")
        );
        row.remove("other:field");
        row.remove("tiny:name");
        row.insert("name", Value::String("valid".into()));
        assert!(
            validate_entity(&row, &descriptor, &catalog).is_err(),
            "plain attribute aliases are not canonical rows"
        );
    }

    #[tokio::test]
    async fn plugin_errors_and_unavailable_bindings_have_collection_diagnostics() {
        for (code, message) in [
            ("rate_limited", "tiny: rate_limited: retry later"),
            ("connection_lost", "virtual database 'tiny' is unavailable"),
        ] {
            let (runtime, jobs, source) = source(
                vec![],
                true,
                Some(VdbError {
                    code: code.into(),
                    message: "retry later".into(),
                }),
            )
            .await;
            assert!(
                collect(source)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains(message)
            );
            runtime.shutdown().await.unwrap();
            jobs.shutdown().await.unwrap();
        }
        let (runtime, jobs, source) = source(vec![], true, None).await;
        runtime.shutdown().await.unwrap();
        assert!(
            source
                .negotiate("tiny", &ScanRequest::default())
                .await
                .unwrap_err()
                .to_string()
                .contains("is unavailable")
        );
        jobs.shutdown().await.unwrap();
    }

    struct BrokenPlugin {
        manifest: PluginManifest,
    }
    struct BrokenImplementation {
        exports: Vec<ImplementationDescriptor>,
    }

    impl Plugin for BrokenPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }
        fn create<'a>(
            &'a self,
            _: PluginInstanceContext,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Arc<dyn InterfaceImplementation>, PluginError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                Ok(Arc::new(BrokenImplementation {
                    exports: self.manifest.exports.clone(),
                }) as Arc<dyn InterfaceImplementation>)
            })
        }
    }

    impl InterfaceImplementation for BrokenImplementation {
        fn descriptors(&self) -> &[ImplementationDescriptor] {
            &self.exports
        }
        fn invoke<'a>(
            &'a self,
            call: ValidatedInvocation,
            _: InvocationContext,
        ) -> Pin<Box<dyn Future<Output = Result<InvocationOutput, InvocationError>> + Send + 'a>>
        {
            Box::pin(async move {
                if call.method == "scan" {
                    Ok(InvocationOutput::Stream(OwnedValueStream::new(
                        futures_util::stream::iter([Ok(StreamEvent::Item(Value::Object(entity(
                            "one", None,
                        ))))]),
                    )))
                } else {
                    let plan = TinyVdb {
                        descriptor: descriptor(true),
                        rows: vec![],
                        error: None,
                    }
                    .negotiate(&ScanRequest::default())
                    .await
                    .unwrap();
                    Ok(InvocationOutput::Values(vec![plan.into_value()]))
                }
            })
        }
    }

    #[tokio::test]
    async fn missing_stream_end_is_an_error() {
        let (runtime, jobs, binding) = runtime(BrokenPlugin {
            manifest: manifest(),
        })
        .await;
        let descriptor = descriptor(true);
        let source = Arc::new(PluginSource::new(
            binding,
            Arc::new(descriptor.clone()),
            catalog(&descriptor),
            "tiny",
        ));
        assert!(
            collect(source)
                .await
                .unwrap_err()
                .to_string()
                .contains("terminal event")
        );
        runtime.shutdown().await.unwrap();
        jobs.shutdown().await.unwrap();
    }
}
