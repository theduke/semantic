use super::*;
use crate::labels::LabelStore;
use semantic_data::value::{FromValue, IntoValue, SemanticType};
use semantic_rpc_core::{CommandAdapter, DynCommand, RpcCommand, RpcCommandSpec, RpcError};
use std::{future::Future, pin::Pin};
pub trait CommentContext: Sync + 'static {
    type Store: LabelStore;
    fn comment_store(
        &self,
        scope_id: Option<String>,
    ) -> impl Future<Output = Result<Self::Store, RpcError>> + Send;
    fn comment_author(&self) -> String;
    fn comment_is_privileged(&self) -> bool;
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct ListCommentsPayload {
    pub scope_id: Option<String>,
    pub target: EntityTarget,
    pub offset: u64,
    pub limit: u64,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct CreateCommentPayload {
    pub scope_id: Option<String>,
    pub target: EntityTarget,
    pub main_content: crate::content::MainContent,
    pub parent: Option<String>,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct EditCommentPayload {
    pub scope_id: Option<String>,
    pub id: String,
    pub main_content: crate::content::MainContent,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct DeleteCommentPayload {
    pub scope_id: Option<String>,
    pub id: String,
}
macro_rules! command {
    ($name:ident,$id:literal,$payload:ty,$output:ty,$run:ident) => {
        pub struct $name;
        impl RpcCommandSpec for $name {
            type Payload = $payload;
            type Output = $output;
            type Error = RpcError;
            const NAME: &'static str = $id;
        }
        impl<Ctx: CommentContext> RpcCommand<Ctx> for $name {
            fn call<'a>(
                &'a self,
                ctx: &'a Ctx,
                p: $payload,
            ) -> Pin<Box<dyn Future<Output = Result<$output, RpcError>> + Send + 'a>> {
                Box::pin($run(ctx, p))
            }
        }
    };
}
async fn list(ctx: &impl CommentContext, p: ListCommentsPayload) -> Result<CommentPage, RpcError> {
    list_comments(&ctx.comment_store(p.scope_id.clone()).await?, p).await
}
async fn create(ctx: &impl CommentContext, p: CreateCommentPayload) -> Result<Comment, RpcError> {
    create_comment(
        &ctx.comment_store(p.scope_id.clone()).await?,
        p,
        ctx.comment_author(),
    )
    .await
}
async fn edit(ctx: &impl CommentContext, p: EditCommentPayload) -> Result<Comment, RpcError> {
    edit_comment(
        &ctx.comment_store(p.scope_id).await?,
        &p.id,
        Some(p.main_content),
        &ctx.comment_author(),
        ctx.comment_is_privileged(),
    )
    .await
}
async fn delete(ctx: &impl CommentContext, p: DeleteCommentPayload) -> Result<Comment, RpcError> {
    edit_comment(
        &ctx.comment_store(p.scope_id).await?,
        &p.id,
        None,
        &ctx.comment_author(),
        ctx.comment_is_privileged(),
    )
    .await
}
command!(
    ListComments,
    "semantic.comments.list",
    ListCommentsPayload,
    CommentPage,
    list
);
command!(
    CreateComment,
    "semantic.comments.create",
    CreateCommentPayload,
    Comment,
    create
);
command!(
    EditComment,
    "semantic.comments.edit",
    EditCommentPayload,
    Comment,
    edit
);
command!(
    DeleteComment,
    "semantic.comments.delete",
    DeleteCommentPayload,
    Comment,
    delete
);
pub fn commands<Ctx: CommentContext, E: From<RpcError>>() -> Vec<Box<dyn DynCommand<Ctx, E>>> {
    vec![
        Box::new(CommandAdapter::new(ListComments)),
        Box::new(CommandAdapter::new(CreateComment)),
        Box::new(CommandAdapter::new(EditComment)),
        Box::new(CommandAdapter::new(DeleteComment)),
    ]
}
