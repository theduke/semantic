use std::sync::{Arc, RwLock};

use crate::catalog::{Catalog, CollectionKind, LocalCollectionId};
use crate::{
    AsyncRuntime, Backend, Batch, BatchOutcome, DbError, DdlBatch, DdlOutcome, DeleteQuery,
    EntityRecord, MutationStats, PackageRegistrationOutcome, Query, QueryExplain, QueryPlan,
    QueryResult, TextQueryInput, UpdateQuery, spawn_blocking_on,
};
use async_trait::async_trait;
use semantic_data::schema::{Package, RelationType};
use semantic_data::value::Object;

use crate::embedded::{EmbeddedDb, EntityStorage};

pub struct EmbeddedBackend<S: EntityStorage> {
    db: Arc<RwLock<EmbeddedDb<S>>>,
    runtime: Arc<dyn AsyncRuntime>,
}

impl<S: EntityStorage> EmbeddedBackend<S> {
    pub fn new(db: EmbeddedDb<S>) -> Self {
        Self::with_runtime(db, default_runtime())
    }

    pub fn with_runtime(db: EmbeddedDb<S>, runtime: Arc<dyn AsyncRuntime>) -> Self {
        Self {
            db: Arc::new(RwLock::new(db)),
            runtime,
        }
    }
}

#[cfg(feature = "tokio")]
fn default_runtime() -> Arc<dyn AsyncRuntime> {
    Arc::new(crate::TokioAsyncRuntime)
}

#[cfg(not(feature = "tokio"))]
fn default_runtime() -> Arc<dyn AsyncRuntime> {
    Arc::new(crate::InlineAsyncRuntime)
}

fn lock_poisoned_error() -> DbError {
    DbError::Storage("embedded backend rwlock poisoned".to_string())
}

#[async_trait]
impl<S: EntityStorage> Backend for EmbeddedBackend<S> {
    async fn validation_preflight(&self) -> Result<Vec<crate::ValidationViolation>, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.read()
                .map_err(|_| lock_poisoned_error())?
                .validation_preflight()
        })
        .await
    }

    async fn activate_validation(&self) -> Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.write()
                .map_err(|_| lock_poisoned_error())?
                .activate_validation()
        })
        .await
    }
    async fn catalog(&self) -> std::result::Result<Arc<Catalog>, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            Ok(db.catalog())
        })
        .await
    }

    async fn create_collection(
        &self,
        name: String,
        kind: CollectionKind,
    ) -> std::result::Result<LocalCollectionId, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.create_collection(name, kind)
        })
        .await
    }

    async fn execute_ddl(&self, ddl: DdlBatch) -> std::result::Result<DdlOutcome, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.transact_ddl(ddl)
        })
        .await
    }

    async fn upsert_package(
        &self,
        package: Package,
    ) -> std::result::Result<PackageRegistrationOutcome, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.upsert_package(package)
        })
        .await
    }

    async fn upsert_relationship(
        &self,
        relationship: RelationType,
    ) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.upsert_relationship(relationship)
        })
        .await
    }

    async fn delete_relationship(&self, id: String) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.delete_relationship(&id)
        })
        .await
    }

    async fn insert(
        &self,
        collection: String,
        id: String,
        object: Object,
    ) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.insert(&collection, id, object)
        })
        .await
    }

    async fn get(
        &self,
        collection: String,
        id: String,
    ) -> std::result::Result<Option<EntityRecord>, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            db.get(&collection, &id)
        })
        .await
    }

    async fn delete(&self, collection: String, id: String) -> std::result::Result<(), DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.delete(&collection, &id)
        })
        .await
    }

    async fn query(&self, query: TextQueryInput) -> std::result::Result<QueryResult, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await?
            }
        };
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || match query {
            Query::Select(query) => {
                let db = db.read().map_err(|_| lock_poisoned_error())?;
                db.select(query).map(QueryResult::Select)
            }
            query => {
                let mut db = db.write().map_err(|_| lock_poisoned_error())?;
                db.query(query)
            }
        })
        .await
    }

    async fn explain(&self, query: TextQueryInput) -> std::result::Result<QueryExplain, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await?
            }
        };
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            db.explain_query(query)
        })
        .await
    }

    async fn plan(&self, query: TextQueryInput) -> std::result::Result<QueryPlan, DbError> {
        let query = match query {
            TextQueryInput::Ast(query) => query,
            TextQueryInput::Text {
                format,
                query,
                params,
            } => {
                self.parse_text_query_with_params(format, &query, &params)
                    .await?
            }
        };
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let db = db.read().map_err(|_| lock_poisoned_error())?;
            db.plan_query(query)
        })
        .await
    }

    async fn update_where(
        &self,
        query: UpdateQuery,
    ) -> std::result::Result<MutationStats, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.update_where(query)
        })
        .await
    }

    async fn delete_where(&self, query: DeleteQuery) -> std::result::Result<usize, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.delete_where(query)
        })
        .await
    }

    async fn execute_batch(&self, batch: Batch) -> std::result::Result<BatchOutcome, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            let mut db = db.write().map_err(|_| lock_poisoned_error())?;
            db.execute_batch(batch)
        })
        .await
    }

    async fn execute_batch_returning(
        &self,
        batch: Batch,
        returning: crate::BatchReturn,
    ) -> Result<crate::BatchReply, DbError> {
        let db = Arc::clone(&self.db);
        spawn_blocking_on(self.runtime.as_ref(), move || {
            db.write()
                .map_err(|_| lock_poisoned_error())?
                .execute_batch_returning(batch, returning)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::Backend;

    use super::*;
    use crate::embedded::MemoryEntityStorage;

    fn assert_backend_impl<T: Backend>() {}

    #[test]
    fn kv_backend_blanket_impl_compiles() {
        assert_backend_impl::<EmbeddedBackend<MemoryEntityStorage>>();
    }

    #[cfg(feature = "tokio")]
    #[test]
    fn kv_backend_runtime_constructors_compile() {
        let db = EmbeddedDb::new(MemoryEntityStorage::new());
        let _ = EmbeddedBackend::with_runtime(db, Arc::new(crate::TokioAsyncRuntime));
    }
}
