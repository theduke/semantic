use crate::DbScopeId;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppSessionId(String);

impl AppSessionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct AppSession {
    pub id: AppSessionId,
    current_scope: tokio::sync::RwLock<Option<DbScopeId>>,
}

impl AppSession {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: AppSessionId::new(id),
            current_scope: tokio::sync::RwLock::new(None),
        }
    }

    pub async fn current_scope(&self) -> Option<DbScopeId> {
        self.current_scope.read().await.clone()
    }

    pub async fn set_current_scope(&self, scope_id: Option<DbScopeId>) {
        *self.current_scope.write().await = scope_id;
    }
}
