use super::*;
use crate::{DbScopeId, Principal, SemanticApp};
use semantic_data::schema::*;
use semantic_db_core::{
    BatchOutcome, DbError, EntityRecord, PackageRegistrationOutcome, TextQueryInput,
};
use std::sync::atomic::{AtomicBool, Ordering};

struct StartupPlugin {
    manifest: semantic_plugin::PluginManifest,
    started: Arc<tokio::sync::Semaphore>,
    release: Option<Arc<tokio::sync::Semaphore>>,
    tokens: Arc<std::sync::Mutex<Vec<semantic_jobs::CancellationToken>>>,
}
impl semantic_plugin::Plugin for StartupPlugin {
    fn manifest(&self) -> &semantic_plugin::PluginManifest {
        &self.manifest
    }
    fn create<'a>(
        &'a self,
        context: semantic_plugin::PluginInstanceContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Arc<dyn semantic_rpc::interface::InterfaceImplementation>,
                        semantic_plugin::PluginError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.tokens
                .lock()
                .unwrap()
                .push(context.cancellation.clone());
            self.started.add_permits(1);
            if let Some(release) = &self.release {
                tokio::select! {
                    _ = context.cancellation.cancelled() => return Err(semantic_plugin::PluginError::new("cancelled", "startup cancelled")),
                    permit = release.acquire() => permit.unwrap().forget(),
                }
            }
            semantic_plugin::Plugin::create(
                &semantic_import::GenericUrlPlugin::new(vec![]),
                context,
            )
            .await
        })
    }
}

#[tokio::test]
async fn cancelled_initial_open_keeps_one_owned_runtime_and_cleans_up() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let started = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let tokens = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = SemanticApp::builder().with_default_scope(DbScopeId::new("main"), db);
    for (id, gated) in [("a-first", false), ("b-second", true)] {
        builder = builder
            .register_plugin(StartupPlugin {
                manifest: semantic_plugin::PluginManifest {
                    id: id.into(),
                    revision: "1".into(),
                    title: id.into(),
                    exports: vec![],
                    source_bindings: Default::default(),
                    configuration_schema: None,
                },
                started: started.clone(),
                release: gated.then(|| release.clone()),
                tokens: tokens.clone(),
            })
            .unwrap();
    }
    let app = builder.build().unwrap();
    let caller_app = app.clone();
    let caller = tokio::spawn(async move {
        caller_app
            .plugins(&Principal::system(), DbScopeId::new("main"))
            .await
    });
    started.acquire_many(2).await.unwrap().forget();
    assert_eq!(tokens.lock().unwrap().len(), 2);
    caller.abort();
    assert!(matches!(caller.await, Err(error) if error.is_cancelled()));
    let second_app = app.clone();
    let second = tokio::spawn(async move {
        second_app
            .plugins(&Principal::system(), DbScopeId::new("main"))
            .await
    });
    release.add_permits(1);
    let plugins = tokio::time::timeout(std::time::Duration::from_secs(5), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let again = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&plugins, &again));
    assert_eq!(tokens.lock().unwrap().len(), 2);
    assert!(tokens.lock().unwrap().iter().all(|t| !t.is_cancelled()));
    app.shutdown().await.unwrap();
    assert!(tokens.lock().unwrap().iter().all(|t| t.is_cancelled()));
}

