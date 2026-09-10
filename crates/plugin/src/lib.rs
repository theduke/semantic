//! Scope-local implementations of package interfaces.
#[cfg(test)]
mod tests;
use futures_util::StreamExt;
use semantic_data::Value;
use semantic_jobs::{CancellationToken, JobError, JobGroup, ScopeJobs};
use semantic_rpc::interface::{
    InterfaceImplementation, InvocationArgument, InvocationContext, InvocationOutput,
    ValidatedInvocation,
};
use semantic_rpc_core::interface::{ImplementationDescriptor, InvocationError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::{Mutex, RwLock};

pub use semantic_data::plugin::{PluginActivation, PluginProvider, PluginState};

pub fn portable_export(value: ImplementationDescriptor) -> semantic_data::plugin::PluginExport {
    semantic_data::plugin::PluginExport {
        export: value.export,
        package: value.interface.package,
        module: value.interface.module,
        contract: value.interface.contract,
        interface: value.interface.name,
        package_version: value.package_version,
        fingerprint: value.fingerprint,
    }
}
pub fn implementation_descriptor(
    value: &semantic_data::plugin::PluginExport,
) -> ImplementationDescriptor {
    ImplementationDescriptor {
        export: value.export.clone(),
        interface: semantic_rpc_core::interface::InterfaceRef {
            package: value.package.clone(),
            module: value.module.clone(),
            contract: value.contract.clone(),
            name: value.interface.clone(),
        },
        package_version: value.package_version.clone(),
        fingerprint: value.fingerprint.clone(),
    }
}

#[derive(Clone, Debug)]
pub struct PluginManifest {
    pub id: String,
    pub revision: String,
    pub title: String,
    pub exports: Vec<ImplementationDescriptor>,
    pub configuration_schema: Option<semantic_data::plugin::PluginConfigurationSchema>,
    pub source_bindings: BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct PluginInstanceContext {
    pub scope: String,
    pub generation: u64,
    pub configuration: Value,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PluginError {
    pub code: String,
    pub message: String,
}
impl PluginError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub trait Plugin: Send + Sync + 'static {
    fn manifest(&self) -> &PluginManifest;
    /// Startup must observe `context.cancellation` and finish its owned cleanup
    /// before returning. Scope close can signal it while creation is pending.
    fn create<'a>(
        &'a self,
        context: PluginInstanceContext,
    ) -> Pin<
        Box<dyn Future<Output = Result<Arc<dyn InterfaceImplementation>, PluginError>> + Send + 'a>,
    >;
}

/// Custom providers retain lifecycle ownership of the started process/session.
pub trait PluginProviderFactory: Send + Sync + 'static {
    fn start<'a>(
        &'a self,
        activation: &'a PluginActivation,
        context: PluginInstanceContext,
    ) -> Pin<Box<dyn Future<Output = Result<StartedPlugin, PluginError>> + Send + 'a>>;
}
pub trait PluginShutdown: Send + Sync + 'static {
    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<(), PluginError>> + Send + '_>>;
    /// Completes when a live provider exits or disconnects without an invocation.
    fn terminated(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(std::future::pending())
    }
}
pub struct StartedPlugin {
    pub implementation: Arc<dyn InterfaceImplementation>,
    pub shutdown: Option<Arc<dyn PluginShutdown>>,
}

#[derive(Default, Clone)]
pub struct PluginRegistry {
    rust: BTreeMap<String, Arc<dyn Plugin>>,
    providers: BTreeMap<String, Arc<dyn PluginProviderFactory>>,
}
impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    #[allow(unused_mut)]
    pub fn with_host_providers(mut self) -> Self {
        #[cfg(feature = "stdio")]
        self.providers
            .insert("stdio".into(), Arc::new(HostProvider));
        #[cfg(feature = "websocket")]
        self.providers
            .insert("websocket".into(), Arc::new(HostProvider));
        self
    }
    pub fn register(&mut self, plugin: impl Plugin) -> Result<(), PluginError> {
        let key = plugin.manifest().id.clone();
        if key.is_empty() || self.rust.contains_key(&key) {
            return Err(PluginError::new("duplicate_plugin", key));
        }
        let mut exports = std::collections::BTreeSet::new();
        for export in &plugin.manifest().exports {
            if export.export.is_empty() || !exports.insert(export.export.clone()) {
                return Err(PluginError::new("duplicate_export", &export.export));
            }
        }
        self.rust.insert(key, Arc::new(plugin));
        Ok(())
    }
    pub fn register_provider(
        &mut self,
        name: impl Into<String>,
        provider: impl PluginProviderFactory,
    ) -> Result<(), PluginError> {
        let name = name.into();
        if self.providers.contains_key(&name) {
            return Err(PluginError::new("duplicate_provider", name));
        }
        self.providers.insert(name, Arc::new(provider));
        Ok(())
    }
    pub fn manifests(&self) -> Vec<PluginManifest> {
        self.rust.values().map(|p| p.manifest().clone()).collect()
    }
}

