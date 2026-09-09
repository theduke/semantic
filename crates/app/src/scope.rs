use std::collections::BTreeMap;
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
}

#[derive(Default)]
struct ScopeState {
    entries: BTreeMap<ScopeKey, ScopeEntry>,
    default_scope: Option<DbScopeId>,
}

pub struct ScopeManager {
    providers: BTreeMap<String, Arc<dyn DbProvider>>,
    idle_ttl: Duration,
    state: RwLock<ScopeState>,
    packages: Vec<semantic_data::schema::Package>,
}

impl ScopeManager {
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
        let existing = {
            let mut state = self.write_state()?;
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.last_used = Instant::now();
                entry.db.clone()
            } else {
                None
            }
        };

        if existing.is_none() {
            let provider = self.provider_for_uri(&options.request.uri)?;
            let db = provider.open(options.request.clone(), principal).await?;
            let mut state = self.write_state()?;
            state.entries.insert(
                key,
                ScopeEntry {
                    request: options.request.clone(),
                    visibility: options.visibility.clone(),
                    db: Some(db),
                    schema_initialized: true,
                    retireable: true,
                    last_used: Instant::now(),
                },
            );
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

        let (existing, request, schema_initialized) = {
            let mut state = self.write_state()?;
            let entry = state
                .entries
                .get_mut(&key)
                .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
            entry.last_used = Instant::now();
            (
                entry.db.clone().map(|db| (db, entry.schema_initialized)),
                entry.request.clone(),
                entry.schema_initialized,
            )
        };

        if let Some((db, schema_initialized)) = existing {
            if !schema_initialized {
                initialize_default_db(&db, &self.packages).await?;
                let mut state = self.write_state()?;
                let entry = state
                    .entries
                    .get_mut(&key)
                    .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
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
            .get_mut(&key)
            .ok_or_else(|| AppError::UnknownScope(scope_id.to_string()))?;
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
        self.write_state()?.entries.remove(&key);
        Ok(())
    }

    pub fn retire_idle_scopes(&self, now: Instant) -> usize {
        let Ok(mut state) = self.state.write() else {
            return 0;
        };
        let mut retired = 0;
        for entry in state.entries.values_mut() {
            if entry.retireable
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
