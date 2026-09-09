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

#[derive(Default)]
struct ScopeObjectStores {
    entries: BTreeMap<ObjectStoreId, ObjectStoreEntry>,
    default_store: Option<ObjectStoreId>,
}

enum ObjectStoreEntry {
    Request(ObjectStoreOpenRequest),
    Opened(DynObjStore),
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
        scope
            .entries
            .insert(store_id, ObjectStoreEntry::Request(request));
        Ok(())
    }

    pub fn attach_store(
        &self,
        scope_id: DbScopeId,
        store_id: ObjectStoreId,
        store: DynObjStore,
        set_default: bool,
    ) -> Result<(), AppError> {
        let mut state = self.write_state()?;
        let scope = state.scopes.entry(scope_id).or_default();
        if set_default {
            scope.default_store = Some(store_id.clone());
        }
        scope
            .entries
            .insert(store_id, ObjectStoreEntry::Opened(store));
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

    pub fn resolve_store(
        &self,
        scope_id: &DbScopeId,
        store_id: &ObjectStoreId,
    ) -> std::result::Result<DynObjStore, AppError> {
        {
            let state = self.read_state()?;
            let scope = state
                .scopes
                .get(scope_id)
                .ok_or_else(|| AppError::UnknownObjectStoreScope(scope_id.to_string()))?;
            let entry = scope.entries.get(store_id).ok_or_else(|| {
                AppError::UnknownObjectStore(scope_id.to_string(), store_id.to_string())
            })?;
            if let ObjectStoreEntry::Opened(store) = entry {
                return Ok(Arc::clone(store));
            }
        }

        // Recheck and build under the write lock so concurrent callers cannot
        // open the same physical store twice (notably an exclusive logfs file).
        let mut state = self.write_state()?;
        let scope = state
            .scopes
            .get_mut(scope_id)
            .ok_or_else(|| AppError::UnknownObjectStoreScope(scope_id.to_string()))?;
        let entry = scope.entries.get_mut(store_id).ok_or_else(|| {
            AppError::UnknownObjectStore(scope_id.to_string(), store_id.to_string())
        })?;
        let store = match entry {
            ObjectStoreEntry::Opened(store) => return Ok(Arc::clone(store)),
            ObjectStoreEntry::Request(request) => self.builder.build(&request.uri)?,
        };
        *entry = ObjectStoreEntry::Opened(Arc::clone(&store));
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
#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Debug)]
    struct CountingProvider {
        store: DynObjStore,
        opens: Arc<AtomicUsize>,
    }

    impl ObjStoreProvider for CountingProvider {
        type Config = objstore_fs::FsObjStoreConfig;

        fn kind(&self) -> &'static str {
            "counting"
        }
        fn url_scheme(&self) -> &str {
            "counting"
        }
        fn build(&self, _url: &url::Url) -> Result<DynObjStore, objstore::ObjStoreError> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            // Give competing callers time to attempt resolving the same entry.
            std::thread::sleep(std::time::Duration::from_millis(20));
            Ok(Arc::clone(&self.store))
        }
    }

    #[test]
    fn concurrent_resolution_opens_store_once() {
        let path = std::env::temp_dir().join(format!("semantic-store-{}", uuid::Uuid::new_v4()));
        let store: DynObjStore = Arc::new(
            objstore_fs::FsObjStore::new(objstore_fs::FsObjStoreConfig::new(path.clone())).unwrap(),
        );
        let opens = Arc::new(AtomicUsize::new(0));
        let manager = ObjectStoreManager::new(vec![Arc::new(CountingProvider {
            store: Arc::clone(&store),
            opens: Arc::clone(&opens),
        })]);
        let scope = DbScopeId::new("default");
        manager
            .attach_store_request(
                scope.clone(),
                ObjectStoreId::new("default"),
                ObjectStoreOpenRequest {
                    uri: "counting://".into(),
                },
                true,
            )
            .unwrap();
        let start = Barrier::new(8);
        std::thread::scope(|threads| {
            for _ in 0..8 {
                threads.spawn(|| {
                    start.wait();
                    assert!(Arc::ptr_eq(
                        &manager.resolve_default_store(&scope).unwrap(),
                        &store
                    ));
                });
            }
        });
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        drop(manager);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
}