#[cfg(any(feature = "stdio", feature = "websocket"))]
struct HostProvider;
#[cfg(any(feature = "stdio", feature = "websocket"))]
struct HostShutdown(Mutex<HostShutdownState>, CancellationToken);
#[cfg(any(feature = "stdio", feature = "websocket"))]
struct HostShutdownState {
    connection: Option<semantic_rpc::plugin::ProviderConnection>,
    cleanup: Option<tokio::task::JoinHandle<Result<(), PluginError>>>,
    result: Option<Result<(), PluginError>>,
}
#[cfg(any(feature = "stdio", feature = "websocket"))]
impl PluginShutdown for HostShutdown {
    fn terminated(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.1.cancelled())
    }
    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<(), PluginError>> + Send + '_>> {
        Box::pin(async move {
            let mut state = self.0.lock().await;
            if let Some(result) = &state.result {
                return result.clone();
            }
            if let Some(connection) = state.connection.take() {
                state.cleanup = Some(tokio::spawn(async move {
                    connection
                        .shutdown()
                        .await
                        .map_err(|e| PluginError::new(e.code, e.message))
                }));
            }
            if let Some(cleanup) = &mut state.cleanup {
                let result = cleanup.await.unwrap_or_else(|error| {
                    Err(PluginError::new("cleanup_failed", error.to_string()))
                });
                state.result = Some(result.clone());
                state.cleanup = None;
                return result;
            }
            Ok(())
        })
    }
}
#[cfg(any(feature = "stdio", feature = "websocket"))]
impl PluginProviderFactory for HostProvider {
    fn start<'a>(
        &'a self,
        activation: &'a PluginActivation,
        context: PluginInstanceContext,
    ) -> Pin<Box<dyn Future<Output = Result<StartedPlugin, PluginError>> + Send + 'a>> {
        Box::pin(async move {
            let exports = activation
                .exports
                .iter()
                .map(implementation_descriptor)
                .collect();
            let connection = match &activation.provider {
                #[cfg(feature = "stdio")]
                PluginProvider::Stdio {
                    program,
                    args,
                    cwd,
                    env,
                } => {
                    semantic_rpc::plugin::stdio::connect_with_cancellation(
                        semantic_rpc::plugin::stdio::StdioConfig {
                            program: program.clone(),
                            args: args.clone(),
                            cwd: cwd.as_ref().map(Into::into),
                            env: env.clone(),
                        },
                        exports,
                        Some(activation.revision.clone()),
                        context.configuration,
                        context.cancellation,
                    )
                    .await
                }
                #[cfg(feature = "websocket")]
                PluginProvider::WebSocket { url } => {
                    semantic_rpc::plugin::websocket::connect_with_cancellation(
                        url,
                        exports,
                        Some(activation.revision.clone()),
                        context.configuration,
                        context.cancellation,
                    )
                    .await
                }
                _ => {
                    return Err(PluginError::new(
                        "provider_unavailable",
                        activation.provider.kind(),
                    ));
                }
            }
            .map_err(|e| PluginError::new(e.code, e.message))?;
            let terminated = CancellationToken::new();
            let signal = terminated.clone();
            let closed = connection.closed();
            tokio::spawn(async move {
                closed.await;
                signal.cancel();
            });
            Ok(StartedPlugin {
                implementation: connection.implementation.clone(),
                shutdown: Some(Arc::new(HostShutdown(
                    Mutex::new(HostShutdownState {
                        connection: Some(connection),
                        cleanup: None,
                        result: None,
                    }),
                    terminated,
                ))),
            })
        })
    }
}

