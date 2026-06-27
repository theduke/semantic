use std::sync::Arc;

use crate::{AppError, AppSession, DbScopeId, Principal, SemanticApp, SemanticDb};

#[derive(Clone)]
pub struct AppRequestContext {
    pub app: SemanticApp,
    pub principal: Principal,
    pub session: Option<Arc<AppSession>>,
    pub request_scope: Option<DbScopeId>,
}

impl AppRequestContext {
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
}
