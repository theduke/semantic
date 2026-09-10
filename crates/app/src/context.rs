use std::sync::Arc;

use objstore::DynObjStore;

use crate::object_store::ObjectStoreId;
use crate::{AppError, AppSession, DbScopeId, Principal, SemanticApp, SemanticDb};

#[derive(Clone)]
pub struct AppRequestContext {
    pub app: SemanticApp,
    pub principal: Principal,
    pub session: Option<Arc<AppSession>>,
    pub request_scope: Option<DbScopeId>,
}

impl AppRequestContext {
    pub async fn jobs(
        &self,
        scope_id: Option<DbScopeId>,
    ) -> Result<semantic_jobs::ScopeJobs, AppError> {
        let scope_id = self.resolve_scope_id(scope_id).await?;
        self.app.jobs(&self.principal, scope_id).await
    }
    pub async fn effective_scope_hint(&self) -> Option<DbScopeId> {
        if self.request_scope.is_some() {
            return self.request_scope.clone();
        }
        match &self.session {
            Some(session) => session.current_scope().await,
            None => None,
        }
    }

    pub async fn set_session_scope(
        &self,
        scope_id: Option<DbScopeId>,
    ) -> std::result::Result<(), AppError> {
        let Some(session) = &self.session else {
            return Err(AppError::InvalidRequest(
                "scope selection requires a session".to_string(),
            ));
        };
        session.set_current_scope(scope_id).await;
        Ok(())
    }

    pub async fn resolve_db(
        &self,
        scope_id: Option<DbScopeId>,
    ) -> std::result::Result<Arc<dyn SemanticDb>, AppError> {
        let session_scope = match (&scope_id, &self.request_scope, &self.session) {
            (Some(_), _, _) => None,
            (None, Some(_), _) => None,
            (None, None, Some(session)) => session.current_scope().await,
            (None, None, None) => None,
        };
        let requested_scope = scope_id.or_else(|| self.request_scope.clone());
        self.app
            .scopes()
            .resolve_scope(&self.principal, requested_scope, session_scope)
            .await
    }

    pub(crate) async fn resolve_object_store(
        &self,
        scope_id: Option<DbScopeId>,
        store_id: Option<ObjectStoreId>,
    ) -> std::result::Result<DynObjStore, AppError> {
        let scope_id = self.object_store_scope_id(scope_id).await?;
        match store_id {
            Some(store_id) => self.app.object_stores().resolve_store(&scope_id, &store_id),
            None => self.app.object_stores().resolve_default_store(&scope_id),
        }
    }

    pub(crate) async fn default_file_store(
        &self,
        scope_id: Option<DbScopeId>,
    ) -> std::result::Result<DynObjStore, AppError> {
        self.resolve_object_store(scope_id, None)
            .await
            .map_err(|err| match err {
                AppError::ObjectStoreRequired(scope_id) => AppError::FileStoreRequired(scope_id),
                other => other,
            })
    }

    pub(crate) async fn resolve_scope_id(
        &self,
        scope_id: Option<DbScopeId>,
    ) -> std::result::Result<DbScopeId, AppError> {
        if let Some(scope_id) = scope_id {
            return Ok(scope_id);
        }
        if let Some(scope_id) = &self.request_scope {
            return Ok(scope_id.clone());
        }
        if let Some(session) = &self.session
            && let Some(scope_id) = session.current_scope().await
        {
            return Ok(scope_id);
        }
        self.app
            .scopes()
            .default_scope()
            .ok_or(AppError::ScopeRequired)
    }

    async fn object_store_scope_id(
        &self,
        scope_id: Option<DbScopeId>,
    ) -> std::result::Result<DbScopeId, AppError> {
        self.resolve_scope_id(scope_id).await
    }
}