struct Generation {
    activation: PluginActivation,
    implementation: Arc<dyn InterfaceImplementation>,
    cancellation: CancellationToken,
    group: JobGroup,
    shutdown: Option<Arc<dyn PluginShutdown>>,
    active: AtomicUsize,
    idle: tokio::sync::Notify,
    health: Arc<RwLock<BTreeMap<String, (PluginState, Option<PluginError>)>>>,
    jobs: ScopeJobs,
    cleanup: std::sync::OnceLock<tokio::sync::watch::Receiver<Option<Result<(), PluginError>>>>,
    failure: std::sync::OnceLock<PluginError>,
}
struct CallGuard(Arc<Generation>);
impl Drop for CallGuard {
    fn drop(&mut self) {
        if self.0.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}
impl Generation {
    fn begin_stop(
        self: &Arc<Self>,
        reason: &str,
    ) -> tokio::sync::watch::Receiver<Option<Result<(), PluginError>>> {
        self.cancellation.cancel();
        self.cleanup
            .get_or_init(|| {
                let (send, receive) = tokio::sync::watch::channel(None);
                let generation = self.clone();
                let reason = reason.to_owned();
                tokio::spawn(async move {
                    let result = generation.cleanup_owned(&reason).await;
                    let _ = send.send(Some(result));
                });
                receive
            })
            .clone()
    }
    async fn stop(self: &Arc<Self>, reason: &str) -> Result<(), PluginError> {
        let mut completion = self.begin_stop(reason);
        loop {
            if let Some(result) = completion.borrow_and_update().clone() {
                return result;
            }
            completion.changed().await.map_err(|_| {
                PluginError::new("cleanup_failed", "Generation cleanup task stopped")
            })?;
        }
    }
    async fn cleanup_owned(&self, reason: &str) -> Result<(), PluginError> {
        let jobs = &self.jobs;
        let persistence_error = {
            let result = match jobs
                .invalidate_group(
                    &self.group,
                    JobError::new(reason, "Plugin generation stopped"),
                )
                .await
            {
                Ok(completion) => completion.wait().await,
                // Scope shutdown invalidates every group together. Its completion
                // is the drain barrier if it won the race with this generation.
                Err(semantic_jobs::JobsError::InvalidGroup | semantic_jobs::JobsError::Closed)
                    if matches!(
                        jobs.health(),
                        semantic_jobs::JobsHealth::ShuttingDown | semantic_jobs::JobsHealth::Closed
                    ) =>
                {
                    jobs.shutdown().await
                }
                Err(error) => Err(error),
            };
            result.err()
        };
        self.wait_idle().await;
        if let Some(shutdown) = &self.shutdown {
            shutdown.shutdown().await?;
        }
        if let Some(error) = persistence_error {
            return Err(PluginError::new("jobs", error.to_string()));
        }
        Ok(())
    }
    async fn report_failure(self: &Arc<Self>, error: &InvocationError) {
        if matches!(
            error.code.as_str(),
            "connection_lost" | "protocol_violation" | "session_closed"
        ) {
            // Never await cleanup here: the active invocation itself owns an
            // idle guard needed by cleanup. The generation owns the task.
            let _ = self
                .failure
                .set(PluginError::new(&error.code, &error.message));
            self.begin_stop(&error.code);
        }
    }
    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active.load(Ordering::Acquire) == 0 {
                break;
            }
            notified.await;
        }
    }
}

