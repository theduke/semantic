use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::{AppError, DbOpenRequest, DbProvider, Principal, PrincipalId, SemanticDb};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DbScopeId(String);

impl DbScopeId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DbScopeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for DbScopeId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DbScopeId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeVisibility {
    Principal,
    System,
}

#[derive(Clone, Debug)]
pub struct ScopeOpenOptions {
    pub scope_id: Option<DbScopeId>,
    pub request: DbOpenRequest,
    pub visibility: ScopeVisibility,
    pub set_current: bool,
}

#[derive(Clone, Debug)]
pub struct ScopeInfo {
    pub scope_id: DbScopeId,
    pub owner: PrincipalId,
    pub visibility: ScopeVisibility,
    pub uri: String,
    pub loaded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ScopeKey {
    owner: PrincipalId,
    scope_id: DbScopeId,
}

struct ScopeEntry {
    request: DbOpenRequest,
    visibility: ScopeVisibility,
    db: Option<Arc<dyn SemanticDb>>,
    schema_initialized: bool,
    retireable: bool,
    last_used: Instant,
    jobs_close: Arc<tokio::sync::Mutex<()>>,
    plugin_cancellation: semantic_jobs::CancellationToken,
}

#[derive(Default)]
struct ScopeState {
    entries: BTreeMap<ScopeKey, ScopeEntry>,
    default_scope: Option<DbScopeId>,
    jobs: BTreeMap<ScopeKey, Option<semantic_jobs::ScopeJobs>>,
    plugins: BTreeMap<ScopeKey, Arc<crate::plugins::AppScopePlugins>>,
    jobs_closing: bool,
    closing_scopes: BTreeSet<ScopeKey>,
}

pub struct ScopeManager {
    providers: BTreeMap<String, Arc<dyn DbProvider>>,
    idle_ttl: Duration,
    state: RwLock<ScopeState>,
    packages: Vec<semantic_data::schema::Package>,
    jobs_lifecycle: tokio::sync::Mutex<()>,
}

impl ScopeManager {
    pub(crate) async fn update_package(
        &self,
        principal: &Principal,
        scope: &DbScopeId,
        package: semantic_data::schema::Package,
    ) -> Result<semantic_db_core::PackageRegistrationOutcome, AppError> {
        let key = self
            .lookup_key(principal, scope)?
            .ok_or_else(|| AppError::UnknownScope(scope.to_string()))?;
        let close = self
            .read_state()?
            .entries
            .get(&key)
            .ok_or_else(|| AppError::UnknownScope(scope.to_string()))?
            .jobs_close
            .clone();
        // Startup publishes its catalog snapshot and runtime while holding this
        // same lock. Never bypass reconciliation while that publication is pending.
        let _guard = close.lock().await;
        let plugins = {
            let state = self.read_state()?;
            if state.jobs_closing || state.closing_scopes.contains(&key) {
                return Err(semantic_jobs::JobsError::Closed.into());
            }
            if !state
                .entries
                .get(&key)
                .is_some_and(|entry| Arc::ptr_eq(&entry.jobs_close, &close))
            {
                return Err(AppError::UnknownScope(scope.to_string()));
            }
            state.plugins.get(&key).cloned()
        };
        match plugins {
            Some(plugins) => plugins.update_package(package).await,
            None => self
                .resolve_scope_key(principal, &key)
                .await?
                .upsert_package(package)
                .await
                .map_err(AppError::from),
        }
    }
    pub(crate) async fn resolve_plugins(
        &self,
        principal: &Principal,
        scope_id: DbScopeId,
        registry: semantic_plugin::PluginRegistry,
        jobs_registry: semantic_jobs::JobsRegistry,
        jobs_config: semantic_jobs::JobsConfig,
    ) -> Result<Arc<crate::plugins::AppScopePlugins>, AppError> {
        let key = self
            .lookup_key(principal, &scope_id)?
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
        {
            let state = self.read_state()?;
            if state.jobs_closing || state.closing_scopes.contains(&key) {
                return Err(semantic_jobs::JobsError::Closed.into());
            }
        }
        let close = self
            .read_state()?
            .entries
            .get(&key)
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?
            .jobs_close
            .clone();
        let _close_guard = close.lock().await;
        let jobs = self
            .resolve_jobs_key(principal, key.clone(), jobs_registry, jobs_config)
            .await?;
        let _guard = self.jobs_lifecycle.lock().await;
        {
            let state = self.read_state()?;
            if state.jobs_closing || state.closing_scopes.contains(&key) {
                return Err(semantic_jobs::JobsError::Closed.into());
            }
            if let Some(plugins) = state.plugins.get(&key) {
                return Ok(plugins.clone());
            }
        }
        drop(_guard);
        let cancellation = self
            .read_state()?
            .entries
            .get(&key)
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?
            .plugin_cancellation
            .clone();
        let db = self.resolve_scope_key(principal, &key).await?;
        let plugins = Arc::new(
            crate::plugins::AppScopePlugins::open_with_cancellation(
                scope_id.to_string(),
                registry,
                jobs,
                db,
                cancellation,
            )
            .await?,
        );
        let mut state = self.write_state()?;
        state.plugins.insert(key.clone(), plugins.clone());
        if state.jobs_closing || state.closing_scopes.contains(&key) {
            return Err(semantic_jobs::JobsError::Closed.into());
        }
        Ok(plugins)
    }
    pub fn new(providers: BTreeMap<String, Arc<dyn DbProvider>>, idle_ttl: Duration) -> Self {
        Self::with_packages(providers, idle_ttl, default_packages())
    }