#[tokio::test]
async fn reopened_missing_or_changed_rust_plugin_can_be_disabled_and_uninstalled() {
    for changed_schema in [false, true] {
        let (_temp, app, db, plugins) = fixture().await;
        let mut activation = plugins.list().await.unwrap().remove(0);
        let id = activation.id.clone();
        app.shutdown().await.unwrap();
        let jobs_app = SemanticApp::builder()
            .with_default_scope(DbScopeId::new("main"), db.clone())
            .build()
            .unwrap();
        let jobs = jobs_app
            .jobs(&Principal::system(), DbScopeId::new("main"))
            .await
            .unwrap();
        let mut registry = PluginRegistry::new();
        if changed_schema {
            registry
                .register(StartupPlugin {
                    manifest: semantic_plugin::PluginManifest {
                        id: id.clone(),
                        revision: activation.revision.clone(),
                        title: id.clone(),
                        exports: vec![],
                        source_bindings: Default::default(),
                        configuration_schema: Some(PluginConfigurationSchema {
                            ty: Type::new_bool(),
                            definitions: Default::default(),
                        }),
                    },
                    started: Arc::new(tokio::sync::Semaphore::new(0)),
                    release: None,
                    tokens: Default::default(),
                })
                .unwrap();
        }
        let reopened = Arc::new(
            AppScopePlugins::open("main".into(), registry, jobs, db)
                .await
                .unwrap(),
        );
        assert!(
            reopened
                .runtime
                .states()
                .await
                .iter()
                .any(|(key, state, error)| key == &id
                    && *state == PluginState::Unavailable
                    && error.is_some())
        );
        let before = reopened.list().await.unwrap();
        assert!(reopened.configure(activation.clone()).await.is_err());
        assert_eq!(reopened.list().await.unwrap(), before);
        activation.enabled = false;
        reopened.configure(activation).await.unwrap();
        reopened.uninstall(&id).await.unwrap();
        assert!(reopened.list().await.unwrap().is_empty());
        reopened.runtime.shutdown().await.unwrap();
        jobs_app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn package_update_waits_for_initial_plugin_publication() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let started = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let app = SemanticApp::builder()
        .with_default_scope("main".into(), db)
        .register_plugin(StartupPlugin {
            manifest: semantic_plugin::PluginManifest {
                id: "zz-gated".into(),
                revision: "1".into(),
                title: "Gated".into(),
                exports: vec![],
                source_bindings: Default::default(),
                configuration_schema: None,
            },
            started: started.clone(),
            release: Some(release.clone()),
            tokens: Default::default(),
        })
        .unwrap()
        .build()
        .unwrap();
    let opening = {
        let app = app.clone();
        tokio::spawn(async move {
            app.plugins(&Principal::system(), "main".into())
                .await
                .unwrap()
        })
    };
    started.acquire().await.unwrap().forget();
    let principal = Principal::system();
    let scope = DbScopeId::new("main");
    let mut update = Box::pin(app.scopes().update_package(
        &principal,
        &scope,
        semantic_data::import::package(),
    ));
    assert!(futures_util::poll!(&mut update).is_pending());
    release.add_permits(1);
    update.await.unwrap();
    let plugins = opening.await.unwrap();
    assert!(
        plugins
            .runtime
            .bindings()
            .await
            .iter()
            .all(|binding| binding.generation() == 2)
    );
    assert!(!plugins.runtime.bindings().await.is_empty());
    app.shutdown().await.unwrap();
}

struct CleanupJob(semantic_jobs::JobKindDescriptor);
impl semantic_jobs::JobHandler for CleanupJob {
    type Input = (
        Arc<tokio::sync::Semaphore>,
        Arc<tokio::sync::Semaphore>,
        Arc<tokio::sync::Semaphore>,
    );
    type Output = ();
    fn kind(&self) -> &semantic_jobs::JobKindDescriptor {
        &self.0
    }
    fn run<'a>(
        &'a self,
        input: Self::Input,
        context: semantic_jobs::JobContext,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), semantic_jobs::JobError>> + Send + 'a>,
    > {
        Box::pin(async move {
            input.0.add_permits(1);
            context.cancellation().cancelled().await;
            input.1.add_permits(1);
            input.2.acquire().await.unwrap().forget();
            Ok(())
        })
    }
}

