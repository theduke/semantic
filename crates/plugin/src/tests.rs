use super::*;
use semantic_data::schema::{BoolType, Type, TypeKind};
use semantic_jobs::*;
use semantic_rpc::interface::{InvocationFuture, OwnedValueStream, StreamEvent};

struct EmptyStore;

struct FailingProvider {
    fixture: Arc<Fixture>,
    cleanup_started: tokio::sync::Semaphore,
    cleanup_release: tokio::sync::Semaphore,
    cleanup_count: AtomicUsize,
    terminated: CancellationToken,
}
impl PluginShutdown for FailingProvider {
    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<(), PluginError>> + Send + '_>> {
        Box::pin(async {
            self.cleanup_count.fetch_add(1, Ordering::SeqCst);
            self.cleanup_started.add_permits(1);
            self.cleanup_release.acquire().await.unwrap().forget();
            Ok(())
        })
    }
    fn terminated(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.terminated.cancelled())
    }
}
impl InterfaceImplementation for FailingProvider {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &self.fixture.manifest.exports
    }
    fn invoke<'a>(&'a self, _: ValidatedInvocation, _: InvocationContext) -> InvocationFuture<'a> {
        Box::pin(async { Err(InvocationError::new("session_closed", "fixture failure")) })
    }
}
impl PluginProviderFactory for Arc<FailingProvider> {
    fn start<'a>(
        &'a self,
        _: &'a PluginActivation,
        _: PluginInstanceContext,
    ) -> Pin<Box<dyn Future<Output = Result<StartedPlugin, PluginError>> + Send + 'a>> {
        Box::pin(async {
            Ok(StartedPlugin {
                implementation: self.clone(),
                shutdown: Some(self.clone()),
            })
        })
    }
}
struct CancelJob(JobKindDescriptor);
impl JobHandler for CancelJob {
    type Input = Arc<tokio::sync::Semaphore>;
    type Output = ();
    fn kind(&self) -> &JobKindDescriptor {
        &self.0
    }
    fn run<'a>(
        &'a self,
        started: Self::Input,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), JobError>> + Send + 'a>> {
        Box::pin(async move {
            started.add_permits(1);
            context.cancellation().cancelled().await;
            Ok(())
        })
    }
}