    pub(crate) fn with_packages(
        providers: BTreeMap<String, Arc<dyn DbProvider>>,
        idle_ttl: Duration,
        packages: Vec<semantic_data::schema::Package>,
    ) -> Self {
        Self {
            packages,
            providers,
            idle_ttl,
            state: RwLock::new(ScopeState::default()),
            jobs_lifecycle: tokio::sync::Mutex::new(()),
        }
    }

    pub fn add_default_scope(
        &self,
        scope_id: DbScopeId,
        db: Arc<dyn SemanticDb>,
    ) -> std::result::Result<(), AppError> {
        let key = ScopeKey {
            owner: Principal::system().id,
            scope_id: scope_id.clone(),
        };
        let mut state = self.write_state()?;
        if state.jobs.contains_key(&key) {
            return Err(AppError::InvalidRequest(
                "close the jobs-enabled scope before replacing its database".into(),
            ));
        }
        state.default_scope = Some(scope_id.clone());
        state.entries.insert(
            key,
            ScopeEntry {
                request: DbOpenRequest {
                    uri: format!("default://{}", scope_id.as_str()),
                    mode: semantic_data::schema::DbOpenMode::AutoCreate,
                },
                visibility: ScopeVisibility::System,
                db: Some(db),
                schema_initialized: false,
                retireable: false,
                last_used: Instant::now(),
                jobs_close: Arc::default(),
                plugin_cancellation: semantic_jobs::CancellationToken::new(),
            },
        );
        Ok(())
    }

    pub fn add_default_scope_request(
        &self,
        scope_id: DbScopeId,
        request: DbOpenRequest,
    ) -> std::result::Result<(), AppError> {
        let key = ScopeKey {
            owner: Principal::system().id,
            scope_id: scope_id.clone(),
        };
        let mut state = self.write_state()?;
        if state.jobs.contains_key(&key) {
            return Err(AppError::InvalidRequest(
                "close the jobs-enabled scope before replacing its database".into(),
            ));
        }
        state.default_scope = Some(scope_id);
        state.entries.insert(
            key,
            ScopeEntry {
                request,
                visibility: ScopeVisibility::System,
                db: None,
                schema_initialized: false,
                retireable: false,
                last_used: Instant::now(),
                jobs_close: Arc::default(),
                plugin_cancellation: semantic_jobs::CancellationToken::new(),
            },
        );
        Ok(())
    }

    pub fn default_scope(&self) -> Option<DbScopeId> {
        self.state
            .read()
            .ok()
            .and_then(|state| state.default_scope.clone())
    }

    pub async fn open_scope(
        &self,
        principal: &Principal,
        options: ScopeOpenOptions,
    ) -> std::result::Result<ScopeInfo, AppError> {
        let scope_id = options.scope_id.unwrap_or_else(|| {
            DbScopeId::new(
                options
                    .request
                    .uri
                    .rsplit_once('/')
                    .map(|(_, tail)| tail)
                    .filter(|tail| !tail.is_empty())
                    .unwrap_or(options.request.uri.as_str()),
            )
        });
        let owner = owner_for(principal, &options.visibility);
        let key = ScopeKey {
            owner: owner.clone(),
            scope_id: scope_id.clone(),
        };
        let (existing, identity) = {
            let mut state = self.write_state()?;
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.last_used = Instant::now();
                (entry.db.clone(), Some(entry.jobs_close.clone()))
            } else {
                (None, None)
            }
        };