#[tokio::test]
async fn close_cancels_initial_startup_without_manual_release_and_signals_jobs() {
    for close_scope in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(semantic_db_core::Db::new(
            semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
                .unwrap(),
        ));
        let started = Arc::new(tokio::sync::Semaphore::new(0));
        let tokens = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut registry = semantic_jobs::JobsBuilder::new();
        let handler = registry
            .register(CleanupJob(semantic_jobs::JobKindDescriptor {
                id: semantic_jobs::JobKindId("test.startup-cleanup".into()),
                title: "Cleanup".into(),
                description: None,
            }))
            .unwrap();
        let app = SemanticApp::builder()
            .with_default_scope("main".into(), db)
            .with_jobs(registry.build(), Default::default())
            .register_plugin(StartupPlugin {
                manifest: semantic_plugin::PluginManifest {
                    id: "zz-startup-gate".into(),
                    revision: "1".into(),
                    title: "Gate".into(),
                    exports: vec![],
                    configuration_schema: None,
                    source_bindings: Default::default(),
                },
                started: started.clone(),
                // This gate is never released: only lifecycle cancellation lets create exit.
                release: Some(Arc::new(tokio::sync::Semaphore::new(0))),
                tokens: tokens.clone(),
            })
            .unwrap()
            .build()
            .unwrap();
        let jobs = app.jobs(&Principal::system(), "main".into()).await.unwrap();
        let job_entered = Arc::new(tokio::sync::Semaphore::new(0));
        let job_cancelled = Arc::new(tokio::sync::Semaphore::new(0));
        let ticket = jobs
            .submit(
                &handler,
                (
                    job_entered.clone(),
                    job_cancelled.clone(),
                    Arc::new(tokio::sync::Semaphore::new(1)),
                ),
                Default::default(),
            )
            .await
            .unwrap();
        job_entered.acquire().await.unwrap().forget();
        let opening = {
            let app = app.clone();
            tokio::spawn(async move { app.plugins(&Principal::system(), "main".into()).await })
        };
        started.acquire().await.unwrap().forget();
        let stopping = {
            let app = app.clone();
            tokio::spawn(async move {
                if close_scope {
                    app.scopes()
                        .close_scope_with_jobs(&Principal::system(), &"main".into())
                        .await
                } else {
                    app.shutdown().await
                }
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), stopping)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(opening.await.unwrap().is_err());
        assert!(
            tokens
                .lock()
                .unwrap()
                .iter()
                .all(|token| token.is_cancelled())
        );
        assert!(job_cancelled.try_acquire().is_ok());
        assert!(ticket.wait().await.is_err());
        assert!(app.jobs(&Principal::system(), "main".into()).await.is_err());
        assert!(
            app.plugins(&Principal::system(), "main".into())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn scope_shutdown_cancels_all_plugins_and_unrelated_jobs_before_cleanup() {
    for close_scope in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = Arc::new(semantic_db_core::Db::new(
            semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
                .unwrap(),
        ));
        let mut registry = semantic_jobs::JobsBuilder::new();
        let handler = registry
            .register(CleanupJob(semantic_jobs::JobKindDescriptor {
                id: semantic_jobs::JobKindId("test.cleanup".into()),
                title: "Cleanup".into(),
                description: None,
            }))
            .unwrap();
        let app = SemanticApp::builder()
            .with_default_scope("main".into(), db)
            .with_jobs(registry.build(), Default::default())
            .build()
            .unwrap();
        let principal = Principal::system();
        let plugins = app.plugins(&principal, "main".into()).await.unwrap();
        let mut second = plugins.list().await.unwrap().remove(0);
        second.id = "second-url".into();
        plugins.configure(second).await.unwrap();
        let bindings = plugins.runtime.bindings().await;
        let first = bindings
            .iter()
            .find(|b| b.plugin_id() != "second-url")
            .unwrap();
        let second = bindings
            .iter()
            .find(|b| b.plugin_id() == "second-url")
            .unwrap();
        let jobs = app.jobs(&principal, "main".into()).await.unwrap();
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let cancelled = Arc::new(tokio::sync::Semaphore::new(0));
        let cleanup = Arc::new(tokio::sync::Semaphore::new(0));
        for groups in [vec![first.group()], vec![second.group()], vec![]] {
            jobs.submit(
                &handler,
                (entered.clone(), cancelled.clone(), cleanup.clone()),
                semantic_jobs::SubmitOptions { groups },
            )
            .await
            .unwrap();
        }
        entered.acquire_many(3).await.unwrap().forget();
        let stopping = {
            let app = app.clone();
            tokio::spawn(async move {
                if close_scope {
                    app.scopes()
                        .close_scope_with_jobs(&Principal::system(), &"main".into())
                        .await
                } else {
                    app.shutdown().await
                }
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), cancelled.acquire_many(3))
            .await
            .unwrap()
            .unwrap()
            .forget();
        assert!(first.cancellation().is_cancelled());
        assert!(second.cancellation().is_cancelled());
        assert!(!stopping.is_finished());
        assert!(jobs.create_group().await.is_err());
        cleanup.add_permits(3);
        stopping.await.unwrap().unwrap();
    }
}

struct PackageGate {
    inner: Arc<dyn SemanticDb>,
    armed: AtomicBool,
    fail: AtomicBool,
    fail_first_persist: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl SemanticDb for PackageGate {
    async fn catalog(&self) -> Result<Arc<semantic_db_core::catalog::Catalog>, DbError> {
        self.inner.catalog().await
    }
    async fn query(&self, query: TextQueryInput) -> Result<QueryResult, DbError> {
        self.inner.query(query).await
    }
    async fn query_data(
        &self,
        query: semantic_data::query::QueryInput,
    ) -> Result<QueryResult, DbError> {
        self.inner.query_data(query).await
    }
    async fn get(&self, collection: String, id: String) -> Result<Option<EntityRecord>, DbError> {
        self.inner.get(collection, id).await
    }
    async fn insert(&self, collection: String, id: String, object: Object) -> Result<(), DbError> {
        self.inner.insert(collection, id, object).await
    }
    async fn delete(&self, collection: String, id: String) -> Result<(), DbError> {
        self.inner.delete(collection, id).await
    }
    async fn execute_batch(&self, batch: Batch) -> Result<BatchOutcome, DbError> {
        if self.fail_first_persist.swap(false, Ordering::SeqCst) {
            return Err(DbError::InvalidQuery(
                "injected activation persistence failure".into(),
            ));
        }
        self.inner.execute_batch(batch).await
    }
    async fn upsert_package(
        &self,
        package: Package,
    ) -> Result<PackageRegistrationOutcome, DbError> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
            if self.fail.load(Ordering::SeqCst) {
                return Err(DbError::InvalidQuery(
                    "injected registration failure".into(),
                ));
            }
        }
        self.inner.upsert_package(package).await
    }
}

async fn fixture() -> (
    tempfile::TempDir,
    SemanticApp,
    Arc<PackageGate>,
    Arc<AppScopePlugins>,
) {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(PackageGate {
        inner: Arc::new(semantic_db_core::Db::new(
            semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
                .unwrap(),
        )),
        armed: AtomicBool::new(false),
        fail: AtomicBool::new(false),
        fail_first_persist: AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .build()
        .unwrap();
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    (temp, app, db, plugins)
}

#[tokio::test]
async fn failed_package_registration_restores_bindings_with_new_generation() {
    let (_temp, app, db, plugins) = fixture().await;
    let stale = plugins.runtime.bindings().await.remove(0);
    db.armed.store(true, Ordering::SeqCst);
    db.fail.store(true, Ordering::SeqCst);
    let task = {
        let plugins = plugins.clone();
        tokio::spawn(async move {
            plugins
                .update_package(semantic_data::import::package())
                .await
        })
    };
    db.entered.notified().await;
    assert!(plugins.runtime.bindings().await.is_empty());
    db.release.notify_one();
    assert!(task.await.unwrap().is_err());
    let bindings = plugins.runtime.bindings().await;
    assert!(!bindings.is_empty());
    assert_eq!(bindings[0].generation(), stale.generation() + 1);
    assert!(stale.cancellation().is_cancelled());
    assert_eq!(
        plugins.list().await.unwrap()[0].generation,
        bindings[0].generation()
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_package_caller_does_not_cancel_reconciliation() {
    let (_temp, app, db, plugins) = fixture().await;
    let old_generation = plugins.list().await.unwrap()[0].generation;
    db.armed.store(true, Ordering::SeqCst);
    let task = {
        let plugins = plugins.clone();
        tokio::spawn(async move {
            plugins
                .update_package(semantic_data::import::package())
                .await
        })
    };
    db.entered.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    db.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if plugins
                .runtime
                .bindings()
                .await
                .iter()
                .any(|b| b.generation() > old_generation)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        plugins.list().await.unwrap()[0].generation,
        old_generation + 1
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn one_failed_activation_write_does_not_strand_other_affected_plugins() {
    let (_temp, app, db, plugins) = fixture().await;
    let mut second = plugins.list().await.unwrap().remove(0);
    second.id = "test.second".into();
    plugins.configure(second).await.unwrap();
    db.armed.store(true, Ordering::SeqCst);
    let task = {
        let plugins = plugins.clone();
        tokio::spawn(async move {
            plugins
                .update_package(semantic_data::import::package())
                .await
        })
    };
    db.entered.notified().await;
    db.fail_first_persist.store(true, Ordering::SeqCst);
    db.release.notify_one();
    let err = task.await.unwrap().unwrap_err();
    assert!(
        err.to_string()
            .contains("injected activation persistence failure")
    );
    let bindings = plugins.runtime.bindings().await;
    let active_ids = bindings
        .iter()
        .map(|binding| binding.plugin_id())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        active_ids.len(),
        1,
        "the remaining affected activation must be reconciled"
    );
    assert!(bindings.iter().all(|binding| binding.generation() == 2));
    // The failed activation can be retried explicitly from its durable desired state.
    let failed = plugins
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|activation| !active_ids.contains(activation.id.as_str()))
        .unwrap();
    plugins.configure(failed).await.unwrap();
    assert_eq!(
        plugins
            .runtime
            .states()
            .await
            .iter()
            .filter(|(_, state, _)| *state == semantic_plugin::PluginState::Ready)
            .count(),
        2
    );
    app.shutdown().await.unwrap();
}

fn schema_package(name: &str) -> Package {
    Package {
        name: name.into(),
        root: Module {
            name: name.into(),
            constants: Default::default(),
            types: Default::default(),
            attributes: Default::default(),
            classes: Default::default(),
            interfaces: Default::default(),
            contracts: Default::default(),
            meta: Default::default(),
        },
        modules: Default::default(),
        migrations: Vec::new(),
        version: None,
        meta: Default::default(),
    }
}

fn typedef(name: &str, ty: Type) -> TypeDef {
    TypeDef {
        name: name.into(),
        module: None,
        params: Vec::new(),
        ty,
        visibility: Visibility::Public,
        meta: Default::default(),
    }
}

fn migrate_types(package: &mut Package, name: &str) {
    package.migrations.push(Migration {
        module: package.root.name.clone(),
        name: name.into(),
        description: None,
        operations: package
            .root
            .types
            .values()
            .cloned()
            .map(|type_def| {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertTypeDef { type_def })
            })
            .collect(),
        meta: Default::default(),
    });
}

#[tokio::test]
async fn transitive_type_package_change_invalidates_export_from_another_package() {
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let mut leaf = schema_package("leaf");
    leaf.root
        .types
        .insert("leaf:Value".into(), typedef("leaf:Value", Type::new_bool()));
    migrate_types(&mut leaf, "001");
    let mut middle = schema_package("middle");
    middle.root.types.insert(
        "middle:Alias".into(),
        typedef(
            "middle:Alias",
            Type::new(TypeKind::Ref(TypeRef::new("leaf:Value"))),
        ),
    );
    migrate_types(&mut middle, "001");
    let mut api = schema_package("api");
    api.root.interfaces.insert(
        "Lookup".into(),
        InterfaceType {
            methods: vec![InterfaceMethod {
                name: "get".into(),
                signature: FunctionType {
                    params: Vec::new(),
                    results: vec![Type::new(TypeKind::Ref(TypeRef::new("middle:Alias")))],
                    throws: None,
                    async_fn: true,
                },
            }],
        },
    );
    for package in [leaf.clone(), middle, api] {
        db.upsert_package(package).await.unwrap();
    }
    let catalog = db.catalog().await.unwrap();
    let interface = catalog
        .resolve_interface("api", "api", None, "Lookup")
        .unwrap();
    let descriptor = semantic_rpc_core::interface::ImplementationDescriptor {
        export: "lookup".into(),
        interface: semantic_rpc_core::interface::InterfaceRef {
            package: "api".into(),
            module: "api".into(),
            contract: None,
            name: "Lookup".into(),
        },
        package_version: String::new(),
        fingerprint: interface.fingerprint,
    };
    // No calls are needed: the URL factory also supplies a concrete Rust plugin
    // instance with this descriptor so lifecycle conformance is exercised.
    let mut registry = PluginRegistry::new();
    registry
        .register(semantic_import::GenericUrlPlugin::new(vec![descriptor]))
        .unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .build()
        .unwrap();
    let jobs = app
        .jobs(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let plugins = Arc::new(
        AppScopePlugins::open("main".into(), registry, jobs, db)
            .await
            .unwrap(),
    );
    let stale = plugins.runtime.bindings().await.remove(0);
    leaf.root.types.get_mut("leaf:Value").unwrap().ty = Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }));
    migrate_types(&mut leaf, "002");
    plugins.update_package(leaf).await.unwrap();
    assert!(stale.cancellation().is_cancelled());
    assert!(plugins.runtime.bindings().await.is_empty());
    let states = plugins.runtime.states().await;
    assert_eq!(states[0].2.as_ref().unwrap().code, "interface_incompatible");
    assert_eq!(
        plugins.list().await.unwrap()[0].generation,
        stale.generation() + 1
    );
    plugins.runtime.shutdown().await.unwrap();
    app.shutdown().await.unwrap();
}
