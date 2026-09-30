mod detail;
use crate::{components::PageHeader, views::Route};
pub use detail::{CreateTaskPage, TaskPage};
use dioxus::prelude::*;
use semantic_base::tasks::{ListTasks, Task, TaskListPayload, TaskPriority, TaskStatus};
use semantic_data::value::{Date, Object, Value};
use semantic_ui_core::{use_active_scope_id, use_rpc_client};

/// Availability is scoped and includes handler registration, so retained schema
/// does not expose disabled application features.
pub(crate) fn use_tasks_available() -> Memo<Option<bool>> {
    let rpc = use_rpc_client();
    let scope = use_active_scope_id();
    let scope_key = use_memo(use_reactive(&scope, |scope| scope));
    let resource = use_resource(move || {
        let rpc = rpc.clone();
        let scope = scope_key();
        async move {
            let mut payload = Object::new();
            if let Some(scope) = scope {
                payload.insert("scope_id", Value::String(scope));
            }
            rpc.invoke_value("semantic.app.capabilities", Value::Object(payload))
                .await
                .ok()
                .and_then(|value| match value {
                    Value::Object(o) => o.get("tasks").and_then(Value::as_bool),
                    _ => None,
                })
                .unwrap_or(false)
        }
    });
    use_memo(move || resource.read().as_ref().copied())
}

pub(super) const STATUSES: [TaskStatus; 6] = [
    TaskStatus::Backlog,
    TaskStatus::Todo,
    TaskStatus::InProgress,
    TaskStatus::Blocked,
    TaskStatus::Done,
    TaskStatus::Canceled,
];
pub(super) const PRIORITIES: [TaskPriority; 5] = [
    TaskPriority::None,
    TaskPriority::Low,
    TaskPriority::Medium,
    TaskPriority::High,
    TaskPriority::Urgent,
];
pub(super) fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Backlog => "Backlog",
        TaskStatus::Todo => "To do",
        TaskStatus::InProgress => "In progress",
        TaskStatus::Blocked => "Blocked",
        TaskStatus::Done => "Done",
        TaskStatus::Canceled => "Canceled",
    }
}
pub(super) fn priority_label(priority: TaskPriority) -> &'static str {
    match priority {
        TaskPriority::None => "No priority",
        TaskPriority::Low => "Low",
        TaskPriority::Medium => "Medium",
        TaskPriority::High => "High",
        TaskPriority::Urgent => "Urgent",
    }
}
pub(super) fn date_text(date: semantic_base::tasks::DueDate) -> String {
    let date: time::Date = date.0.into();
    date.to_string()
}
pub(super) fn parse_date(value: &str) -> Result<Option<semantic_base::tasks::DueDate>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    time::Date::parse(value, &time::format_description::well_known::Iso8601::DATE)
        .map(|date| Some(semantic_base::tasks::DueDate(Date::from(date))))
        .map_err(|_| "Enter a valid due date.".into())
}

#[component]
pub fn TasksPage() -> Element {
    let scope = use_active_scope_id();
    rsx! { TaskWorkspace { key: "{scope:?}", scope } }
}