#[derive(Clone)]
pub struct PluginBinding {
    generation: Arc<Generation>,
    descriptor: ImplementationDescriptor,
}
impl PluginBinding {
    pub fn source_export(&self) -> Option<&str> {
        self.generation
            .activation
            .source_for_export(&self.descriptor.export)
    }
    pub fn descriptor(&self) -> &ImplementationDescriptor {
        &self.descriptor
    }
    pub fn plugin_id(&self) -> &str {
        &self.generation.activation.id
    }
    pub fn generation(&self) -> u64 {
        self.generation.activation.generation
    }
    pub fn priority(&self) -> Option<i32> {
        self.generation.activation.priority
    }
    pub fn group(&self) -> JobGroup {
        self.generation.group.clone()
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.generation.cancellation.clone()
    }
    pub async fn invoke(
        &self,
        method: impl Into<String>,
        arguments: Vec<InvocationArgument>,
    ) -> Result<InvocationOutput, InvocationError> {
        // The runtime owns the invocation even when its caller abandons the
        // request. Cancellation is cooperative: replacement awaits cleanup.
        struct CancelOnDrop(Option<CancellationToken>);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                if let Some(token) = &self.0 {
                    token.cancel();
                }
            }
        }
        let cancellation = self.cancellation().child_token();
        let mut cancel_on_drop = CancelOnDrop(Some(cancellation.clone()));
        let binding = self.clone();
        let method = method.into();
        // Register before spawning, so replacement cannot pass the idle barrier
        // while the invocation task has not yet been polled.
        self.generation.active.fetch_add(1, Ordering::AcqRel);
        let guard = CallGuard(self.generation.clone());
        let (send, receive) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = binding
                .invoke_owned(method, arguments, cancellation, guard)
                .await;
            let _ = send.send(result);
        });
        let result = receive.await.unwrap_or_else(|_| Err(changed()));
        cancel_on_drop.0 = None;
        result
    }
    async fn invoke_owned(
        &self,
        method: String,
        arguments: Vec<InvocationArgument>,
        cancellation: CancellationToken,
        guard: CallGuard,
    ) -> Result<InvocationOutput, InvocationError> {
        if self.generation.cancellation.is_cancelled() {
            return Err(changed());
        }
        let output = self
            .generation
            .implementation
            .invoke(
                ValidatedInvocation {
                    export: self.descriptor.export.clone(),
                    method,
                    arguments,
                },
                InvocationContext {
                    generation: self.generation(),
                    cancellation: cancellation.clone(),
                },
            )
            .await;
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                self.generation.report_failure(&error).await;
                return Err(error);
            }
        };
        match output {
            InvocationOutput::Values(values) => {
                if cancellation.is_cancelled() {
                    Err(changed())
                } else {
                    Ok(InvocationOutput::Values(values))
                }
            }
            InvocationOutput::Stream(mut stream) => {
                // The producer task owns cleanup, so a caller retaining an idle
                // stream cannot prevent generation cancellation from settling.
                let (demand, mut requests) = tokio::sync::mpsc::channel::<
                    tokio::sync::oneshot::Sender<
                        Option<Result<semantic_rpc::interface::StreamEvent, InvocationError>>,
                    >,
                >(1);
                tokio::spawn(async move {
                    let _guard = guard;
                    loop {
                        let request = tokio::select! { biased; _ = cancellation.cancelled() => break, request = requests.recv() => request };
                        let Some(mut request) = request else {
                            break;
                        };
                        let next = tokio::select! { biased; _ = cancellation.cancelled() => Some(Err(changed())), _ = request.closed() => break, next = stream.next() => next };
                        if let Some(Err(error)) = &next {
                            _guard.0.report_failure(error).await;
                        }
                        let terminal = !matches!(
                            &next,
                            Some(Ok(semantic_rpc::interface::StreamEvent::Item(_)))
                        );
                        if request.send(next).is_err() || terminal {
                            break;
                        }
                    }
                    // Wake a cooperative producer and retain the generation's
                    // barrier until its asynchronous stream cleanup settles.
                    cancellation.cancel();
                    while let Some(event) = stream.next().await {
                        if !matches!(event, Ok(semantic_rpc::interface::StreamEvent::Item(_))) {
                            break;
                        }
                    }
                });
                let stream = futures_util::stream::unfold(demand, |demand| async move {
                    let (reply, receive) = tokio::sync::oneshot::channel();
                    let next = if demand.send(reply).await.is_err() {
                        Some(Err(changed()))
                    } else {
                        receive.await.unwrap_or_else(|_| Some(Err(changed())))
                    };
                    next.map(|next| (next, demand))
                });
                Ok(InvocationOutput::Stream(
                    semantic_rpc::interface::OwnedValueStream::new(stream),
                ))
            }
        }
    }
}
fn changed() -> InvocationError {
    InvocationError {
        code: "plugin_changed".into(),
        message: "Plugin generation is no longer available".into(),
        data: None,
    }
}

