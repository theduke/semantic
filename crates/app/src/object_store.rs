use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use objstore::{DynObjStore, ObjStoreBuilder, ObjStoreProvider};

use crate::{AppError, DbScopeId};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectStoreId(String);

impl ObjectStoreId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObjectStoreId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ObjectStoreId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ObjectStoreId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug)]
pub struct ObjectStoreOpenRequest {
    pub uri: String,
}

#[derive(Clone, Debug)]
pub struct ObjectStoreInfo {
    pub scope_id: DbScopeId,
    pub store_id: ObjectStoreId,
    pub uri: String,
    pub default: bool,
    pub loaded: bool,
}

#[derive(Default)]
struct ScopeObjectStores {
    entries: BTreeMap<ObjectStoreId, ObjectStoreEntry>,
    default_store: Option<ObjectStoreId>,
}

struct ObjectStoreEntry {
    request: ObjectStoreOpenRequest,
    store: Option<DynObjStore>,
}

#[derive(Default)]
struct ObjectStoreState {
    scopes: BTreeMap<DbScopeId, ScopeObjectStores>,
}

pub struct ObjectStoreManager {
    builder: ObjStoreBuilder,
    state: RwLock<ObjectStoreState>,
}

impl ObjectStoreManager {
    pub fn new(providers: Vec<Arc<dyn ObjStoreProvider>>) -> Self {
        let mut builder = ObjStoreBuilder::new();
        builder.register_provider(objstore_fs::FsProvider::new());
        for provider in providers {
            builder = builder.with_provider(provider);
        }
        Self {
            builder,
            state: RwLock::new(ObjectStoreState::default()),
        }
    }

    pub fn attach_store_request(
        &self,
        scope_id: DbScopeId,
        store_id: ObjectStoreId,
        request: ObjectStoreOpenRequest,
        set_default: bool,
    ) -> std::result::Result<(), AppError> {
        let mut state = self.write_state()?;
        let scope = state.scopes.entry(scope_id).or_default();
        if set_default {
            scope.default_store = Some(store_id.clone());
        }
        scope.entries.insert(
            store_id,
            ObjectStoreEntry {
                request,
                store: None,
            },
        );
        Ok(())
    }

    pub fn attach_store(
        &self,
        scope_id: DbScopeId,
        store_id: ObjectStoreId,
        uri: String,
        store: DynObjStore,
        set_default: bool,
    ) -> std::result::Result<(), AppError> {
        let mut state = self.write_state()?;
        let scope = state.scopes.entry(scope_id).or_default();
        if set_default {
            scope.default_store = Some(store_id.clone());
        }
        scope.entries.insert(
            store_id,
            ObjectStoreEntry {
                request: ObjectStoreOpenRequest { uri },
                store: Some(store),
            },
        );
        Ok(())
    }

    pub fn default_store_id(
        &self,
        scope_id: &DbScopeId,
    ) -> std::result::Result<Option<ObjectStoreId>, AppError> {
        Ok(self
            .read_state()?
            .scopes
            .get(scope_id)
            .and_then(|scope| scope.default_store.clone()))
    }

    pub fn list_stores(
        &self,
        scope_id: &DbScopeId,
    ) -> std::result::Result<Vec<ObjectStoreInfo>, AppError> {
        let state = self.read_state()?;
        let Some(scope) = state.scopes.get(scope_id) else {
            return Ok(Vec::new());
        };
        Ok(scope
            .entries
            .iter()
            .map(|(store_id, entry)| ObjectStoreInfo {
                scope_id: scope_id.clone(),
                store_id: store_id.clone(),
                uri: entry.request.uri.clone(),
                default: scope.default_store.as_ref() == Some(store_id),
                loaded: entry.store.is_some(),
            })
            .collect())
    }

    pub fn resolve_store(
        &self,
        scope_id: &DbScopeId,
        store_id: &ObjectStoreId,
    ) -> std::result::Result<DynObjStore, AppError> {
        let (existing, request) = {
            let state = self.read_state()?;
            let scope = state
                .scopes
                .get(scope_id)
                .ok_or_else(|| AppError::UnknownObjectStoreScope(scope_id.to_string()))?;
            let entry = scope.entries.get(store_id).ok_or_else(|| {
                AppError::UnknownObjectStore(scope_id.to_string(), store_id.to_string())
            })?;
            (entry.store.clone(), entry.request.clone())
        };
        if let Some(store) = existing {
            return Ok(store);
        }

        let store = self.builder.build(&request.uri)?;
        let mut state = self.write_state()?;
        let scope = state
            .scopes
            .get_mut(scope_id)
            .ok_or_else(|| AppError::UnknownObjectStoreScope(scope_id.to_string()))?;
        let entry = scope.entries.get_mut(store_id).ok_or_else(|| {
            AppError::UnknownObjectStore(scope_id.to_string(), store_id.to_string())
        })?;
        entry.store = Some(Arc::clone(&store));
        Ok(store)
    }

    pub fn resolve_default_store(
        &self,
        scope_id: &DbScopeId,
    ) -> std::result::Result<DynObjStore, AppError> {
        let store_id = self
            .default_store_id(scope_id)?
            .ok_or_else(|| AppError::ObjectStoreRequired(scope_id.to_string()))?;
        self.resolve_store(scope_id, &store_id)
    }

    fn read_state(
        &self,
    ) -> std::result::Result<std::sync::RwLockReadGuard<'_, ObjectStoreState>, AppError> {
        self.state
            .read()
            .map_err(|_| AppError::InvalidRequest("object store state lock poisoned".to_string()))
    }

    fn write_state(
        &self,
    ) -> std::result::Result<std::sync::RwLockWriteGuard<'_, ObjectStoreState>, AppError> {
        self.state
            .write()
            .map_err(|_| AppError::InvalidRequest("object store state lock poisoned".to_string()))
    }
}