#[tokio::test]
async fn active_session_failure_owns_job_drain_and_provider_cleanup() {
    let fixture = Fixture::new(false);
    let provider = Arc::new(FailingProvider {
        fixture: fixture.clone(),
        cleanup_started: tokio::sync::Semaphore::new(0),
        cleanup_release: tokio::sync::Semaphore::new(0),
        cleanup_count: AtomicUsize::new(0),
        terminated: CancellationToken::new(),
    });
    let mut registry = PluginRegistry::new();
    registry
        .register_provider("websocket", provider.clone())
        .unwrap();
    registry.register(fixture.clone()).unwrap();
    let mut builder = JobsBuilder::new();
    let handler = builder
        .register(CancelJob(JobKindDescriptor {
            id: JobKindId("fixture.cancel".into()),
            title: "Cancel".into(),
            description: None,
        }))
        .unwrap();
    let jobs = ScopeJobs::open(
        Arc::new(EmptyStore),
        builder.build(),
        JobsConfig {
            max_concurrent_jobs: 1.try_into().unwrap(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let runtime = Arc::new(ScopePlugins::new("scope", registry, jobs.clone()));
    runtime.activate(fixture.activation(true)).await.unwrap();
    let binding = runtime.bindings().await.remove(0);
    let started = Arc::new(tokio::sync::Semaphore::new(0));
    let options = || SubmitOptions {
        groups: vec![binding.group()],
    };
    let running = jobs
        .submit(&handler, started.clone(), options())
        .await
        .unwrap();
    started.acquire().await.unwrap().forget();
    let queued = jobs
        .submit(&handler, started.clone(), options())
        .await
        .unwrap();
    assert!(binding.invoke("fail", vec![]).await.is_err());
    // Invocation cancellation alone must start cleanup; no termination signal.
    provider.cleanup_started.acquire().await.unwrap().forget();
    assert!(binding.cancellation().is_cancelled());
    assert!(matches!(
        running.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    assert!(matches!(
        queued.wait().await,
        Err(JobCompletionError::Cancelled(_))
    ));
    assert_eq!(started.available_permits(), 0);
    assert!(jobs.submit(&handler, started, options()).await.is_err());
    let replace = tokio::spawn({
        let runtime = runtime.clone();
        let mut activation = fixture.activation(false);
        activation.generation = 2;
        async move { runtime.activate(activation).await }
    });
    tokio::task::yield_now().await;
    assert!(!replace.is_finished());
    provider.cleanup_release.add_permits(1);
    replace.await.unwrap().unwrap();
    provider.terminated.cancel();
    // Explicitly join the old owner again after replacement: idempotent cleanup.
    binding.generation.stop("stale").await.unwrap();
    assert_eq!(provider.cleanup_count.load(Ordering::SeqCst), 1);
    let current = runtime.bindings().await.remove(0);
    assert_eq!(current.generation(), 2);
    assert!(!current.cancellation().is_cancelled());
    assert_eq!(runtime.states().await[0].1, PluginState::Ready);
    runtime.shutdown().await.unwrap();
    jobs.shutdown().await.unwrap();
}

struct ObservedProvider {
    fixture: Arc<Fixture>,
    terminations: std::sync::Mutex<Vec<CancellationToken>>,
}
struct ObservedShutdown(CancellationToken);
impl PluginShutdown for ObservedShutdown {
    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<(), PluginError>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
    fn terminated(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.0.cancelled())
    }
}
impl PluginProviderFactory for Arc<ObservedProvider> {
    fn start<'a>(
        &'a self,
        _: &'a PluginActivation,
        context: PluginInstanceContext,
    ) -> Pin<Box<dyn Future<Output = Result<StartedPlugin, PluginError>> + Send + 'a>> {
        Box::pin(async move {
            let termination = CancellationToken::new();
            self.terminations.lock().unwrap().push(termination.clone());
            Ok(StartedPlugin {
                implementation: self.fixture.create(context).await?,
                shutdown: Some(Arc::new(ObservedShutdown(termination))),
            })
        })
    }
}

#[tokio::test]
async fn old_termination_observer_cannot_invalidate_replacement() {
    let fixture = Fixture::new(false);
    let provider = Arc::new(ObservedProvider {
        fixture: fixture.clone(),
        terminations: std::sync::Mutex::new(Vec::new()),
    });
    let mut registry = PluginRegistry::new();
    registry
        .register_provider("websocket", provider.clone())
        .unwrap();
    let jobs = ScopeJobs::open(
        Arc::new(EmptyStore),
        JobsBuilder::new().build(),
        Default::default(),
    )
    .await
    .unwrap();
    let runtime = ScopePlugins::new("scope", registry, jobs.clone());
    let mut activation = fixture.activation(true);
    runtime.activate(activation.clone()).await.unwrap();
    let old = runtime.bindings().await.remove(0);
    activation.generation = 2;
    runtime.activate(activation).await.unwrap();
    provider.terminations.lock().unwrap()[0].cancel();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(old.cancellation().is_cancelled());
    let current = runtime.bindings().await.remove(0);
    assert_eq!(current.generation(), 2);
    assert!(!current.cancellation().is_cancelled());
    assert_eq!(runtime.states().await[0].1, PluginState::Ready);
    runtime.shutdown().await.unwrap();
    jobs.shutdown().await.unwrap();
}

#[cfg(feature = "stdio")]
#[tokio::test]
#[ignore = "build semantic_rpc --example interface_fixture --features plugin-stdio first"]
async fn idle_stdio_exit_closes_generation_without_invocation() {
    let executable = std::env::var("SEMANTIC_INTERFACE_FIXTURE").unwrap_or_else(|_| {
        format!(
            "{}/../../target/debug/examples/interface_fixture",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    let jobs = ScopeJobs::open(
        Arc::new(EmptyStore),
        JobsBuilder::new().build(),
        Default::default(),
    )
    .await
    .unwrap();
    let runtime = ScopePlugins::new(
        "scope",
        PluginRegistry::new().with_host_providers(),
        jobs.clone(),
    );
    let mut activation = Fixture::new(false).activation(true);
    activation.provider = PluginProvider::Stdio {
        program: executable,
        args: vec!["--exit-after-ready".into()],
        cwd: None,
        env: Default::default(),
    };
    runtime.activate(activation.clone()).await.unwrap();
    let binding = runtime.bindings().await.remove(0);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if runtime.states().await[0].1 == PluginState::Unavailable {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(binding.cancellation().is_cancelled());
    assert!(runtime.bindings().await.is_empty());
    activation.generation += 1;
    if let PluginProvider::Stdio { args, .. } = &mut activation.provider {
        *args = vec!["--crash-on-invoke".into()];
    }
    runtime.activate(activation).await.unwrap();
    assert_eq!(runtime.states().await[0].1, PluginState::Ready);
    assert!(!runtime.bindings().await[0].cancellation().is_cancelled());
    runtime.shutdown().await.unwrap();
    jobs.shutdown().await.unwrap();
}

#[cfg(feature = "stdio")]
#[tokio::test]
#[ignore = "build semantic_rpc --example interface_fixture --features plugin-stdio first"]
async fn stdio_crash_allows_explicit_restart_and_disable() {
    let executable = std::env::var("SEMANTIC_INTERFACE_FIXTURE").unwrap_or_else(|_| {
        format!(
            "{}/../../target/debug/examples/interface_fixture",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    assert!(std::path::Path::new(&executable).is_file());
    let jobs = ScopeJobs::open(
        Arc::new(EmptyStore),
        JobsBuilder::new().build(),
        Default::default(),
    )
    .await
    .unwrap();
    let runtime = ScopePlugins::new(
        "scope",
        PluginRegistry::new().with_host_providers(),
        jobs.clone(),
    );
    let mut activation = Fixture::new(false).activation(true);
    activation.provider = PluginProvider::Stdio {
        program: executable,
        args: vec!["--crash-on-invoke".into()],
        cwd: None,
        env: Default::default(),
    };
    for generation in 1..=2 {
        activation.generation = generation;
        runtime.activate(activation.clone()).await.unwrap();
        assert_eq!(runtime.states().await[0].1, PluginState::Ready);
        let binding = runtime.bindings().await.remove(0);
        assert_eq!(binding.generation(), generation);
        let error = match binding.invoke("crash", vec![]).await {
            Err(error) => error,
            Ok(_) => panic!("crashed provider returned successfully"),
        };
        assert_eq!(error.code, "connection_lost");
        let states = runtime.states().await;
        assert_eq!(states[0].1, PluginState::Unavailable);
        assert_eq!(states[0].2.as_ref().unwrap().code, "connection_lost");
    }
    activation.generation += 1;
    activation.enabled = false;
    runtime.activate(activation).await.unwrap();
    assert_eq!(runtime.states().await[0].1, PluginState::Disabled);
    assert!(runtime.bindings().await.is_empty());
    runtime.shutdown().await.unwrap();
    jobs.shutdown().await.unwrap();
}

#[async_trait::async_trait]
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

struct Fixture {
    manifest: PluginManifest,
    creates: AtomicUsize,
    entered: tokio::sync::Semaphore,
    cancelled: tokio::sync::Semaphore,
    cleanup: tokio::sync::Semaphore,
    cleaned: AtomicUsize,
}
impl Fixture {
    fn new(schema: bool) -> Arc<Self> {
        Arc::new(Self {
            manifest: PluginManifest {
                id: "fixture".into(),
                revision: "1".into(),
                title: "Fixture".into(),
                exports: vec![ImplementationDescriptor {
                    export: "fixture".into(),
                    interface: semantic_rpc_core::interface::InterfaceRef {
                        package: "fixture".into(),
                        module: "v1".into(),
                        contract: None,
                        name: "Fixture".into(),
                    },
                    package_version: "1".into(),
                    fingerprint: "fixture".into(),
                }],
                source_bindings: Default::default(),
                configuration_schema: schema.then(|| {
                    semantic_data::plugin::PluginConfigurationSchema {
                        ty: Type::new(TypeKind::Bool(BoolType)),
                        definitions: Default::default(),
                    }
                }),
            },
            creates: AtomicUsize::new(0),
            entered: tokio::sync::Semaphore::new(0),
            cancelled: tokio::sync::Semaphore::new(0),
            cleanup: tokio::sync::Semaphore::new(0),
            cleaned: AtomicUsize::new(0),
        })
    }
    fn activation(&self, host: bool) -> PluginActivation {
        PluginActivation {
            id: "fixture".into(),
            revision: "1".into(),
            provider: if host {
                PluginProvider::WebSocket {
                    url: "ws://fixture".into(),
                }
            } else {
                PluginProvider::Rust {
                    key: "fixture".into(),
                }
            },
            enabled: true,
            generation: 1,
            configuration: Value::Bool(true),
            source_bindings: Default::default(),
            configuration_schema: self.manifest.configuration_schema.clone(),
            priority: None,
            exports: self
                .manifest
                .exports
                .iter()
                .cloned()
                .map(portable_export)
                .collect(),
        }
    }
}
impl Plugin for Arc<Fixture> {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn create<'a>(
        &'a self,
        _: PluginInstanceContext,
    ) -> Pin<
        Box<dyn Future<Output = Result<Arc<dyn InterfaceImplementation>, PluginError>> + Send + 'a>,
    > {
        self.creates.fetch_add(1, Ordering::SeqCst);
        let fixture = self.clone();
        Box::pin(async move {
            Ok(Arc::new(Implementation(fixture)) as Arc<dyn InterfaceImplementation>)
        })
    }
}
impl PluginProviderFactory for Arc<Fixture> {
    fn start<'a>(
        &'a self,
        _: &'a PluginActivation,
        context: PluginInstanceContext,
    ) -> Pin<Box<dyn Future<Output = Result<StartedPlugin, PluginError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(StartedPlugin {
                implementation: self.create(context).await?,
                shutdown: None,
            })
        })
    }
}
struct Implementation(Arc<Fixture>);
impl InterfaceImplementation for Implementation {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &self.0.manifest.exports
    }
    fn invoke<'a>(
        &'a self,
        call: ValidatedInvocation,
        context: InvocationContext,
    ) -> InvocationFuture<'a> {
        Box::pin(async move {
            if call.method == "stream" {
                let fixture = self.0.clone();
                return Ok(InvocationOutput::Stream(OwnedValueStream::new(
                    futures_util::stream::once(async move {
                        fixture.entered.add_permits(1);
                        context.cancellation.cancelled().await;
                        fixture.cancelled.add_permits(1);
                        fixture.cleanup.acquire().await.unwrap().forget();
                        fixture.cleaned.fetch_add(1, Ordering::SeqCst);
                        Ok(StreamEvent::End(None))
                    }),
                )));
            }
            self.0.entered.add_permits(1);
            context.cancellation.cancelled().await;
            self.0.cancelled.add_permits(1);
            self.0.cleanup.acquire().await.unwrap().forget();
            self.0.cleaned.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationOutput::Values(vec![]))
        })
    }
}
async fn runtime(fixture: &Arc<Fixture>) -> (Arc<ScopePlugins>, ScopeJobs) {
    let jobs = ScopeJobs::open(
        Arc::new(EmptyStore),
        JobsBuilder::new().build(),
        Default::default(),
    )
    .await
    .unwrap();
    let mut registry = PluginRegistry::new();
    registry.register(fixture.clone()).unwrap();
    registry
        .register_provider("websocket", fixture.clone())
        .unwrap();
    (
        Arc::new(ScopePlugins::new("scope", registry, jobs.clone())),
        jobs,
    )
}

