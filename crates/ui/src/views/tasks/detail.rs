use super::{
    PRIORITIES, STATUSES, UnavailableTasks, date_text, parse_date, priority_label, status_label,
    use_tasks_available,
};
use crate::{components::PageHeader, views::Route};
use dioxus::prelude::*;
use semantic_base::{
    content::MainContent,
    tasks::{
        ArchiveTask, ArchiveTaskPayload, CreateTask, CreateTaskPayload, DueDate, GetTask, Task,
        TaskIdPayload, TaskListPayload, TaskPriority, TaskStatus, UpdateTask, UpdateTaskPayload,
    },
};
use semantic_data::value::{FromValue, Value};
use semantic_ui_core::{
    EntityComments, EntityLabelsButton, EntityTarget, MainContentEditor, use_active_scope_id,
    use_rpc_client,
};

#[component]
pub fn CreateTaskPage(parent: Option<String>) -> Element {
    let scope = use_active_scope_id();
    rsx! { CreateTaskSession { key: "{scope:?}:{parent:?}", parent } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::value::DateTime;

    fn task() -> Task {
        Task {
            id: "task".into(),
            title: "Original".into(),
            main_content: MainContent::note("Original description"),
            status: TaskStatus::Todo,
            priority: TaskPriority::None,
            progress: 0,
            due_date: None,
            parent: None,
            archived: false,
            created_at: DateTime::now_utc(),
            updated_at: DateTime::now_utc(),
        }
    }

    #[test]
    fn successful_save_and_archive_reconcile_remote_fields_before_next_patch() {
        for archived in [false, true] {
            let initial = task();
            let mut local = TaskDraft::from(&initial);
            local.title = "Local title".into();
            // Another client changed all other fields before our mutation returned.
            let mut returned = initial.clone();
            returned.title = local.title.clone();
            returned.main_content = MainContent::note("Remote description");
            returned.status = TaskStatus::InProgress;
            returned.priority = TaskPriority::Urgent;
            returned.progress = 40;
            returned.due_date = parse_date("2026-10-15").unwrap();
            returned.parent = Some("remote-parent".into());
            returned.archived = archived;
            assert!(local != TaskDraft::from(&returned));
            local = TaskDraft::from(&returned);
            assert!(local == TaskDraft::from(&returned));
            assert_eq!(
                MainContent::from_value(local.content.clone()).unwrap(),
                returned.main_content
            );
            local.title = "Next title".into();
            let patch = local.update_payload(
                &returned,
                None,
                MainContent::from_value(local.content.clone()).unwrap(),
                local.progress.parse().unwrap(),
                parse_date(&local.due_date).unwrap(),
            );
            assert_eq!(patch.title.as_deref(), Some("Next title"));
            assert!(patch.main_content.is_none());
            assert!(patch.status.is_none());
            assert!(patch.priority.is_none());
            assert!(patch.progress.is_none());
            assert!(patch.due_date.is_none() && !patch.clear_due_date);
            assert!(patch.parent.is_none() && !patch.clear_parent);
        }
    }
}
#[component]
fn CreateTaskSession(parent: Option<String>) -> Element {
    let available = use_tasks_available();
    rsx! { section { class: "semantic-page semantic-tasks",
        PageHeader { title: if parent.is_some() { "New subtask" } else { "New task" }, description: "Give your next step a name and the context it needs.", breadcrumbs: rsx! { Link { to: Route::TasksPage, "Tasks" } } }
        if available().is_none() { p { role: "status", "Opening task editor…" } }
        else if available() == Some(false) { UnavailableTasks {} }
        else { TaskForm { parent, on_saved: move |task: Task| { navigator().replace(Route::TaskPage { id: task.id }); } } }
    } }
}
#[component]
pub fn TaskPage(id: String) -> Element {
    let scope = use_active_scope_id();
    rsx! { TaskSession { key: "{scope:?}:{id}", id } }
}
#[component]
fn TaskSession(id: String) -> Element {
    let scope = use_active_scope_id();
    let available = use_tasks_available();
    let rpc = use_rpc_client();
    let mut refresh = use_signal(|| 0u64);
    let mut resource = use_resource(move || {
        let rpc = rpc.clone();
        let id = id.clone();
        let scope = scope.clone();
        let enabled = available();
        let _ = refresh();
        async move {
            if enabled != Some(true) {
                return Ok(None);
            }
            rpc.invoke::<GetTask>(TaskIdPayload {
                scope_id: scope,
                id,
            })
            .await
            .map(Some)
            .map_err(|e| e.to_string())
        }
    });
    let response = resource.read().clone();
    rsx! { section { class: "semantic-page semantic-tasks",
        if available().is_none() { p { role: "status", "Opening task…" } }
        else if available() == Some(false) { UnavailableTasks {} }
        else {
            match response {
                Some(Ok(Some(detail))) => rsx! {
                    PageHeader { title: detail.task.title.clone(), description: if detail.task.archived { "Archived task · restore it whenever you need." } else { "Keep the next step clear." }, breadcrumbs: rsx! { div { class: "semantic-task-breadcrumb", Link { to: Route::TasksPage, "Tasks" } if let Some(parent) = detail.task.parent.clone() { span { "/" } ParentLink { id: parent } } } } }
                    TaskForm { initial: Some(detail.task.clone()), subtasks: detail.children, on_saved: move |_| refresh += 1 }
                    EntityComments { target: EntityTarget::default_collection(detail.task.id) }
                },
                Some(Err(message)) => rsx! { PageHeader { title: "Task unavailable" } p { class: "semantic-error", role: "alert", "{message}" } dxcomp::Button { onclick: move |_| resource.restart(), "Try again" } Link { to: Route::TasksPage, "Return to tasks" } },
                _ => rsx! { p { role: "status", "Loading task…" } },
            }
        }
    } }
}
#[component]
fn ParentLink(id: String) -> Element {
    let scope = use_active_scope_id();
    let rpc = use_rpc_client();
    let load_id = id.clone();
    let parent = use_resource(move || {
        let rpc = rpc.clone();
        let id = load_id.clone();
        let scope = scope.clone();
        async move {
            rpc.invoke::<GetTask>(TaskIdPayload {
                scope_id: scope,
                id,
            })
            .await
            .ok()
            .map(|d| d.task.title)
        }
    });
    let title = parent
        .read()
        .as_ref()
        .cloned()
        .flatten()
        .unwrap_or_else(|| "Parent task".into());
    rsx! { Link { to: Route::TaskPage { id }, "{title}" } }
}

/// A complete controlled draft. Successful mutations replace it and the baseline together,
/// so unrelated changes returned by the server never become accidental local edits.
#[derive(Clone, PartialEq)]
struct TaskDraft {
    title: String,
    content: Value,
    status: TaskStatus,
    priority: TaskPriority,
    progress: String,
    due_date: String,
    parent: Option<String>,
}

impl TaskDraft {
    fn update_payload(
        &self,
        previous: &Task,
        scope: Option<String>,
        main_content: MainContent,
        amount: u64,
        date: Option<DueDate>,
    ) -> UpdateTaskPayload {
        UpdateTaskPayload {
            scope_id: scope,
            id: previous.id.clone(),
            title: (previous.title != self.title).then_some(self.title.clone()),
            main_content: (previous.main_content != main_content).then_some(main_content),
            status: (previous.status != self.status).then_some(self.status),
            priority: (previous.priority != self.priority).then_some(self.priority),
            progress: (previous.progress != amount).then_some(amount),
            due_date: (previous.due_date != date).then_some(date).flatten(),
            clear_due_date: previous.due_date.is_some() && date.is_none(),
            parent: (previous.parent != self.parent)
                .then_some(self.parent.clone())
                .flatten(),
            clear_parent: previous.parent.is_some() && self.parent.is_none(),
        }
    }
}

impl From<&Task> for TaskDraft {
    fn from(task: &Task) -> Self {
        Self {
            title: task.title.clone(),
            content: task.main_content.to_value(),
            status: task.status,
            priority: task.priority,
            progress: task.progress.to_string(),
            due_date: task.due_date.map(date_text).unwrap_or_default(),
            parent: task.parent.clone(),
        }
    }
}

#[component]
fn TaskForm(
    #[props(default)] initial: Option<Task>,
    #[props(default)] parent: Option<String>,
    #[props(default)] subtasks: Vec<Task>,
    on_saved: EventHandler<Task>,
) -> Element {
    let scope = use_active_scope_id();
    let rpc = use_rpc_client();
    let mut baseline = use_signal(|| initial.clone());
    let mut draft = use_signal(|| {
        initial
            .as_ref()
            .map(TaskDraft::from)
            .unwrap_or_else(|| TaskDraft {
                title: String::new(),
                content: MainContent::note("").to_value(),
                status: TaskStatus::default(),
                priority: TaskPriority::default(),
                progress: "0".into(),
                due_date: String::new(),
                parent: parent.clone(),
            })
    });
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut saved = use_signal(|| false);
    let options_rpc = rpc.clone();
    let options_scope = scope.clone();
    let options = use_resource(move || {
        let rpc = options_rpc.clone();
        let scope = options_scope.clone();
        async move {
            super::load_page(
                &rpc,
                TaskListPayload {
                    scope_id: scope,
                    limit: u64::MAX,
                    sort: Some("title".into()),
                    include_archived: true,
                    ..Default::default()
                },
            )
            .await
            .ok()
            .map(|p| p.tasks)
            .unwrap_or_default()
        }
    });
    let current_id = baseline().map(|t| t.id);
    let completed_children = subtasks
        .iter()
        .filter(|t| t.status == TaskStatus::Done)
        .count();
    let child_count = subtasks.len();
    let dirty = baseline().is_none_or(|task| TaskDraft::from(&task) != draft());
    let save_rpc = rpc.clone();
    let save_scope = scope.clone();
    let save = move |_| {
        if busy() {
            return;
        }
        let parsed = (|| -> Result<(MainContent, u64, Option<DueDate>), String> {
            if draft().title.trim().is_empty() {
                return Err("Give this task a title before saving.".into());
            }
            let progress = draft()
                .progress
                .parse::<u64>()
                .map_err(|_| "Progress must be a whole number from 0 to 100.".to_owned())?;
            if progress > 100 {
                return Err("Progress must be between 0 and 100.".into());
            }
            if draft().status == TaskStatus::Done && progress != 100 {
                return Err("A completed task must have 100% progress.".into());
            }
            let content = MainContent::from_value(draft().content).map_err(|e| e.to_string())?;
            Ok((content, progress, parse_date(&draft().due_date)?))
        })();
        let (main_content, amount, date) = match parsed {
            Ok(value) => value,
            Err(message) => {
                error.set(Some(message));
                return;
            }
        };
        let rpc = save_rpc.clone();
        let scope = save_scope.clone();
        let previous = baseline();
        let values = draft();
        let title_value = values.title.clone();
        let new_status = draft().status;
        let new_priority = draft().priority;
        let new_parent = draft().parent;
        busy.set(true);
        error.set(None);
        saved.set(false);
        spawn(async move {
            let result = if let Some(previous) = previous {
                rpc.invoke::<UpdateTask>(values.update_payload(
                    &previous,
                    scope,
                    main_content,
                    amount,
                    date,
                ))
                .await
            } else {
                rpc.invoke::<CreateTask>(CreateTaskPayload {
                    scope_id: scope,
                    title: title_value,
                    main_content,
                    status: new_status,
                    priority: new_priority,
                    progress: Some(amount),
                    due_date: date,
                    parent: new_parent,
                })
                .await
            };
            busy.set(false);
            match result {
                Ok(task) => {
                    draft.set(TaskDraft::from(&task));
                    baseline.set(Some(task.clone()));
                    saved.set(true);
                    on_saved.call(task);
                }
                Err(e) => error.set(Some(e.to_string())),
            }
        });
    };
    let archive_rpc = rpc.clone();
    let archive_scope = scope.clone();
    rsx! {
        div { class: "semantic-task-detail",
            div { class: "semantic-task-detail__main",
                div { class: "semantic-task-description",
                    label { class: "semantic-task-field semantic-task-field--title", "Title", input { aria_label: "Task title", value: draft().title, placeholder: "What needs to happen?", disabled: busy(), oninput: move |e| { draft.write().title = e.value(); saved.set(false); } } }
                    h2 { "Description" }
                    MainContentEditor { value: draft().content, disabled: busy(), on_change: move |value: Value| { draft.write().content = value; saved.set(false); } }
                    if let Some(message) = error() { p { class: "semantic-error", role: "alert", "{message}" } }
                    div { class: "semantic-task-actions",
                        dxcomp::Button { disabled: busy() || (!dirty && baseline().is_some()), onclick: save, if busy() { "Saving…" } else if baseline().is_some() { "Save changes" } else { "Create task" } }
                        if baseline().is_none() { Link { class: "semantic-button-link semantic-button-link--secondary", to: Route::TasksPage, "Cancel" } }
                        if saved() && !dirty { span { role: "status", class: "semantic-label-muted", "Changes saved" } }
                        if dirty && baseline().is_some() { span { class: "semantic-label-muted", "Unsaved changes" } }
                    }
                }
                if let Some(id) = current_id.clone() {
                    div { class: "semantic-task-subtasks",
                        div { class: "semantic-task-subtasks__heading", h2 { "Subtasks · {completed_children}/{child_count} complete" } Link { class: "semantic-button-link semantic-button-link--secondary", to: Route::CreateTaskPage { parent: Some(id) }, "+ Add subtask" } }
                        if subtasks.is_empty() { p { class: "semantic-label-muted", "Break this task into smaller steps. Child completion is tracked separately from this task’s progress." } }
                        for child in subtasks { Link { key: "{child.id}", class: "semantic-task-subtasks__row", to: Route::TaskPage { id: child.id }, span { "{child.title}" } span { class: "semantic-task-status", "data-status": child.status.as_str(), "{status_label(child.status)}" } } }
                    }
                }
            }
            aside { class: "semantic-task-properties", aria_label: "Task properties",
                h2 { "Properties" }
                label { class: "semantic-task-field", "Status", select { aria_label: "Status", disabled: busy(), value: draft().status.as_str(), onchange: move |e| { if let Some(value) = STATUSES.into_iter().find(|s| s.as_str() == e.value()) { if value == TaskStatus::Done { draft.write().progress = "100".into(); } else if draft().status == TaskStatus::Done { draft.write().progress = "0".into(); } draft.write().status = value; saved.set(false); } }, for value in STATUSES { option { value: value.as_str(), "{status_label(value)}" } } } }
                label { class: "semantic-task-field", "Priority", select { aria_label: "Priority", disabled: busy(), value: draft().priority.as_str(), onchange: move |e| { if let Some(value) = PRIORITIES.into_iter().find(|p| p.as_str() == e.value()) { draft.write().priority = value; saved.set(false); } }, for value in PRIORITIES { option { value: value.as_str(), "{priority_label(value)}" } } } }
                label { class: "semantic-task-field", "Due date", input { r#type: "date", disabled: busy(), value: draft().due_date, oninput: move |e| { draft.write().due_date = e.value(); saved.set(false); } } }
                label { class: "semantic-task-field", "Progress (%)", input { r#type: "number", min: "0", max: "100", step: "1", disabled: busy(), value: draft().progress, oninput: move |e| { draft.write().progress = e.value(); saved.set(false); } } small { "Your estimate, independent of subtask completion." } }
                label { class: "semantic-task-field semantic-task-field--parent", "Parent task", select { aria_label: "Parent task", disabled: busy(), value: draft().parent.unwrap_or_default(), onchange: move |e| { let value = e.value(); draft.write().parent = (!value.is_empty()).then_some(value); saved.set(false); }, option { value: "", selected: draft().parent.is_none(), "No parent" } if let Some(tasks) = options.read().as_ref() { for task in tasks.iter().filter(|t| Some(&t.id) != current_id.as_ref()) { option { key: "{task.id}", value: task.id.clone(), selected: draft().parent.as_deref() == Some(task.id.as_str()), if task.archived { "{task.title} (archived)" } else { "{task.title}" } } } } } }
                if let Some(id) = current_id {
                    EntityLabelsButton { target: EntityTarget::default_collection(id) }
                    div { class: "semantic-task-actions",
                        dxcomp::Button { variant: dxcomp::ButtonVariant::Outline, disabled: busy() || dirty,
                            onclick: move |_| {
                                let Some(task) = baseline() else { return; }; let rpc = archive_rpc.clone(); let scope = archive_scope.clone();
                                busy.set(true); error.set(None);
                                spawn(async move { let result = rpc.invoke::<ArchiveTask>(ArchiveTaskPayload { scope_id: scope, id: task.id, archived: !task.archived }).await; busy.set(false); match result { Ok(task) => { draft.set(TaskDraft::from(&task)); baseline.set(Some(task.clone())); saved.set(true); on_saved.call(task); }, Err(e) => error.set(Some(e.to_string())) } });
                            }, if baseline().is_some_and(|t| t.archived) { "Restore task" } else { "Archive task" }
                        }
                    }
                }
            }
        }
    }
}