#[component]
fn TaskWorkspace(scope: Option<String>) -> Element {
    let available = use_tasks_available();
    let rpc = use_rpc_client();
    let mut search = use_signal(String::new);
    let mut status = use_signal(String::new);
    let mut priority = use_signal(String::new);
    let mut due = use_signal(String::new);
    let mut archived = use_signal(|| false);
    let mut sort = use_signal(|| "updated".to_owned());
    let mut limit = use_signal(|| 40u64);
    let mut reload = use_signal(|| 0u64);
    let rows = use_resource(move || {
        let rpc = rpc.clone();
        let scope = scope.clone();
        let search = search();
        let status = status();
        let priority = priority();
        let due = due();
        let enabled = available();
        let _ = reload();
        let payload = TaskListPayload {
            scope_id: scope,
            search: Some(search),
            status: STATUSES.into_iter().find(|s| s.as_str() == status),
            priority: PRIORITIES.into_iter().find(|p| p.as_str() == priority),
            overdue: due == "overdue",
            has_due_date: due == "scheduled",
            include_archived: archived(),
            sort: Some(sort()),
            limit: limit(),
            ..Default::default()
        };
        async move {
            if enabled != Some(true) {
                return Ok(None);
            }
            load_page(&rpc, payload).await.map(Some)
        }
    });
    let filtered = !search().is_empty()
        || !status().is_empty()
        || !priority().is_empty()
        || !due().is_empty()
        || archived();
    let clear = move |_| {
        search.set(String::new());
        status.set(String::new());
        priority.set(String::new());
        due.set(String::new());
        archived.set(false);
        limit.set(40);
    };
    let response = rows.read().clone();
    rsx! {
        section { class: "semantic-page semantic-tasks",
            PageHeader { title: "Tasks", description: "Make room for what matters. Keep the next step clear.",
                actions: rsx! { if available() == Some(true) { Link { class: "semantic-button-link", to: Route::CreateTaskPage { parent: None }, "+ New task" } } }
            }
            if available().is_none() { p { role: "status", "Opening task workspace…" } }
            else if available() == Some(false) { UnavailableTasks {} }
            else {
                div { class: "semantic-task-toolbar",
                    label { "Search", input { r#type: "search", placeholder: "Find a task…", value: search(), oninput: move |e| { search.set(e.value()); limit.set(40); } } }
                    label { "Status", select { value: status(), onchange: move |e| { status.set(e.value()); limit.set(40); }, option { value: "", "All statuses" } for value in STATUSES { option { value: value.as_str(), "{status_label(value)}" } } } }
                    label { "Priority", select { value: priority(), onchange: move |e| { priority.set(e.value()); limit.set(40); }, option { value: "", "All priorities" } for value in PRIORITIES { option { value: value.as_str(), "{priority_label(value)}" } } } }
                    label { "Due date", select { value: due(), onchange: move |e| { due.set(e.value()); limit.set(40); }, option { value: "", "Any date" } option { value: "scheduled", "Has due date" } option { value: "overdue", "Overdue" } } }
                    label { "Sort", select { value: sort(), onchange: move |e| sort.set(e.value()), option { value: "updated", "Recently updated" } option { value: "due", "Due date" } option { value: "title", "Title" } } }
                    label { "Archive", select { value: if archived() { "all" } else { "active" }, onchange: move |e| { archived.set(e.value() == "all"); limit.set(40); }, option { value: "active", "Active tasks" } option { value: "all", "Include archived" } } }
                    if filtered { dxcomp::Button { variant: dxcomp::ButtonVariant::Ghost, onclick: clear, "Clear all" } }
                }
                if *rows.state().read() == UseResourceState::Pending { p { role: "status", "Loading tasks…" } }
                match response {
                    Some(Err(message)) => rsx! { p { class: "semantic-error", role: "alert", "{message}" } dxcomp::Button { onclick: move |_| reload += 1, "Try again" } },
                    Some(Ok(Some(page))) => rsx! {
                        div { class: "semantic-task-summary", span { "{page.total} tasks" } span { "Explicit progress · nested subtasks" } }
                        if page.tasks.is_empty() {
                            div { class: "semantic-task-empty",
                                span { class: "semantic-task-empty__mark", aria_hidden: "true", "✓" }
                                h2 { if filtered { "No tasks match these filters" } else { "A clear place to start" } }
                                p { if filtered { "Try another search or clear the filters to see more of your workspace." } else { "Capture a next step, add a little context, and turn a bigger idea into manageable subtasks." } }
                                if filtered { dxcomp::Button { onclick: clear, "Clear filters" } }
                                else { Link { class: "semantic-button-link", to: Route::CreateTaskPage { parent: None }, "Create your first task" } }
                            }
                        } else {
                            div { class: "semantic-task-list", for task in page.tasks { TaskRow { key: "{task.id}", task } } }
                            if page.total > limit() { div { class: "semantic-task-loadmore", dxcomp::Button { variant: dxcomp::ButtonVariant::Outline, onclick: move |_| limit += 40, "Load more tasks" } } }
                        }
                    },
                    _ => rsx! {},
                }
            }
        }
    }
}

#[component]
pub(super) fn TaskRow(task: Task) -> Element {
    let overdue = task.due_date.is_some_and(|d| d.0 < Date::now_utc())
        && !matches!(task.status, TaskStatus::Done | TaskStatus::Canceled);
    rsx! {
        Link { to: Route::TaskPage { id: task.id.clone() }, class: "semantic-task-row",
            div { class: "semantic-task-row__title", "{task.title}", div { class: "semantic-task-row__meta", if task.parent.is_some() { span { "↳ Subtask" } } if task.archived { span { "Archived" } } } }
            span { class: "semantic-task-status", "data-status": task.status.as_str(), "{status_label(task.status)}" }
            span { class: "semantic-task-priority", "{priority_label(task.priority)}" }
            span { class: "semantic-task-due", "data-overdue": overdue, if let Some(date) = task.due_date { "{date_text(date)}" } else { "No due date" } }
            span { class: "semantic-task-progress", progress { max: "100", value: "{task.progress}", aria_label: "Task progress" } "{task.progress}%" }
        }
    }
}

#[component]
pub(super) fn UnavailableTasks() -> Element {
    rsx! { div { class: "semantic-task-empty", h2 { "Tasks are unavailable in this scope" } p { "This workspace has not enabled the task package. Choose a scope with tasks enabled, or enable tasks in the application configuration." } Link { class: "semantic-button-link semantic-button-link--secondary", to: Route::HomePage, "Return to workspace" } } }
}

/// Fetch consecutive bounded server pages as the visible list grows.
pub(super) async fn load_page(
    rpc: &semantic_rpc::RpcClient,
    mut payload: TaskListPayload,
) -> Result<semantic_base::tasks::TaskPage, String> {
    let wanted = payload.limit;
    let mut tasks = Vec::new();
    loop {
        payload.offset = tasks.len() as u64;
        payload.limit = (wanted - payload.offset).min(100);
        let page = rpc
            .invoke::<ListTasks>(payload.clone())
            .await
            .map_err(|e| e.to_string())?;
        let total = page.total;
        let empty = page.tasks.is_empty();
        tasks.extend(page.tasks);
        if empty || tasks.len() as u64 >= wanted || tasks.len() as u64 >= total {
            return Ok(semantic_base::tasks::TaskPage { tasks, total });
        }
    }
}