/// A serialized lifecycle lane per activation; no registry lock spans provider I/O.
pub struct ScopePlugins {
    scope: String,
    registry: PluginRegistry,
    jobs: ScopeJobs,
    entries: RwLock<BTreeMap<String, Arc<Mutex<PluginEntry>>>>,
    health: Arc<RwLock<BTreeMap<String, (PluginState, Option<PluginError>)>>>,
    closed: AtomicBool,
    cancellation: CancellationToken,
    conform: Option<
        Arc<
            dyn Fn(
                    Arc<dyn InterfaceImplementation>,
                ) -> Result<Arc<dyn InterfaceImplementation>, PluginError>
                + Send
                + Sync,
        >,
    >,
}
struct PluginEntry {
    state: PluginState,
    generation: Option<Arc<Generation>>,
    error: Option<PluginError>,
}
impl ScopePlugins {
    /// Validate before persisting desired state or stopping a live generation.
    pub fn validate_configuration(&self, activation: &PluginActivation) -> Result<(), PluginError> {
        activation
            .validate()
            .map_err(|e| PluginError::new("invalid_configuration", e))?;
        if !activation.enabled {
            return Ok(());
        }
        let schema = match &activation.provider {
            PluginProvider::Rust { key } => {
                let plugin = self
                    .registry
                    .rust
                    .get(key)
                    .ok_or_else(|| PluginError::new("provider_unavailable", key))?;
                if plugin.manifest().configuration_schema != activation.configuration_schema {
                    return Err(PluginError::new(
                        "invalid_configuration",
                        "Installed configuration schema differs from registered manifest",
                    ));
                }
                if plugin.manifest().source_bindings != activation.source_bindings {
                    return Err(PluginError::new(
                        "invalid_configuration",
                        "Installed source bindings differ from registered manifest",
                    ));
                }
                plugin.manifest().configuration_schema.as_ref()
            }
            _ => activation.configuration_schema.as_ref(),
        };
        if let Some(schema) = schema {
            semantic_rpc::interface::validate_configuration(
                &activation.configuration,
                &schema.ty,
                &schema.definitions,
            )
            .map_err(|e| PluginError::new(e.code, e.message))?;
        }
        Ok(())
    }
    pub fn new(scope: impl Into<String>, registry: PluginRegistry, jobs: ScopeJobs) -> Self {
        Self {
            scope: scope.into(),
            registry,
            jobs,
            entries: RwLock::new(BTreeMap::new()),
            health: Arc::new(RwLock::new(BTreeMap::new())),
            closed: AtomicBool::new(false),
            cancellation: CancellationToken::new(),
            conform: None,
        }
    }
    /// The scope owner publishes this token before starting any plugin so close
    /// can cancel cooperative startup without acquiring the activation lock.
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
    pub fn with_conformance(
        mut self,
        conform: impl Fn(
            Arc<dyn InterfaceImplementation>,
        ) -> Result<Arc<dyn InterfaceImplementation>, PluginError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.conform = Some(Arc::new(conform));
        self
    }
    pub async fn activate(&self, activation: PluginActivation) -> Result<(), PluginError> {
        let activation_id = activation.id.clone();
        let entry = {
            let mut entries = self.entries.write().await;
            if self.closed.load(Ordering::Acquire) || self.cancellation.is_cancelled() {
                return Err(PluginError::new("scope_closed", "Plugin runtime is closed"));
            }
            entries
                .entry(activation.id.clone())
                .or_insert_with(|| {
                    Arc::new(Mutex::new(PluginEntry {
                        state: PluginState::Disabled,
                        generation: None,
                        error: None,
                    }))
                })
                .clone()
        };
        let monitor_entry = Arc::downgrade(&entry);
        let mut entry = entry.lock().await;
        if self.closed.load(Ordering::Acquire) || self.cancellation.is_cancelled() {
            return Err(PluginError::new("scope_closed", "Plugin runtime is closed"));
        }
        if let Err(error) = self.validate_configuration(&activation) {
            // Rejected edits preserve a live generation. On reopen there is no
            // generation to preserve, so expose the unavailable installation.
            if entry.generation.is_none() {
                entry.state = PluginState::Unavailable;
                entry.error = Some(error.clone());
                self.health
                    .write()
                    .await
                    .insert(activation_id, (entry.state, Some(error.clone())));
            }
            return Err(error);
        }
        if let Some(old) = &entry.generation {
            if old.activation == activation && !old.cancellation.is_cancelled() {
                return Ok(());
            }
        }
        if let Some(old) = entry.generation.clone() {
            entry.state = PluginState::Stopping;
            self.health
                .write()
                .await
                .insert(activation_id.clone(), (PluginState::Stopping, None));
            old.stop("plugin_changed").await?;
            entry.generation = None;
        }
        if self.cancellation.is_cancelled() {
            return Err(PluginError::new("scope_closed", "Plugin runtime is closed"));
        }
        if !activation.enabled {
            entry.state = PluginState::Disabled;
            self.health
                .write()
                .await
                .insert(activation_id, (PluginState::Disabled, None));
            return Ok(());
        }
        entry.state = PluginState::Starting;
        self.health
            .write()
            .await
            .insert(activation_id.clone(), (PluginState::Starting, None));
        let cancellation = self.cancellation.child_token();
        let context = PluginInstanceContext {
            scope: self.scope.clone(),
            generation: activation.generation,
            configuration: activation.configuration.clone(),
            cancellation: cancellation.clone(),
        };
        let started = match &activation.provider {
            PluginProvider::Rust { key } => match self.registry.rust.get(key) {
                Some(plugin) if plugin.manifest().revision != activation.revision => {
                    Err(PluginError::new(
                        "interface_incompatible",
                        "Registered Rust implementation revision differs from installation",
                    ))
                }
                Some(plugin) => plugin
                    .create(context)
                    .await
                    .map(|implementation| StartedPlugin {
                        implementation,
                        shutdown: None,
                    }),
                None => Err(PluginError::new(
                    "provider_unavailable",
                    format!("Rust plugin {key} is not registered"),
                )),
            },
            provider => match self.registry.providers.get(provider.kind()) {
                Some(factory) => factory.start(&activation, context).await,
                None => Err(PluginError::new("provider_unavailable", provider.kind())),
            },
        };
        let started = match started {
            Ok(mut started) => {
                let expected: Vec<_> = activation
                    .exports
                    .iter()
                    .map(implementation_descriptor)
                    .collect();
                let result = if cancellation.is_cancelled() {
                    Err(PluginError::new("scope_closed", "Plugin runtime is closed"))
                } else if started.implementation.descriptors() != expected {
                    Err(PluginError::new(
                        "interface_incompatible",
                        "Implementation exports differ from installed descriptor",
                    ))
                } else if let Some(conform) = &self.conform {
                    conform(started.implementation.clone())
                } else {
                    Ok(started.implementation.clone())
                };
                match result {
                    Ok(implementation) => {
                        started.implementation = implementation;
                        Ok(started)
                    }
                    Err(mut error) => {
                        cancellation.cancel();
                        if let Some(shutdown) = started.shutdown {
                            if let Err(cleanup) = shutdown.shutdown().await {
                                error = PluginError::new(
                                    error.code,
                                    format!("{}; cleanup: {cleanup}", error.message),
                                );
                            }
                        }
                        Err(error)
                    }
                }
            }
            Err(error) => Err(error),
        };
        match started {
            Ok(started) => {
                let group = match self.jobs.create_group().await {
                    Ok(group) => group,
                    Err(error) => {
                        cancellation.cancel();
                        let mut error = PluginError::new("jobs", error.to_string());
                        if let Some(shutdown) = started.shutdown {
                            if let Err(cleanup) = shutdown.shutdown().await {
                                error.message.push_str(&format!("; cleanup: {cleanup}"));
                            }
                        }
                        entry.state = PluginState::Unavailable;
                        entry.error = Some(error.clone());
                        self.health
                            .write()
                            .await
                            .insert(activation_id, (entry.state, Some(error.clone())));
                        return Err(error);
                    }
                };
                entry.generation = Some(Arc::new(Generation {
                    activation,
                    implementation: started.implementation,
                    cancellation,
                    group,
                    shutdown: started.shutdown,
                    active: AtomicUsize::new(0),
                    idle: tokio::sync::Notify::new(),
                    health: self.health.clone(),
                    jobs: self.jobs.clone(),
                    cleanup: std::sync::OnceLock::new(),
                    failure: std::sync::OnceLock::new(),
                }));
                entry.state = PluginState::Ready;
                entry.error = None;
                self.health
                    .write()
                    .await
                    .insert(activation_id, (PluginState::Ready, None));
                let generation = entry.generation.as_ref().unwrap().clone();
                {
                    let shutdown = generation.shutdown.clone();
                    tokio::spawn(async move {
                        tokio::select! {
                                biased;
                            _ = generation.cancellation.cancelled() => {},
                            _ = async { match &shutdown { Some(shutdown) => shutdown.terminated().await, None => std::future::pending().await } } => {}
                        }
                        // Cancellation may be an invocation failure, explicit stop,
                        // or scope shutdown. Every path joins the same cleanup owner.
                        generation.begin_stop("connection_lost");
                        let Some(entry) = monitor_entry.upgrade() else {
                            return;
                        };
                        let mut entry = entry.lock().await;
                        if !entry
                            .generation
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &generation))
                        {
                            return;
                        }
                        let error = generation.failure.get().cloned().unwrap_or_else(|| {
                            PluginError::new("connection_lost", "Plugin provider terminated")
                        });
                        entry.state = PluginState::Unavailable;
                        entry.error = Some(error.clone());
                        generation
                            .health
                            .write()
                            .await
                            .insert(generation.activation.id.clone(), (entry.state, Some(error)));
                        // Closing admission and draining the generation also cancels
                        // queued/running jobs, even if no plugin call was in flight.
                        let _ = generation.stop("connection_lost").await;
                    });
                }
                Ok(())
            }
            Err(error) => {
                cancellation.cancel();
                entry.state = if error.code == "interface_incompatible" {
                    PluginState::Incompatible
                } else {
                    PluginState::Unavailable
                };
                entry.error = Some(error.clone());
                self.health
                    .write()
                    .await
                    .insert(activation_id, (entry.state, Some(error.clone())));
                Err(error)
            }
        }
    }
    pub async fn bindings(&self) -> Vec<PluginBinding> {
        let entries: Vec<_> = self.entries.read().await.values().cloned().collect();
        let mut bindings = Vec::new();
        for entry in entries {
            let Ok(entry) = entry.try_lock() else {
                continue;
            };
            if let Some(generation) = &entry.generation {
                if !generation.cancellation.is_cancelled() {
                    bindings.extend(generation.implementation.descriptors().iter().cloned().map(
                        |descriptor| PluginBinding {
                            generation: generation.clone(),
                            descriptor,
                        },
                    ));
                }
            }
        }
        bindings
    }
    pub async fn states(&self) -> Vec<(String, PluginState, Option<PluginError>)> {
        self.health
            .read()
            .await
            .iter()
            .map(|(id, (state, error))| (id.clone(), *state, error.clone()))
            .collect()
    }
    pub async fn shutdown(&self) -> Result<(), PluginError> {
        self.closed.store(true, Ordering::Release);
        self.cancellation.cancel();
        let entries: Vec<_> = {
            let entries = self.entries.write().await;
            self.closed.store(true, Ordering::Release);
            entries.values().cloned().collect()
        };
        let results = futures_util::future::join_all(entries.into_iter().map(|entry| async move {
            let mut entry = entry.lock().await;
            if let Some(old) = entry.generation.clone() {
                entry.state = PluginState::Stopping;
                self.health
                    .write()
                    .await
                    .insert(old.activation.id.clone(), (PluginState::Stopping, None));
                old.stop("plugin_disabled").await?;
                entry.generation = None;
                entry.state = PluginState::Disabled;
                self.health
                    .write()
                    .await
                    .insert(old.activation.id.clone(), (PluginState::Disabled, None));
            }
            Ok::<(), PluginError>(())
        }))
        .await;
        for result in results {
            result?;
        }
        Ok(())
    }
}
