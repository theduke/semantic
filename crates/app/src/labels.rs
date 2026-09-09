//! App authorization/scope adapter for the base package's label commands.
use crate::{AppRequestContext, DbScopeId, SemanticDb};
use semantic_base::labels::{LabelContext, LabelStore};
use semantic_data::{query::Batch, value::Object};
use semantic_db_core::{QueryResult, TextQueryInput};
use semantic_rpc_core::RpcError;
use std::sync::Arc;

pub struct AppLabelStore(Arc<dyn SemanticDb>);

impl LabelContext for AppRequestContext {
    type Store = AppLabelStore;
    async fn label_store(&self, scope_id: Option<String>) -> Result<AppLabelStore, RpcError> {
        self.resolve_db(scope_id.map(DbScopeId::new))
            .await
            .map(AppLabelStore)
            .map_err(Into::into)
    }
}

impl LabelStore for AppLabelStore {
    async fn select(&self, sql: String) -> Result<Vec<Object>, RpcError> {
        match self
            .0
            .query(TextQueryInput::sql(sql))
            .await
            .map_err(db_error)?
        {
            QueryResult::Select(rows) => Ok(rows),
            _ => Err(RpcError::internal("Expected label query rows")),
        }
    }
    async fn get(&self, collection: &str, id: &str) -> Result<Option<Object>, RpcError> {
        self.0
            .get(collection.into(), id.into())
            .await
            .map(|row| row.map(|row| row.object))
            .map_err(db_error)
    }
    async fn commit(&self, batch: Batch) -> Result<(), RpcError> {
        self.0
            .execute_batch(semantic_db_core::Batch {
                operations: batch.operations.into_iter().map(Into::into).collect(),
            })
            .await
            .map(|_| ())
            .map_err(db_error)
    }
}

fn db_error(error: semantic_db_core::DbError) -> RpcError {
    RpcError::new("label_storage_error", error.to_string())
}
