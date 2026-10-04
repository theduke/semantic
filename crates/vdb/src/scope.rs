use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use semantic_data::{FromValue, query::DdlBatch};
use semantic_db_core::catalog::Catalog;
use semantic_db_core::{VirtualSource, virtual_overlay};
use semantic_plugin::PluginBinding;
use semantic_rpc::interface::InvocationOutput;
use tokio::{
    sync::OnceCell,
    time::{Duration, Instant},
};

use crate::{
    DatabaseDescriptor, DatabaseSchema, INTERFACE_NAME, MODULE_NAME, PACKAGE_NAME, PluginSource,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VdbStatus {
    Available,
    Unavailable { reason: String },
}

#[derive(Clone, Debug)]
pub struct VdbEntry {
    pub name: String,
    pub plugin_id: String,
    pub export: String,
    pub generation: u64,
    pub status: VdbStatus,
    pub descriptor: Option<DatabaseDescriptor>,
}

pub struct PreparedVdb {
    pub descriptor: Arc<DatabaseDescriptor>,
    pub schema: Arc<DdlBatch>,
    pub overlay: Arc<Catalog>,
}

#[derive(Clone, Default)]
pub struct VdbSet {
    entries: Vec<VdbEntry>,
    pub sources: BTreeMap<String, VirtualSource>,
}

impl VdbSet {
    pub fn get_available(&self, name: &str) -> Option<&VirtualSource> {
        self.sources.get(name)
    }
    pub fn entry(&self, name: &str) -> Option<&VdbEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
    pub fn entries(&self) -> &[VdbEntry] {
        &self.entries
    }
    pub fn schema(&self, name: &str) -> Option<&DatabaseSchema> {
        self.entry(name)?
            .descriptor
            .as_ref()
            .map(|descriptor| &descriptor.schema)
    }
}

type VdbKey = (String, String, u64);
const DESCRIBE_RETRY_BACKOFF: Duration = Duration::from_secs(5);
struct Description {
    result: Result<Arc<DatabaseDescriptor>, String>,
    retry_at: Option<Instant>,
}

struct CachedVdb {
    name: String,
    description: Arc<OnceCell<Description>>,
    prepared: Option<(Arc<Catalog>, Result<Arc<PreparedVdb>, String>)>,
}

impl CachedVdb {
    fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            description: Arc::new(OnceCell::new()),
            prepared: None,
        }
    }
}

/// Runtime schema cache for one scope. Definitions never enter persisted state.
#[derive(Default)]
pub struct ScopeVdbs {
    cache: Mutex<BTreeMap<VdbKey, CachedVdb>>,
}