        if existing.is_none() {
            let provider = self.provider_for_uri(&options.request.uri)?;
            let db = provider.open(options.request.clone(), principal).await?;
            let mut state = self.write_state()?;
            match state.entries.entry(key) {
                std::collections::btree_map::Entry::Occupied(mut occupied) => {
                    let entry = occupied.get_mut();
                    if identity
                        .as_ref()
                        .is_some_and(|identity| !Arc::ptr_eq(identity, &entry.jobs_close))
                    {
                        return Err(AppError::UnknownScope(scope_id.to_string()));
                    }
                    // A concurrent resolver may already have attached a coordinator
                    // to this database. Never replace that resolver's winner.
                    if entry.db.is_none() {
                        entry.db = Some(db);
                        entry.schema_initialized = true;
                    }
                }
                std::collections::btree_map::Entry::Vacant(vacant) => {
                    if identity.is_some() {
                        return Err(AppError::UnknownScope(scope_id.to_string()));
                    }
                    vacant.insert(ScopeEntry {
                        request: options.request.clone(),
                        visibility: options.visibility.clone(),
                        db: Some(db),
                        schema_initialized: true,
                        retireable: true,
                        last_used: Instant::now(),
                        jobs_close: Arc::default(),
                        plugin_cancellation: semantic_jobs::CancellationToken::new(),
                    });
                }
            }
        }

        Ok(ScopeInfo {
            scope_id,
            owner,
            visibility: options.visibility,
            uri: options.request.uri,
            loaded: true,
        })
    }

    pub async fn resolve_scope(
        &self,
        principal: &Principal,
        requested_scope: Option<DbScopeId>,
        session_default: Option<DbScopeId>,
    ) -> std::result::Result<Arc<dyn SemanticDb>, AppError> {
        let scope_id = requested_scope
            .or(session_default)
            .or_else(|| self.default_scope())
            .ok_or(AppError::ScopeRequired)?;

        let key = self
            .lookup_key(principal, &scope_id)?
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;

        self.resolve_scope_key(principal, &key).await
    }