#[tokio::test]
async fn replacement_waits_for_cooperative_invocation_cleanup_even_after_caller_drop() {
    for drop_caller in [false, true] {
        let fixture = Fixture::new(false);
        let (runtime, jobs) = runtime(&fixture).await;
        let mut activation = fixture.activation(false);
        runtime.activate(activation.clone()).await.unwrap();
        let binding = runtime.bindings().await.remove(0);
        let call = tokio::spawn(async move { binding.invoke("wait", vec![]).await });
        fixture.entered.acquire().await.unwrap().forget();
        if drop_caller {
            call.abort();
        }
        activation.generation += 1;
        let replacement = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.activate(activation).await })
        };
        fixture.cancelled.acquire().await.unwrap().forget();
        assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
        assert!(!replacement.is_finished());
        fixture.cleanup.add_permits(1);
        replacement.await.unwrap().unwrap();
        assert_eq!(fixture.cleaned.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.creates.load(Ordering::SeqCst), 2);
        if !drop_caller {
            assert!(call.await.unwrap().is_err());
        }
        runtime.shutdown().await.unwrap();
        jobs.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn stream_drop_retains_cleanup_barrier() {
    let fixture = Fixture::new(false);
    let (runtime, jobs) = runtime(&fixture).await;
    let mut activation = fixture.activation(false);
    runtime.activate(activation.clone()).await.unwrap();
    let binding = runtime.bindings().await.remove(0);
    let InvocationOutput::Stream(mut stream) = binding.invoke("stream", vec![]).await.unwrap()
    else {
        panic!()
    };
    let reader = tokio::spawn(async move { stream.next().await });
    fixture.entered.acquire().await.unwrap().forget();
    reader.abort();
    // Consumer disappearance alone wakes cleanup before any replacement.
    fixture.cancelled.acquire().await.unwrap().forget();
    activation.generation += 1;
    let replacement = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.activate(activation).await })
    };
    assert!(!replacement.is_finished());
    fixture.cleanup.add_permits(1);
    replacement.await.unwrap().unwrap();
    assert_eq!(fixture.cleaned.load(Ordering::SeqCst), 1);
    runtime.shutdown().await.unwrap();
    jobs.shutdown().await.unwrap();
}

