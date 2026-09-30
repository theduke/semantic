use super::*;
use crate::labels::LabelStore;
use semantic_data::value::{FromValue, IntoValue, SemanticType};
use semantic_rpc_core::{CommandAdapter, DynCommand, RpcCommand, RpcCommandSpec, RpcError};
use std::{future::Future, pin::Pin};
pub trait TaskContext: Sync + 'static {
    type Store: LabelStore;
    fn task_store(
        &self,
        scope_id: Option<String>,
    ) -> impl Future<Output = Result<Self::Store, RpcError>> + Send;
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, Default)]
pub struct TaskListPayload {
    pub scope_id: Option<String>,
    pub search: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub parent: Option<String>,
    pub overdue: bool,
    pub has_due_date: bool,
    pub include_archived: bool,
    pub sort: Option<String>,
    pub offset: u64,
    pub limit: u64,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct TaskIdPayload {
    pub scope_id: Option<String>,
    pub id: String,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct CreateTaskPayload {
    pub scope_id: Option<String>,
    pub title: String,
    pub main_content: crate::content::MainContent,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub progress: Option<u64>,
    pub due_date: Option<DueDate>,
    pub parent: Option<String>,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug, Default)]
pub struct UpdateTaskPayload {
    pub scope_id: Option<String>,
    pub id: String,
    pub title: Option<String>,
    pub main_content: Option<crate::content::MainContent>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub progress: Option<u64>,
    pub due_date: Option<DueDate>,
    pub clear_due_date: bool,
    pub parent: Option<String>,
    pub clear_parent: bool,
}
#[derive(SemanticType, IntoValue, FromValue, Clone, Debug)]
pub struct ArchiveTaskPayload {
    pub scope_id: Option<String>,
    pub id: String,
    pub archived: bool,
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
        impl<Ctx: TaskContext> RpcCommand<Ctx> for $name {
            fn call<'a>(
                &'a self,
                ctx: &'a Ctx,
                p: $payload,
            ) -> Pin<Box<dyn Future<Output = Result<$output, RpcError>> + Send + 'a>> {
                Box::pin(async move {
                    let store = ctx.task_store(p.scope_id.clone()).await?;
                    $run(&store, p).await
                })
            }
        }
    };
}
async fn detail(store: &impl LabelStore, p: TaskIdPayload) -> Result<TaskDetail, RpcError> {
    task_detail(store, &p.id).await
}
async fn archive(store: &impl LabelStore, p: ArchiveTaskPayload) -> Result<Task, RpcError> {
    archive_task(store, &p.id, p.archived).await
}
command!(
    ListTasks,
    "semantic.tasks.list",
    TaskListPayload,
    TaskPage,
    list_tasks
);
command!(
    GetTask,
    "semantic.tasks.get",
    TaskIdPayload,
    TaskDetail,
    detail
);
command!(
    CreateTask,
    "semantic.tasks.create",
    CreateTaskPayload,
    Task,
    create_task
);
command!(
    UpdateTask,
    "semantic.tasks.update",
    UpdateTaskPayload,
    Task,
    update_task
);
command!(
    ArchiveTask,
    "semantic.tasks.archive",
    ArchiveTaskPayload,
    Task,
    archive
);
pub fn commands<Ctx: TaskContext, E: From<RpcError>>() -> Vec<Box<dyn DynCommand<Ctx, E>>> {
    vec![
        Box::new(CommandAdapter::new(ListTasks)),
        Box::new(CommandAdapter::new(GetTask)),
        Box::new(CommandAdapter::new(CreateTask)),
        Box::new(CommandAdapter::new(UpdateTask)),
        Box::new(CommandAdapter::new(ArchiveTask)),
    ]
}