impl ScopeVdbs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Describe all active exports concurrently for a complete listing.
    /// Descriptors survive local catalog changes. Invocation failures retry after
    /// five seconds; malformed descriptor responses last for the generation.
    pub async fn snapshot(
        &self,
        bindings: Vec<PluginBinding>,
        local_catalog: Arc<Catalog>,
    ) -> VdbSet {
        self.snapshot_selected(bindings, local_catalog, None).await
    }

    /// Prepare only requested names. Naming and conflicts still consider every
    /// active binding, and unrelated cached exports remain available.
    pub async fn snapshot_named(
        &self,
        bindings: Vec<PluginBinding>,
        local_catalog: Arc<Catalog>,
        names: &BTreeSet<String>,
    ) -> VdbSet {
        self.snapshot_selected(bindings, local_catalog, Some(names))
            .await
    }

    async fn snapshot_selected(
        &self,
        bindings: Vec<PluginBinding>,
        local_catalog: Arc<Catalog>,
        selected: Option<&BTreeSet<String>>,
    ) -> VdbSet {
        let bindings = bindings
            .into_iter()
            .filter(|binding| {
                let interface = &binding.descriptor().interface;
                interface.package == PACKAGE_NAME
                    && interface.module == MODULE_NAME
                    && interface.name == INTERFACE_NAME
                    && interface.contract.is_none()
            })
            .collect::<Vec<_>>();
        let mut exports_per_plugin = BTreeMap::<String, usize>::new();
        for binding in &bindings {
            *exports_per_plugin
                .entry(binding.plugin_id().into())
                .or_default() += 1;
        }
        let named = bindings
            .into_iter()
            .map(|binding| {
                let name = if exports_per_plugin[binding.plugin_id()] > 1 {
                    format!("{}.{}", binding.plugin_id(), binding.descriptor().export)
                } else {
                    binding.plugin_id().into()
                };
                (name, binding)
            })
            .collect::<Vec<_>>();
        let mut names = BTreeMap::<String, usize>::new();
        let active = named
            .iter()
            .map(|(name, binding)| {
                *names.entry(name.clone()).or_default() += 1;
                key(binding)
            })
            .collect::<BTreeSet<_>>();
        self.cache
            .lock()
            .expect("VDB cache poisoned")
            .retain(|key, _| active.contains(key));
        let prepared = futures_util::future::join_all(
            named
                .into_iter()
                .filter(|(name, _)| selected.is_none_or(|selected| selected.contains(name)))
                .map(|(name, binding)| {
                    let local_catalog = local_catalog.clone();
                    let names = &names;
                    async move {
                        let mut entry = VdbEntry {
                            name: name.clone(),
                            plugin_id: binding.plugin_id().into(),
                            export: binding.descriptor().export.clone(),
                            generation: binding.generation(),
                            status: VdbStatus::Available,
                            descriptor: None,
                        };
                        let prepared = if local_catalog.collection_by_name(&name).is_some() {
                            Err("conflicts with local collection".into())
                        } else if names[&name] > 1 {
                            Err("conflicts with another virtual database collection name".into())
                        } else {
                            self.prepare(&binding, &name, local_catalog).await
                        };
                        let source = match prepared {
                            Ok(prepared) => {
                                entry.descriptor = Some((*prepared.descriptor).clone());
                                Some(VirtualSource {
                                    source: Arc::new(PluginSource::new(
                                        binding,
                                        prepared.descriptor.clone(),
                                        prepared.overlay.clone(),
                                        name,
                                    )),
                                    schema: prepared.schema.clone(),
                                    schema_revision: prepared.descriptor.schema_revision.clone(),
                                })
                            }
                            Err(reason) => {
                                entry.status = VdbStatus::Unavailable { reason };
                                None
                            }
                        };
                        (entry, source)
                    }
                }),
        )
        .await;
        let mut set = VdbSet::default();
        for (entry, source) in prepared {
            if let Some(source) = source {
                set.sources.insert(entry.name.clone(), source);
            }
            set.entries.push(entry);
        }
        set.entries.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.plugin_id.cmp(&right.plugin_id))
                .then_with(|| left.export.cmp(&right.export))
        });
        set
    }

    async fn prepare(
        &self,
        binding: &PluginBinding,
        name: &str,
        local: Arc<Catalog>,
    ) -> Result<Arc<PreparedVdb>, String> {
        let description = {
            let mut cache = self.cache.lock().expect("VDB cache poisoned");
            let cached = cache
                .entry(key(binding))
                .or_insert_with(|| CachedVdb::new(name));
            if cached.name != name
                || cached.description.get().is_some_and(|description| {
                    description
                        .retry_at
                        .is_some_and(|deadline| Instant::now() >= deadline)
                })
            {
                *cached = CachedVdb::new(name);
            }
            cached.description.clone()
        };
        let descriptor = description
            .get_or_init(|| describe(binding))
            .await
            .result
            .as_ref()
            .map_err(Clone::clone)?
            .clone();
        {
            let cache = self.cache.lock().expect("VDB cache poisoned");
            if let Some(cached) = cache.get(&key(binding))
                && Arc::ptr_eq(&cached.description, &description)
                && let Some((catalog, prepared)) = &cached.prepared
                && Arc::ptr_eq(catalog, &local)
            {
                return prepared.clone();
            }
        }
        // Catalog identity only skips repeated pure validation. A changed or
        // freshly deserialized catalog never causes another describe call.
        let prepared = prepare(descriptor, name, &local);
        let mut cache = self.cache.lock().expect("VDB cache poisoned");
        if let Some(cached) = cache.get_mut(&key(binding))
            && Arc::ptr_eq(&cached.description, &description)
        {
            cached.prepared = Some((local, prepared.clone()));
        }
        prepared
    }

    /// Remove only the stale revision observed by the failed query. Newer
    /// descriptors and in-flight refreshes survive delayed invalidations.
    pub fn invalidate(&self, name: &str, observed_revision: &str) {
        self.cache
            .lock()
            .expect("VDB cache poisoned")
            .retain(|_, cached| {
                !(cached.name == name
                    && cached.description.get().is_some_and(|description| {
                        description
                            .result
                            .as_ref()
                            .is_ok_and(|descriptor| descriptor.schema_revision == observed_revision)
                    }))
            });
    }
}