#[tokio::test]
async fn activation_is_single_flight_and_scope_shutdown_is_isolated() {
    let fixture = Fixture::new(false);
    let (first, first_jobs) = runtime(&fixture).await;
    let (second, second_jobs) = runtime(&fixture).await;
    let activation = fixture.activation(false);
    let (a, b) = tokio::join!(
        first.activate(activation.clone()),
        first.activate(activation.clone())
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
    second.activate(activation).await.unwrap();
    first.shutdown().await.unwrap();
    assert!(first.bindings().await.is_empty());
    assert_eq!(second.bindings().await.len(), 1);
    second.shutdown().await.unwrap();
    first_jobs.shutdown().await.unwrap();
    second_jobs.shutdown().await.unwrap();
}

#[cfg(any(feature = "stdio", feature = "websocket"))]
#[tokio::test]
async fn host_cleanup_survives_abandoned_wait_and_preserves_failure() {
    let (release, receive) = tokio::sync::oneshot::channel();
    let cleanup = tokio::spawn(async move {
        receive.await.unwrap();
        Err(PluginError::new("cleanup_failed", "fixture failure"))
    });
    let shutdown = Arc::new(HostShutdown(
        Mutex::new(HostShutdownState {
            connection: None,
            cleanup: Some(cleanup),
            result: None,
        }),
        CancellationToken::new(),
    ));
    let waiter = {
        let shutdown = shutdown.clone();
        tokio::spawn(async move { shutdown.shutdown().await })
    };
    tokio::task::yield_now().await;
    waiter.abort();
    let _ = waiter.await;
    release.send(()).unwrap();
    for _ in 0..2 {
        assert_eq!(
            shutdown.shutdown().await.unwrap_err().code,
            "cleanup_failed"
        );
    }
}

#[tokio::test]
async fn configuration_validation_has_native_host_parity_and_preserves_generation() {
    for host in [false, true] {
        let fixture = Fixture::new(true);
        let (runtime, jobs) = runtime(&fixture).await;
        let mut activation = fixture.activation(host);
        assert_eq!(
            PluginActivation::from_value(&activation.to_value()).unwrap(),
            activation
        );
        runtime.activate(activation.clone()).await.unwrap();
        activation.generation += 1;
        activation.configuration = Value::String("invalid".into());
        assert_eq!(
            runtime.activate(activation).await.unwrap_err().code,
            "invalid_configuration"
        );
        assert_eq!(fixture.creates.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.bindings().await[0].generation(), 1);
        runtime.shutdown().await.unwrap();
        jobs.shutdown().await.unwrap();
    }
}