    async fn resolve_scope_key(
        &self,
        principal: &Principal,
        key: &ScopeKey,
    ) -> Result<Arc<dyn SemanticDb>, AppError> {
        let scope_id = &key.scope_id;

        let (existing, request, schema_initialized, identity) = {
            let mut state = self.write_state()?;
            let entry = state
                .entries
                .get_mut(key)
                .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
            entry.last_used = Instant::now();
            (
                entry.db.clone().map(|db| (db, entry.schema_initialized)),
                entry.request.clone(),
                entry.schema_initialized,
                entry.jobs_close.clone(),
            )
        };

        if let Some((db, schema_initialized)) = existing {
            if !schema_initialized {
                initialize_default_db(&db, &self.packages).await?;
                let mut state = self.write_state()?;
                let entry = state
                    .entries
                    .get_mut(key)
                    .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
                if !Arc::ptr_eq(&identity, &entry.jobs_close)
                    || !entry
                        .db
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &db))
                {
                    return Err(AppError::UnknownScope(scope_id.to_string()));
                }
                entry.schema_initialized = true;
                entry.last_used = Instant::now();
            }
            return Ok(db);
        }

        let provider = self.provider_for_uri(&request.uri)?;
        let db = provider.open(request, principal).await?;
        if !schema_initialized {
            initialize_default_db(&db, &self.packages).await?;
        }
        let mut state = self.write_state()?;
        let entry = state
            .entries
            .get_mut(key)
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
        if !Arc::ptr_eq(&identity, &entry.jobs_close) {
            return Err(AppError::UnknownScope(scope_id.to_string()));
        }
        if let Some(existing) = &entry.db {
            return Ok(existing.clone());
        }
        entry.db = Some(Arc::clone(&db));
        entry.schema_initialized = true;
        entry.last_used = Instant::now();
        Ok(db)
    }

    pub fn list_scopes(&self, principal: &Principal) -> Vec<ScopeInfo> {
        let Ok(state) = self.state.read() else {
            return Vec::new();
        };
        state
            .entries
            .iter()
            .filter(|(key, entry)| {
                key.owner == principal.id || entry.visibility == ScopeVisibility::System
            })
            .map(|(key, entry)| ScopeInfo {
                scope_id: key.scope_id.clone(),
                owner: key.owner.clone(),
                visibility: entry.visibility.clone(),
                uri: entry.request.uri.clone(),
                loaded: entry.db.is_some(),
            })
            .collect()
    }

    pub fn close_scope(
        &self,
        principal: &Principal,
        scope_id: &DbScopeId,
    ) -> std::result::Result<(), AppError> {
        let key = self
            .lookup_key(principal, scope_id)?
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
        let mut state = self.write_state()?;
        if state.jobs.contains_key(&key) {
            return Err(AppError::InvalidRequest(
                "scope has a jobs coordinator; use close_scope_with_jobs".into(),
            ));
        }
        state.entries.remove(&key);
        Ok(())
    }

    pub fn retire_idle_scopes(&self, now: Instant) -> usize {
        let Ok(mut state) = self.state.write() else {
            return 0;
        };
        let mut retired = 0;
        let job_keys: Vec<_> = state.jobs.keys().cloned().collect();
        for (key, entry) in state.entries.iter_mut() {
            if entry.retireable
                && !job_keys.contains(key)
                && entry.db.is_some()
                && now
                    .checked_duration_since(entry.last_used)
                    .is_some_and(|idle| idle >= self.idle_ttl)
            {
                entry.db = None;
                retired += 1;
            }
        }
        retired
    }

    /// Resolve exactly one native coordinator for the actual owner/scope key.
    /// Attached coordinators pin their DB until explicit asynchronous close.
    pub async fn resolve_jobs(
        &self,
        principal: &Principal,
        scope_id: DbScopeId,
        registry: semantic_jobs::JobsRegistry,
        config: semantic_jobs::JobsConfig,
    ) -> Result<semantic_jobs::ScopeJobs, AppError> {
        let key = self
            .lookup_key(principal, &scope_id)?
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
        self.resolve_jobs_key(principal, key, registry, config)
            .await
    }

    async fn resolve_jobs_key(
        &self,
        principal: &Principal,
        key: ScopeKey,
        registry: semantic_jobs::JobsRegistry,
        config: semantic_jobs::JobsConfig,
    ) -> Result<semantic_jobs::ScopeJobs, AppError> {
        let scope_id = key.scope_id.clone();
        // Common control calls do not wait for another scope's initialization or
        // cooperative shutdown. Closing scopes cannot create replacement owners.
        {
            let state = self.read_state()?;
            if state.jobs_closing || state.closing_scopes.contains(&key) {
                return Err(semantic_jobs::JobsError::Closed.into());
            }
            if let Some(Some(jobs)) = state.jobs.get(&key) {
                return Ok(jobs.clone());
            }
        }
        let _guard = self.jobs_lifecycle.lock().await;
        {
            let mut state = self.write_state()?;
            if state.jobs_closing || state.closing_scopes.contains(&key) {
                return Err(semantic_jobs::JobsError::Closed.into());
            }
            if !state.entries.contains_key(&key) {
                return Err(AppError::UnknownScope(scope_id.to_string()));
            }
            if let Some(Some(jobs)) = state.jobs.get(&key) {
                return Ok(jobs.clone());
            }
            state.jobs.insert(key.clone(), None);
        }
        let result = async {
            let db = self.resolve_scope_key(principal, &key).await?;
            semantic_jobs::ScopeJobs::open(
                Arc::new(crate::jobs::DbJobStore::new(db)),
                registry,
                config,
            )
            .await
            .map_err(AppError::from)
        }
        .await;
        let mut state = self.write_state()?;
        match result {
            Ok(jobs) => {
                state.jobs.insert(key.clone(), Some(jobs.clone()));
                if state.jobs_closing || state.closing_scopes.contains(&key) {
                    // The shutdown snapshot waits for this initialization lock.
                    Err(semantic_jobs::JobsError::Closed.into())
                } else {
                    Ok(jobs)
                }
            }
            Err(error) => {
                state.jobs.remove(&key);
                Err(error)
            }
        }
    }

    pub async fn close_scope_with_jobs(
        &self,
        principal: &Principal,
        scope_id: &DbScopeId,
    ) -> Result<(), AppError> {
        let key = self
            .lookup_key(principal, scope_id)?
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
        let close = self
            .read_state()?
            .entries
            .get(&key)
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?
            .jobs_close
            .clone();
        // Publish closure and signal startup before the initialization/cleanup
        // lock. A Plugin::create may itself be waiting for this cancellation.
        let initial_jobs = {
            let mut state = self.write_state()?;
            let entry = state
                .entries
                .get(&key)
                .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
            if !Arc::ptr_eq(&entry.jobs_close, &close) {
                return Err(AppError::UnknownScope(scope_id.to_string()));
            }
            entry.plugin_cancellation.cancel();
            state.closing_scopes.insert(key.clone());
            state.jobs.get(&key).cloned().flatten()
        };
        let had_jobs = initial_jobs.is_some();
        let (jobs_result, close_result) = futures_util::future::join(
            async {
                match initial_jobs {
                    Some(jobs) => jobs.shutdown().await.map_err(AppError::from),
                    None => Ok(()),
                }
            },
            self.finish_scope_close(key.clone(), scope_id, close, had_jobs),
        )
        .await;
        let _close_guard = close_result?;
        jobs_result?;
        let mut state = self.write_state()?;
        state.jobs.remove(&key);
        state.plugins.remove(&key);
        state.entries.remove(&key);
        state.closing_scopes.remove(&key);
        Ok(())
    }

    async fn finish_scope_close(
        &self,
        key: ScopeKey,
        scope_id: &DbScopeId,
        close: Arc<tokio::sync::Mutex<()>>,
        had_jobs: bool,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, AppError> {
        let close_guard = close.clone().lock_owned().await;
        let _guard = self.jobs_lifecycle.lock().await;
        // Another close may have finished while this call waited. Never remove
        // a replacement entry installed under the same owner/key in the meantime.
        if !self
            .read_state()?
            .entries
            .get(&key)
            .is_some_and(|entry| Arc::ptr_eq(&entry.jobs_close, &close))
        {
            return Err(AppError::UnknownScope(scope_id.to_string()));
        }
        let jobs = if had_jobs {
            None
        } else {
            self.read_state()?.jobs.get(&key).cloned().flatten()
        };
        let plugins = self.read_state()?.plugins.get(&key).cloned();
        self.write_state()?.closing_scopes.insert(key.clone());
        drop(_guard);
        let (plugin_result, jobs_result) = futures_util::future::join(
            async {
                match plugins {
                    Some(plugins) => plugins
                        .runtime
                        .shutdown()
                        .await
                        .map_err(crate::plugins::error),
                    None => Ok(()),
                }
            },
            async {
                match jobs {
                    Some(jobs) => jobs.shutdown().await.map_err(AppError::from),
                    None => Ok(()),
                }
            },
        )
        .await;
        plugin_result?;
        jobs_result?;
        Ok(close_guard)
    }

    pub async fn shutdown_jobs(&self) -> Result<(), AppError> {
        // Publish before waiting for initialization, so concurrent and future
        // lookups cannot admit a new coordinator after the shutdown snapshot.
        let initial_jobs = {
            let mut state = self.write_state()?;
            state.jobs_closing = true;
            for entry in state.entries.values() {
                entry.plugin_cancellation.cancel();
            }
            state
                .jobs
                .iter()
                .filter_map(|(key, jobs)| jobs.clone().map(|jobs| (key.clone(), jobs)))
                .collect::<BTreeMap<_, _>>()
        };
        let (initial_results, remaining_result) = futures_util::future::join(
            futures_util::future::join_all(initial_jobs.values().map(|jobs| jobs.shutdown())),
            self.finish_jobs_shutdown(&initial_jobs),
        )
        .await;
        remaining_result?;
        for result in initial_results {
            result?;
        }
        Ok(())
    }

    async fn finish_jobs_shutdown(
        &self,
        initial_jobs: &BTreeMap<ScopeKey, semantic_jobs::ScopeJobs>,
    ) -> Result<(), AppError> {
        let _guard = self.jobs_lifecycle.lock().await;
        let jobs: Vec<_> = self
            .read_state()?
            .jobs
            .iter()
            .filter(|(key, _)| !initial_jobs.contains_key(*key))
            .filter_map(|(_, jobs)| jobs.clone())
            .collect();
        let plugin_initializations: Vec<_> = self
            .read_state()?
            .entries
            .values()
            .map(|entry| entry.jobs_close.clone())
            .collect();
        drop(_guard);
        let _plugin_guards =
            futures_util::future::join_all(plugin_initializations.iter().map(|lock| lock.lock()))
                .await;
        let plugins: Vec<_> = self.read_state()?.plugins.values().cloned().collect();
        // Poll every plugin and coordinator stop before waiting for any cleanup.
        let (plugin_results, results) = futures_util::future::join(
            futures_util::future::join_all(
                plugins.iter().map(|plugins| plugins.runtime.shutdown()),
            ),
            futures_util::future::join_all(jobs.iter().map(|jobs| jobs.shutdown())),
        )
        .await;
        for result in plugin_results {
            result.map_err(crate::plugins::error)?;
        }
        for result in results {
            result?;
        }
        Ok(())
    }

    fn lookup_key(
        &self,
        principal: &Principal,
        scope_id: &DbScopeId,
    ) -> std::result::Result<Option<ScopeKey>, AppError> {
        let state = self.read_state()?;
        let principal_key = ScopeKey {
            owner: principal.id.clone(),
            scope_id: scope_id.clone(),
        };
        if state.entries.contains_key(&principal_key) {
            return Ok(Some(principal_key));
        }
        let system_key = ScopeKey {
            owner: Principal::system().id,
            scope_id: scope_id.clone(),
        };
        if state
            .entries
            .get(&system_key)
            .is_some_and(|entry| entry.visibility == ScopeVisibility::System)
        {
            return Ok(Some(system_key));
        }
        Ok(None)
    }

    fn provider_for_uri(&self, uri: &str) -> std::result::Result<Arc<dyn DbProvider>, AppError> {
        let scheme = uri
            .split_once(':')
            .map(|(scheme, _)| scheme)
            .ok_or_else(|| AppError::InvalidRequest(format!("invalid database uri '{uri}'")))?;
        self.providers
            .get(scheme)
            .cloned()
            .ok_or_else(|| AppError::UnsupportedDbScheme(scheme.to_string()))
    }

    fn read_state(
        &self,
    ) -> std::result::Result<std::sync::RwLockReadGuard<'_, ScopeState>, AppError> {
        self.state
            .read()
            .map_err(|_| AppError::InvalidRequest("scope state lock poisoned".to_string()))
    }

    fn write_state(
        &self,
    ) -> std::result::Result<std::sync::RwLockWriteGuard<'_, ScopeState>, AppError> {
        self.state
            .write()
            .map_err(|_| AppError::InvalidRequest("scope state lock poisoned".to_string()))
    }
}

