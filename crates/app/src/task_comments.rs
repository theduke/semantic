//! Scoped, authorized store adapters for optional task/comment packages.
use crate::{AppRequestContext, PrincipalKind};
use semantic_base::labels::LabelContext;
use semantic_base::{comments::CommentContext, tasks::TaskContext};
use semantic_rpc_core::RpcError;

impl TaskContext for AppRequestContext {
    type Store = crate::labels::AppLabelStore;
    async fn task_store(&self, scope_id: Option<String>) -> Result<Self::Store, RpcError> {
        self.label_store(scope_id).await
    }
}

impl CommentContext for AppRequestContext {
    type Store = crate::labels::AppLabelStore;
    async fn comment_store(&self, scope_id: Option<String>) -> Result<Self::Store, RpcError> {
        self.label_store(scope_id).await
    }
    fn comment_author(&self) -> String {
        self.principal.id.to_string()
    }
    fn comment_is_privileged(&self) -> bool {
        self.principal.kind == PrincipalKind::System
    }
}
