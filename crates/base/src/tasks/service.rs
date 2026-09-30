use super::*;
use crate::{
    directory_query::{sql_ident, sql_string},
    domain_support::{error, new_id, persist},
    labels::LabelStore,
};
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    value::{Date, DateTime},
};
use semantic_rpc_core::RpcError;
use std::collections::BTreeSet;
static WRITES: futures::lock::Mutex<()> = futures::lock::Mutex::new(());
pub async fn get_task(store: &impl LabelStore, id: &str) -> Result<Task, RpcError> {
    let object = store
        .get(DEFAULT_COLLECTION, id)
        .await?
        .ok_or_else(|| error("Task not found"))?;
    Task::from_object(&object)
}
async fn parent_valid(
    store: &impl LabelStore,
    id: &str,
    parent: Option<&str>,
) -> Result<(), RpcError> {
    let mut visited = BTreeSet::from([id.to_owned()]);
    let mut cursor = parent.map(str::to_owned);
    while let Some(id) = cursor {
        if !visited.insert(id.clone()) {
            return Err(error("Task parent would create a cycle"));
        }
        cursor = get_task(store, &id).await?.parent;
    }
    Ok(())
}
fn validate(task: &mut Task) -> Result<(), RpcError> {
    task.title = task.title.trim().into();
    if task.title.is_empty() {
        return Err(error("Task title cannot be empty"));
    }
    if task.progress > 100 || (task.status == TaskStatus::Done && task.progress != 100) {
        return Err(error(
            "Progress must be 0–100 and done tasks must have 100% progress",
        ));
    }
    task.main_content.validate()
}
pub async fn create_task(store: &impl LabelStore, p: CreateTaskPayload) -> Result<Task, RpcError> {
    let _guard = WRITES.lock().await;
    let now = DateTime::now_utc();
    let mut task = Task {
        id: new_id("task"),
        title: p.title,
        main_content: p.main_content,
        status: p.status,
        priority: p.priority,
        progress: p
            .progress
            .unwrap_or(if p.status == TaskStatus::Done { 100 } else { 0 }),
        due_date: p.due_date,
        parent: p.parent,
        archived: false,
        created_at: now,
        updated_at: now,
    };
    validate(&mut task)?;
    parent_valid(store, &task.id, task.parent.as_deref()).await?;
    persist(store, task.id.clone(), task.to_object()).await?;
    Ok(task)
}
pub async fn update_task(store: &impl LabelStore, p: UpdateTaskPayload) -> Result<Task, RpcError> {
    let _guard = WRITES.lock().await;
    let mut object = store
        .get(DEFAULT_COLLECTION, &p.id)
        .await?
        .ok_or_else(|| error("Task not found"))?;
    let mut task = Task::from_object(&object)?;
    if let Some(title) = p.title {
        task.title = title;
    }
    if let Some(content) = p.main_content {
        task.main_content = content;
    }
    if let Some(status) = p.status {
        if status == TaskStatus::Done {
            task.progress = 100;
        } else if task.status == TaskStatus::Done {
            task.progress = 0;
        }
        task.status = status;
    }
    if let Some(progress) = p.progress {
        task.progress = progress;
    }
    if let Some(priority) = p.priority {
        task.priority = priority;
    }
    if p.clear_due_date {
        task.due_date = None;
    } else if let Some(date) = p.due_date {
        task.due_date = Some(date);
    }
    if p.clear_parent {
        task.parent = None;
    } else if let Some(parent) = p.parent {
        task.parent = Some(parent);
    }
    validate(&mut task)?;
    parent_valid(store, &task.id, task.parent.as_deref()).await?;
    task.updated_at = DateTime::now_utc();
    task.apply(&mut object);
    persist(store, task.id.clone(), object).await?;
    Ok(task)
}
pub async fn archive_task(
    store: &impl LabelStore,
    id: &str,
    archived: bool,
) -> Result<Task, RpcError> {
    let _guard = WRITES.lock().await;
    let mut object = store
        .get(DEFAULT_COLLECTION, id)
        .await?
        .ok_or_else(|| error("Task not found"))?;
    let mut task = Task::from_object(&object)?;
    task.archived = archived;
    task.updated_at = DateTime::now_utc();
    task.apply(&mut object);
    persist(store, id.into(), object).await?;
    Ok(task)
}
pub async fn list_tasks(store: &impl LabelStore, p: TaskListPayload) -> Result<TaskPage, RpcError> {
    let objects = store
        .select(format!(
            "SELECT * FROM {} WHERE type = {}",
            sql_ident(DEFAULT_COLLECTION),
            sql_string(schema::CLASS_ID)
        ))
        .await?;
    let mut tasks = objects
        .iter()
        .map(Task::from_object)
        .collect::<Result<Vec<_>, _>>()?;
    let today = Date::now_utc();
    let search = p.search.as_deref().unwrap_or("").trim().to_lowercase();
    tasks.retain(|t| {
        (p.include_archived || !t.archived)
            && p.status.is_none_or(|s| s == t.status)
            && p.priority.is_none_or(|v| v == t.priority)
            && p.parent
                .as_ref()
                .is_none_or(|v| t.parent.as_ref() == Some(v))
            && (!p.overdue
                || t.due_date.is_some_and(|d| d.0 < today)
                    && !matches!(t.status, TaskStatus::Done | TaskStatus::Canceled))
            && (!p.has_due_date || t.due_date.is_some())
            && (search.is_empty()
                || t.title.to_lowercase().contains(&search)
                || t.main_content.body().to_lowercase().contains(&search))
    });
    match p.sort.as_deref().unwrap_or("updated") {
        "updated" => tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id))),
        "due" => tasks.sort_by(|a, b| {
            a.due_date
                .is_none()
                .cmp(&b.due_date.is_none())
                .then(a.due_date.cmp(&b.due_date))
                .then(a.id.cmp(&b.id))
        }),
        "title" => tasks.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id))),
        _ => return Err(error("Unsupported task sort")),
    };
    let total = tasks.len() as u64;
    let tasks = tasks
        .into_iter()
        .skip(p.offset as usize)
        .take(p.limit.clamp(1, 100) as usize)
        .collect();
    Ok(TaskPage { tasks, total })
}
pub async fn task_detail(store: &impl LabelStore, id: &str) -> Result<TaskDetail, RpcError> {
    let task = get_task(store, id).await?;
    let objects = store
        .select(format!(
            "SELECT * FROM {} WHERE type = {} AND {} = {}",
            sql_ident(DEFAULT_COLLECTION),
            sql_string(schema::CLASS_ID),
            sql_ident(semantic_data::attr::ATTR_PARENT),
            sql_string(id)
        ))
        .await?;
    let mut children = objects
        .iter()
        .map(Task::from_object)
        .collect::<Result<Vec<_>, _>>()?;
    children.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    Ok(TaskDetail { task, children })
}