fn key(binding: &PluginBinding) -> VdbKey {
    (
        binding.plugin_id().into(),
        binding.descriptor().export.clone(),
        binding.generation(),
    )
}

// Descriptions are independent of the local catalog. Invocation failures back
// off; malformed descriptor responses stay cached for the generation.
async fn describe(binding: &PluginBinding) -> Description {
    let output = match binding.invoke("describe", vec![]).await {
        Ok(output) => output,
        Err(error) => {
            return Description {
                retry_at: Some(Instant::now() + DESCRIBE_RETRY_BACKOFF),
                result: Err(format!("{}: {}", error.code, error.message)),
            };
        }
    };
    let result = (|| {
        let InvocationOutput::Values(mut values) = output else {
            return Err("describe must return one value".into());
        };
        if values.len() != 1 {
            return Err("describe must return one value".into());
        }
        DatabaseDescriptor::from_value(values.remove(0))
            .map(Arc::new)
            .map_err(|error| format!("invalid database descriptor: {error}"))
    })();
    Description {
        result,
        retry_at: None,
    }
}

fn prepare(
    descriptor: Arc<DatabaseDescriptor>,
    name: &str,
    local: &Catalog,
) -> Result<Arc<PreparedVdb>, String> {
    let schema = Arc::new(descriptor.schema.to_ddl_batch());
    let overlay = virtual_overlay(local, &[(name, schema.as_ref())])?;
    Ok(Arc::new(PreparedVdb {
        descriptor,
        schema,
        overlay: Arc::new(overlay),
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::test_support::EmptyStore;
    use async_trait::async_trait;
    use semantic_data::schema::{
        AnyType, AttributeRef, AttributeType, ClassAttribute, ClassType, Meta,
        RelationIndexingMode, RelationMode, RelationType, StringType, Type, TypeDef, TypeKind,
        Visibility,
    };
    use semantic_data::{Object, Value};
    use semantic_jobs::{JobsBuilder, ScopeJobs};
    use semantic_plugin::{
        Plugin, PluginActivation, PluginManifest, PluginProvider, PluginRegistry, ScopePlugins,
        portable_export,
    };
    use tokio::sync::{Notify, Semaphore};

    use super::*;
    use crate::{
        AcceptedScan, CancellationToken, EntityStream, ScanRequest, VdbError, VirtualDatabase,
        VirtualDatabasePlugin, implementation_descriptor,
    };
    use semantic_db_core::catalog::{CollectionKind, IntegrityMode};

    fn attribute(id: &str) -> AttributeType {
        AttributeType {
            id: id.into(),
            name: id.split(':').next_back().unwrap().into(),
            ty: Type::new(TypeKind::String(StringType {
                format: None,
                normalization: None,
            })),
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn class_attribute(id: &str) -> ClassAttribute {
        ClassAttribute {
            attribute: AttributeRef { id: id.into() },
            required: false,
            ui_order: None,
            computed: None,
            default: None,
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn descriptor() -> DatabaseDescriptor {
        DatabaseDescriptor {
            title: "Test VDB".into(),
            description: None,
            schema_revision: "1".into(),
            allow_untyped: true,
            schema: DatabaseSchema {
                attributes: vec![attribute("test:value")],
                classes: vec![ClassType {
                    id: "test:Item".into(),
                    name: "Item".into(),
                    inherits: None,
                    extends: vec![],
                    strict_schema: false,
                    creatable_in_ui: None,
                    include_in_ui_listings: None,
                    attributes: BTreeMap::from([
                        ("value".into(), class_attribute("test:value")),
                        ("title".into(), class_attribute("base:title")),
                    ]),
                    constraints: vec![],
                    meta: Meta::default(),
                }],
                ..DatabaseSchema::default()
            },
        }
    }

    fn local() -> Arc<Catalog> {
        let mut catalog = Catalog::new();
        catalog.upsert_attribute(attribute("base:title"));
        catalog
            .upsert_collection(
                "local",
                CollectionKind::Polymorphic,
                IntegrityMode::Permissive,
            )
            .unwrap();
        Arc::new(catalog)
    }

    #[derive(Clone)]
    struct TestDb {
        descriptor: Arc<Mutex<DatabaseDescriptor>>,
        calls: Arc<AtomicUsize>,
        fail_next: Arc<AtomicBool>,
        failure_code: Arc<Mutex<&'static str>>,
        pause_next: Arc<AtomicBool>,
        entered: Arc<Notify>,
        release: Arc<Semaphore>,
    }

    impl TestDb {
        fn new() -> Self {
            Self {
                descriptor: Arc::new(Mutex::new(descriptor())),
                calls: Arc::new(AtomicUsize::new(0)),
                fail_next: Arc::new(AtomicBool::new(false)),
                failure_code: Arc::new(Mutex::new("temporary_failure")),
                pause_next: Arc::new(AtomicBool::new(false)),
                entered: Arc::new(Notify::new()),
                release: Arc::new(Semaphore::new(0)),
            }
        }
    }

    #[async_trait]
    impl VirtualDatabase for TestDb {
        async fn describe(&self) -> Result<DatabaseDescriptor, VdbError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_next.swap(false, Ordering::SeqCst) {
                return Err(VdbError {
                    code: self.failure_code.lock().unwrap().to_string(),
                    message: "retry describe".into(),
                });
            }
            if self.pause_next.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.acquire().await.unwrap().forget();
            }
            Ok(self.descriptor.lock().unwrap().clone())
        }
        fn schema_revision(&self) -> String {
            self.descriptor.lock().unwrap().schema_revision.clone()
        }
        fn scan(
            &self,
            _: ScanRequest,
            _: AcceptedScan,
            _: Object,
            _: CancellationToken,
        ) -> EntityStream {
            Box::pin(futures_util::stream::empty())
        }
    }

    async fn runtime(
        database: TestDb,
        exports: &[&str],
    ) -> (Arc<ScopePlugins>, ScopeJobs, PluginActivation) {
        let mut catalog = Catalog::new();
        catalog.upsert_package(semantic_data::bundles::query::package());
        catalog.upsert_package(crate::package());
        // This definition exists solely in the plugin configuration layer.
        let configuration_schema = Some(semantic_data::plugin::PluginConfigurationSchema {
            ty: Type::new(TypeKind::Any(AnyType)),
            definitions: BTreeMap::from([(
                "plugin:Helper".into(),
                TypeDef {
                    name: "plugin:Helper".into(),
                    module: None,
                    params: vec![],
                    ty: Type::new(TypeKind::Any(AnyType)),
                    visibility: Visibility::Private,
                    meta: Meta::default(),
                },
            )]),
        });
        let manifest = PluginManifest {
            id: "test-plugin".into(),
            revision: "1".into(),
            title: "Test plugin".into(),
            exports: exports
                .iter()
                .map(|name| implementation_descriptor(&catalog, name).unwrap())
                .collect(),
            configuration_schema: configuration_schema.clone(),
            source_bindings: BTreeMap::new(),
        };
        let activation = PluginActivation {
            id: "fx".into(),
            revision: "1".into(),
            provider: PluginProvider::Rust {
                key: manifest.id.clone(),
            },
            enabled: true,
            generation: 1,
            configuration: Value::Null,
            configuration_schema,
            priority: None,
            exports: manifest
                .exports
                .iter()
                .cloned()
                .map(portable_export)
                .collect(),
            source_bindings: BTreeMap::new(),
        };
        let plugin = VirtualDatabasePlugin::new(manifest, move |_| {
            let database = database.clone();
            async move { Ok(database) }
        });
        assert_eq!(plugin.manifest().id, "test-plugin");
        let mut registry = PluginRegistry::new();
        registry.register(plugin).unwrap();
        let jobs = ScopeJobs::open(
            Arc::new(EmptyStore),
            JobsBuilder::new().build(),
            Default::default(),
        )
        .await
        .unwrap();
        let runtime = Arc::new(ScopePlugins::new("scope", registry, jobs.clone()));
        runtime.activate(activation.clone()).await.unwrap();
        (runtime, jobs, activation)
    }

    async fn close(runtime: &ScopePlugins, jobs: ScopeJobs) {
        runtime.shutdown().await.unwrap();
        jobs.shutdown().await.unwrap();
    }

    fn reason(set: &VdbSet, name: &str) -> String {
        let VdbStatus::Unavailable { reason } = &set.entry(name).unwrap().status else {
            panic!("expected unavailable")
        };
        reason.clone()
    }

    #[tokio::test]
    async fn names_single_and_multiple_exports() {
        for (exports, expected) in [
            (vec!["database"], vec!["fx"]),
            (vec!["one", "two"], vec!["fx.one", "fx.two"]),
        ] {
            let (runtime, jobs, _) = runtime(TestDb::new(), &exports).await;
            let set = ScopeVdbs::new()
                .snapshot(runtime.bindings().await, local())
                .await;
            assert_eq!(
                set.entries()
                    .iter()
                    .map(|entry| entry.name.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(
                set.entries()
                    .iter()
                    .all(|entry| entry.status == VdbStatus::Available)
            );
            assert_eq!(set.sources.len(), exports.len());
            close(&runtime, jobs).await;
        }
    }

    #[tokio::test]
    async fn conflicts_never_shadow_local_collections() {
        let database = TestDb::new();
        let (runtime, jobs, _) = runtime(database.clone(), &["database"]).await;
        let mut local = (*local()).clone();
        local
            .upsert_collection("fx", CollectionKind::Untyped, IntegrityMode::Permissive)
            .unwrap();
        let set = ScopeVdbs::new()
            .snapshot(runtime.bindings().await, Arc::new(local))
            .await;
        assert!(reason(&set, "fx").contains("conflicts with local collection"));
        assert!(set.get_available("fx").is_none());
        assert_eq!(database.calls.load(Ordering::SeqCst), 0);
        close(&runtime, jobs).await;
    }

    #[tokio::test]
    async fn runtime_schema_reuses_local_attributes_without_leaking_plugin_types() {
        let (runtime, jobs, _) = runtime(TestDb::new(), &["database"]).await;
        let scope = ScopeVdbs::new();
        let local = local();
        let set = scope
            .snapshot(runtime.bindings().await, local.clone())
            .await;
        assert_eq!(set.schema("fx").unwrap().classes[0].id, "test:Item");
        assert!(local.class_id("test:Item").is_none());
        let prepared = scope
            .cache
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .1
            .as_ref()
            .unwrap()
            .clone();
        assert!(prepared.overlay.class_id("test:Item").is_some());
        assert!(prepared.overlay.attribute_by_id("base:title").is_some());
        assert!(prepared.overlay.type_def_by_name("plugin:Helper").is_none());
        close(&runtime, jobs).await;
    }

    #[tokio::test]
    async fn invalid_schemas_report_definition_ids_and_are_cached() {
        for duplicate in [false, true] {
            let database = TestDb::new();
            if duplicate {
                database
                    .descriptor
                    .lock()
                    .unwrap()
                    .schema
                    .attributes
                    .push(attribute("base:title"));
            } else {
                database.descriptor.lock().unwrap().schema.classes[0]
                    .attributes
                    .insert("missing".into(), class_attribute("missing:attribute"));
            }
            let (runtime, jobs, _) = runtime(database.clone(), &["database"]).await;
            let scope = ScopeVdbs::new();
            let local = local();
            let set = scope
                .snapshot(runtime.bindings().await, local.clone())
                .await;
            assert!(reason(&set, "fx").contains(if duplicate {
                "base:title"
            } else {
                "missing:attribute"
            }));
            scope.snapshot(runtime.bindings().await, local).await;
            assert_eq!(database.calls.load(Ordering::SeqCst), 1);
            close(&runtime, jobs).await;
        }
    }

    #[tokio::test]
    async fn concurrent_snapshots_describe_once() {
        let database = TestDb::new();
        let (runtime, jobs, _) = runtime(database.clone(), &["database"]).await;
        let scope = Arc::new(ScopeVdbs::new());
        let local = local();
        let bindings = runtime.bindings().await;
        let mut snapshots = vec![];
        for _ in 0..8 {
            let scope = scope.clone();
            let local = local.clone();
            let bindings = bindings.clone();
            snapshots.push(tokio::spawn(async move {
                scope.snapshot(bindings, local).await
            }));
        }
        for snapshot in snapshots {
            assert!(snapshot.await.unwrap().get_available("fx").is_some());
        }
        assert_eq!(database.calls.load(Ordering::SeqCst), 1);
        close(&runtime, jobs).await;
    }

    #[tokio::test]
    async fn named_snapshots_skip_unrelated_describes_and_keep_global_export_names() {
        let database = TestDb::new();
        let (runtime, jobs, _) = runtime(database.clone(), &["one", "two"]).await;
        let scope = Arc::new(ScopeVdbs::new());
        let local = local();
        let bindings = runtime.bindings().await;
        let names = BTreeSet::from(["fx.one".into()]);
        let set = scope
            .snapshot_named(bindings.clone(), local.clone(), &names)
            .await;
        assert!(set.get_available("fx.one").is_some());
        assert_eq!(set.entries().len(), 1);
        assert_eq!(database.calls.load(Ordering::SeqCst), 1);

        database.pause_next.store(true, Ordering::SeqCst);
        let listing = {
            let scope = scope.clone();
            let local = local.clone();
            let bindings = bindings.clone();
            tokio::spawn(async move { scope.snapshot(bindings, local).await })
        };
        database.entered.notified().await;
        let selected = tokio::time::timeout(
            Duration::from_secs(1),
            scope.snapshot_named(bindings, local, &names),
        )
        .await
        .unwrap();
        assert!(selected.get_available("fx.one").is_some());
        assert!(
            !listing.is_finished(),
            "unrelated describe must remain pending"
        );
        assert_eq!(database.calls.load(Ordering::SeqCst), 2);
        database.release.add_permits(1);
        assert_eq!(listing.await.unwrap().sources.len(), 2);
        close(&runtime, jobs).await;
    }

    #[tokio::test]
    async fn listing_describes_exports_concurrently() {
        let database = TestDb::new();
        database.pause_next.store(true, Ordering::SeqCst);
        let (runtime, jobs, _) = runtime(database.clone(), &["one", "two"]).await;
        let listing = {
            let bindings = runtime.bindings().await;
            tokio::spawn(async move { ScopeVdbs::new().snapshot(bindings, local()).await })
        };
        database.entered.notified().await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while database.calls.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!listing.is_finished());
        database.release.add_permits(1);
        assert_eq!(listing.await.unwrap().sources.len(), 2);
        close(&runtime, jobs).await;
    }

    #[tokio::test(start_paused = true)]
    async fn cached_invalid_descriptor_waits_for_generation_replacement() {
        let database = TestDb::new();
        database.descriptor.lock().unwrap().schema.classes[0]
            .attributes
            .insert("missing".into(), class_attribute("missing:attribute"));
        let (runtime, jobs, mut activation) = runtime(database.clone(), &["database"]).await;
        let scope = ScopeVdbs::new();
        let local = local();
        assert!(
            reason(
                &scope
                    .snapshot(runtime.bindings().await, local.clone())
                    .await,
                "fx"
            )
            .contains("missing:attribute")
        );
        *database.descriptor.lock().unwrap() = descriptor();
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(
            scope
                .snapshot(runtime.bindings().await, local.clone())
                .await
                .get_available("fx")
                .is_none()
        );
        assert_eq!(database.calls.load(Ordering::SeqCst), 1);
        activation.generation = 2;
        runtime.activate(activation).await.unwrap();
        tokio::task::yield_now().await;
        assert!(
            scope
                .snapshot(runtime.bindings().await, local)
                .await
                .get_available("fx")
                .is_some()
        );
        assert_eq!(database.calls.load(Ordering::SeqCst), 2);
        close(&runtime, jobs).await;
    }

    #[tokio::test(start_paused = true)]
    async fn invocation_failure_backs_off_then_recovers() {
        for code in [
            "temporary_failure",
            "cancelled",
            "network_error",
            "invalid_schema",
        ] {
            let database = TestDb::new();
            database.fail_next.store(true, Ordering::SeqCst);
            *database.failure_code.lock().unwrap() = code;
            let (runtime, jobs, _) = runtime(database.clone(), &["database"]).await;
            let scope = ScopeVdbs::new();
            let local = local();
            assert!(
                reason(
                    &scope
                        .snapshot(runtime.bindings().await, local.clone())
                        .await,
                    "fx"
                )
                .contains("retry describe")
            );
            let set = scope
                .snapshot(runtime.bindings().await, local.clone())
                .await;
            assert!(reason(&set, "fx").contains("retry describe"));
            assert_eq!(database.calls.load(Ordering::SeqCst), 1);
            tokio::time::advance(Duration::from_secs(4)).await;
            assert!(
                scope
                    .snapshot(runtime.bindings().await, local.clone())
                    .await
                    .get_available("fx")
                    .is_none()
            );
            assert_eq!(database.calls.load(Ordering::SeqCst), 1);
            tokio::time::advance(Duration::from_secs(1)).await;
            assert!(
                scope
                    .snapshot(runtime.bindings().await, local)
                    .await
                    .get_available("fx")
                    .is_some()
            );
            assert_eq!(database.calls.load(Ordering::SeqCst), 2);
            close(&runtime, jobs).await;
        }
    }

    #[tokio::test]
    async fn delayed_invalidations_preserve_inflight_and_newer_refresh() {
        let database = TestDb::new();
        let (runtime, jobs, _) = runtime(database.clone(), &["database"]).await;
        let scope = Arc::new(ScopeVdbs::new());
        let local = local();
        let bindings = runtime.bindings().await;
        assert_eq!(
            scope
                .snapshot(bindings.clone(), local.clone())
                .await
                .get_available("fx")
                .unwrap()
                .schema_revision,
            "1"
        );
        database.descriptor.lock().unwrap().schema_revision = "2".into();
        database.pause_next.store(true, Ordering::SeqCst);
        scope.invalidate("fx", "1");
        let refresh = {
            let scope = scope.clone();
            let local = local.clone();
            let bindings = bindings.clone();
            tokio::spawn(async move { scope.snapshot(bindings, local).await })
        };
        database.entered.notified().await;
        scope.invalidate("fx", "1");
        let concurrent = {
            let scope = scope.clone();
            let local = local.clone();
            let bindings = bindings.clone();
            tokio::spawn(async move { scope.snapshot(bindings, local).await })
        };
        database.release.add_permits(1);
        assert_eq!(
            refresh
                .await
                .unwrap()
                .get_available("fx")
                .unwrap()
                .schema_revision,
            "2"
        );
        assert_eq!(
            concurrent
                .await
                .unwrap()
                .get_available("fx")
                .unwrap()
                .schema_revision,
            "2"
        );
        scope.invalidate("fx", "1");
        scope.snapshot(bindings, local).await;
        assert_eq!(database.calls.load(Ordering::SeqCst), 2);
        close(&runtime, jobs).await;
    }

    #[tokio::test]
    async fn generation_change_and_disabling_drop_cached_schema() {
        let database = TestDb::new();
        let (runtime, jobs, mut activation) = runtime(database.clone(), &["database"]).await;
        let scope = ScopeVdbs::new();
        let local = local();
        scope
            .snapshot(runtime.bindings().await, local.clone())
            .await;
        activation.generation = 2;
        runtime.activate(activation.clone()).await.unwrap();
        // The existing runtime exposes bindings through try_lock; let the old
        // generation's termination observer release the entry after replacement.
        tokio::task::yield_now().await;
        let set = scope
            .snapshot(runtime.bindings().await, local.clone())
            .await;
        assert_eq!(set.entry("fx").unwrap().generation, 2);
        assert_eq!(database.calls.load(Ordering::SeqCst), 2);
        assert_eq!(scope.cache.lock().unwrap().len(), 1);
        activation.generation = 3;
        activation.enabled = false;
        runtime.activate(activation).await.unwrap();
        assert!(
            scope
                .snapshot(runtime.bindings().await, local)
                .await
                .entries()
                .is_empty()
        );
        assert!(scope.cache.lock().unwrap().is_empty());
        close(&runtime, jobs).await;
    }

    #[tokio::test]
    async fn local_catalog_change_revalidates_cached_definitions() {
        let database = TestDb::new();
        let (runtime, jobs, _) = runtime(database.clone(), &["database"]).await;
        let scope = ScopeVdbs::new();
        let local = local();
        assert!(
            scope
                .snapshot(runtime.bindings().await, local.clone())
                .await
                .get_available("fx")
                .is_some()
        );
        assert!(
            scope
                .snapshot(runtime.bindings().await, Arc::new((*local).clone()))
                .await
                .get_available("fx")
                .is_some()
        );
        assert_eq!(database.calls.load(Ordering::SeqCst), 1);
        let mut changed = (*local).clone();
        changed.upsert_attribute(attribute("test:value"));
        let set = scope
            .snapshot(runtime.bindings().await, Arc::new(changed))
            .await;
        assert!(reason(&set, "fx").contains("test:value"));
        assert_eq!(database.calls.load(Ordering::SeqCst), 1);
        assert!(
            scope
                .snapshot(runtime.bindings().await, local)
                .await
                .get_available("fx")
                .is_some(),
            "restored catalog must discard the conflicting overlay"
        );
        close(&runtime, jobs).await;
    }

    #[tokio::test]
    async fn own_collection_relationships_are_runtime_only() {
        let database = TestDb::new();
        database
            .descriptor
            .lock()
            .unwrap()
            .schema
            .relationships
            .push(RelationType {
                id: "test:links".into(),
                name: "Links".into(),
                source_collection: "fx".into(),
                mode: RelationMode::External,
                indexing_mode: RelationIndexingMode::Disabled,
                meta: Meta::default(),
            });
        let (runtime, jobs, _) = runtime(database, &["database"]).await;
        let local = local();
        let set = ScopeVdbs::new()
            .snapshot(runtime.bindings().await, local.clone())
            .await;
        assert!(set.get_available("fx").is_some());
        assert_eq!(set.schema("fx").unwrap().relationships.len(), 1);
        assert!(local.relationship_by_id("test:links").is_none());
        close(&runtime, jobs).await;
    }
}