fn owner_for(principal: &Principal, visibility: &ScopeVisibility) -> PrincipalId {
    match visibility {
        ScopeVisibility::Principal => principal.id.clone(),
        ScopeVisibility::System => Principal::system().id,
    }
}

pub(crate) fn default_packages() -> Vec<semantic_data::schema::Package> {
    vec![
        #[cfg(feature = "base")]
        <semantic_base::BasePackage as semantic_rpc_core::RuntimePackage<
            crate::AppRequestContext,
        >>::schema(&semantic_base::BasePackage),
        semantic_data::filestore::package(),
    ]
}

async fn initialize_default_db(
    db: &Arc<dyn SemanticDb>,
    packages: &[semantic_data::schema::Package],
) -> Result<(), AppError> {
    for package in packages {
        db.upsert_package(package.clone()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod jobs_lifecycle_tests {
    use super::*;
    use semantic_data::schema::DbOpenMode;

    fn db(path: &std::path::Path) -> Arc<dyn SemanticDb> {
        Arc::new(semantic_db_core::Db::new(
            semantic_db_redb::open_backend(path, DbOpenMode::AutoCreate).unwrap(),
        ))
    }

    #[tokio::test]
    async fn shadowing_while_waiting_keeps_captured_scope_owner() {
        let dir = tempfile::tempdir().unwrap();
        let system_db = db(&dir.path().join("system.redb"));
        let private_db = db(&dir.path().join("private.redb"));
        let manager = ScopeManager::with_packages(BTreeMap::new(), Duration::from_secs(60), vec![]);
        manager
            .add_default_scope("same".into(), system_db.clone())
            .unwrap();
        let user = Principal::user("alice");
        let guard = manager.jobs_lifecycle.lock().await;
        let mut lookup = Box::pin(manager.resolve_jobs(
            &user,
            "same".into(),
            Default::default(),
            Default::default(),
        ));
        assert!(futures_util::poll!(&mut lookup).is_pending());
        // Install a private scope after lookup selected the visible system key,
        // but before the coordinator can initialize its store.
        manager.write_state().unwrap().entries.insert(
            ScopeKey {
                owner: user.id.clone(),
                scope_id: "same".into(),
            },
            ScopeEntry {
                request: DbOpenRequest {
                    uri: "default://private".into(),
                    mode: DbOpenMode::AutoCreate,
                },
                visibility: ScopeVisibility::Principal,
                db: Some(private_db.clone()),
                schema_initialized: true,
                retireable: false,
                last_used: Instant::now(),
                jobs_close: Arc::default(),
                plugin_cancellation: semantic_jobs::CancellationToken::new(),
            },
        );
        drop(guard);
        let jobs = lookup.await.unwrap();
        // Only the captured system database received the jobs package.
        use semantic_jobs::JobStore;
        assert_eq!(
            crate::jobs::DbJobStore::new(system_db)
                .count()
                .await
                .unwrap(),
            0
        );
        assert!(
            crate::jobs::DbJobStore::new(private_db)
                .count()
                .await
                .is_err()
        );
        jobs.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_gates_waiting_and_future_coordinator_initialization() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ScopeManager::with_packages(BTreeMap::new(), Duration::from_secs(60), vec![]);
        manager
            .add_default_scope("test".into(), db(&dir.path().join("scope.redb")))
            .unwrap();
        let principal = Principal::system();
        let guard = manager.jobs_lifecycle.lock().await;
        let mut lookup = Box::pin(manager.resolve_jobs(
            &principal,
            "test".into(),
            Default::default(),
            Default::default(),
        ));
        assert!(futures_util::poll!(&mut lookup).is_pending());
        let mut shutdown = Box::pin(manager.shutdown_jobs());
        assert!(futures_util::poll!(&mut shutdown).is_pending());
        assert!(manager.read_state().unwrap().jobs_closing);
        drop(guard);
        assert!(lookup.await.is_err());
        shutdown.await.unwrap();
        assert!(
            manager
                .resolve_jobs(
                    &principal,
                    "test".into(),
                    Default::default(),
                    Default::default()
                )
                .await
                .is_err()
        );
        assert!(manager.read_state().unwrap().jobs.is_empty());
    }

    struct DelayedProvider {
        db: Arc<dyn SemanticDb>,
        release: Arc<tokio::sync::Semaphore>,
    }

    struct FirstOpenDelayedProvider {
        first: Arc<dyn SemanticDb>,
        next: Arc<dyn SemanticDb>,
        calls: std::sync::atomic::AtomicUsize,
        release: tokio::sync::Semaphore,
    }

    #[async_trait::async_trait]
    impl DbProvider for FirstOpenDelayedProvider {
        fn scheme(&self) -> &str {
            "gated"
        }
        async fn open(
            &self,
            _: DbOpenRequest,
            _: &Principal,
        ) -> Result<Arc<dyn SemanticDb>, AppError> {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                self.release.acquire().await.unwrap().forget();
                Ok(self.first.clone())
            } else {
                Ok(self.next.clone())
            }
        }
    }

    #[tokio::test]
    async fn delayed_open_preserves_database_attached_to_coordinator() {
        for explicit_open in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let provider = Arc::new(FirstOpenDelayedProvider {
                first: db(&dir.path().join("delayed.redb")),
                next: db(&dir.path().join("winner.redb")),
                calls: std::sync::atomic::AtomicUsize::new(0),
                release: tokio::sync::Semaphore::new(0),
            });
            let manager = ScopeManager::with_packages(
                BTreeMap::from([("gated".into(), provider.clone() as Arc<dyn DbProvider>)]),
                Duration::from_secs(60),
                vec![],
            );
            let request = DbOpenRequest {
                uri: "gated://test".into(),
                mode: DbOpenMode::AutoCreate,
            };
            manager
                .add_default_scope_request("test".into(), request.clone())
                .unwrap();
            let principal = Principal::system();
            let mut delayed = Box::pin(async {
                if explicit_open {
                    manager
                        .open_scope(
                            &principal,
                            ScopeOpenOptions {
                                scope_id: Some("test".into()),
                                request,
                                visibility: ScopeVisibility::System,
                                set_current: false,
                            },
                        )
                        .await
                        .unwrap();
                } else {
                    let resolved = manager
                        .resolve_scope(&principal, Some("test".into()), None)
                        .await
                        .unwrap();
                    assert!(Arc::ptr_eq(&resolved, &provider.next));
                }
            });
            assert!(futures_util::poll!(&mut delayed).is_pending());
            let jobs = manager
                .resolve_jobs(
                    &principal,
                    "test".into(),
                    Default::default(),
                    Default::default(),
                )
                .await
                .unwrap();
            provider.release.add_permits(1);
            delayed.await;
            let resolved = manager
                .resolve_scope(&principal, Some("test".into()), None)
                .await
                .unwrap();
            assert!(Arc::ptr_eq(&resolved, &provider.next));
            assert!(
                manager
                    .add_default_scope("test".into(), provider.first.clone())
                    .is_err()
            );
            jobs.shutdown().await.unwrap();
        }
    }

    #[async_trait::async_trait]
    impl DbProvider for DelayedProvider {
        fn scheme(&self) -> &str {
            "delayed"
        }
        async fn open(
            &self,
            _: DbOpenRequest,
            _: &Principal,
        ) -> Result<Arc<dyn SemanticDb>, AppError> {
            self.release.acquire().await.unwrap().forget();
            Ok(self.db.clone())
        }
    }

    #[tokio::test]
    async fn shutdown_includes_coordinator_already_initializing() {
        let dir = tempfile::tempdir().unwrap();
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let provider: Arc<dyn DbProvider> = Arc::new(DelayedProvider {
            db: db(&dir.path().join("scope.redb")),
            release: release.clone(),
        });
        let manager = ScopeManager::with_packages(
            BTreeMap::from([("delayed".into(), provider)]),
            Duration::from_secs(60),
            vec![],
        );
        manager
            .add_default_scope_request(
                "test".into(),
                DbOpenRequest {
                    uri: "delayed://test".into(),
                    mode: DbOpenMode::AutoCreate,
                },
            )
            .unwrap();
        let principal = Principal::system();
        let mut lookup = Box::pin(manager.resolve_jobs(
            &principal,
            "test".into(),
            Default::default(),
            Default::default(),
        ));
        assert!(futures_util::poll!(&mut lookup).is_pending());
        assert_eq!(manager.read_state().unwrap().jobs.len(), 1);
        let mut shutdown = Box::pin(manager.shutdown_jobs());
        assert!(futures_util::poll!(&mut shutdown).is_pending());
        release.add_permits(1);
        assert!(lookup.await.is_err());
        shutdown.await.unwrap();
        let state = manager.read_state().unwrap();
        assert!(
            state
                .jobs
                .values()
                .all(|jobs| jobs.as_ref().unwrap().health() == semantic_jobs::JobsHealth::Closed)
        );
    }

    struct WaitHandler(semantic_jobs::JobKindDescriptor);
    impl semantic_jobs::JobHandler for WaitHandler {
        type Input = (
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        );
        type Output = ();
        fn kind(&self) -> &semantic_jobs::JobKindDescriptor {
            &self.0
        }
        fn run<'a>(
            &'a self,
            input: Self::Input,
            _: semantic_jobs::JobContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), semantic_jobs::JobError>> + Send + 'a>,
        > {
            Box::pin(async move {
                input.0.send(()).unwrap();
                input.1.await.unwrap();
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn scope_close_does_not_block_other_scopes_or_remove_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ScopeManager::with_packages(BTreeMap::new(), Duration::from_secs(60), vec![]);
        manager
            .add_default_scope("one".into(), db(&dir.path().join("one.redb")))
            .unwrap();
        manager
            .add_default_scope("two".into(), db(&dir.path().join("two.redb")))
            .unwrap();
        let principal = Principal::system();
        let mut builder = semantic_jobs::JobsBuilder::new();
        let handler = builder
            .register(WaitHandler(semantic_jobs::JobKindDescriptor {
                id: semantic_jobs::JobKindId("test.wait".into()),
                title: "Wait".into(),
                description: None,
            }))
            .unwrap();
        let jobs = manager
            .resolve_jobs(
                &principal,
                "one".into(),
                builder.build(),
                Default::default(),
            )
            .await
            .unwrap();
        let (started, start) = tokio::sync::oneshot::channel();
        let (finish, end) = tokio::sync::oneshot::channel();
        let ticket = jobs
            .submit(&handler, (started, end), Default::default())
            .await
            .unwrap();
        start.await.unwrap();
        let scope = DbScopeId::from("one");
        let mut first_close = Box::pin(manager.close_scope_with_jobs(&principal, &scope));
        assert!(futures_util::poll!(&mut first_close).is_pending());
        let mut second_close = Box::pin(manager.close_scope_with_jobs(&principal, &scope));
        assert!(futures_util::poll!(&mut second_close).is_pending());
        let other = tokio::time::timeout(
            Duration::from_secs(5),
            manager.resolve_jobs(
                &principal,
                "two".into(),
                Default::default(),
                Default::default(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            manager
                .resolve_jobs(
                    &principal,
                    scope.clone(),
                    Default::default(),
                    Default::default()
                )
                .await
                .is_err()
        );
        finish.send(()).unwrap();
        first_close.await.unwrap();
        assert!(ticket.wait().await.is_err());
        manager
            .add_default_scope(scope.clone(), db(&dir.path().join("replacement.redb")))
            .unwrap();
        assert!(second_close.await.is_err());
        assert!(manager.lookup_key(&principal, &scope).unwrap().is_some());
        other.shutdown().await.unwrap();
    }
}
