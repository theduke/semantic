use dioxus::prelude::*;
use semantic_data::{Object, Value, jobs::*};
use semantic_ui_core::{use_active_scope_id, use_rpc_client};
use std::time::Duration;

#[component]
pub fn JobsPage() -> Element {
    let scope = use_active_scope_id();
    rsx! { JobsView { key: "{scope:?}", scope } }
}

#[component]
fn JobsView(scope: Option<String>) -> Element {
    let rpc = use_rpc_client();
    let mut page = use_signal(|| JobListPage {
        records: Vec::new(),
        next_cursor: None,
    });
    let mut cursor = use_signal(|| None::<JobListCursor>);
    let mut status = use_signal(String::new);
    let mut kind = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut refresh = use_signal(|| 0u64);
    let mut titles = use_signal(std::collections::BTreeMap::<String, String>::new);
    let poll_rpc = rpc.clone();
    let poll_scope = scope.clone();
    use_resource(move || {
        let rpc = poll_rpc.clone();
        let scope = poll_scope.clone();
        let _ = refresh();
        let query = JobListQuery {
            statuses: JobStatus::parse(&status()).into_iter().collect(),
            kind: (!kind().is_empty()).then(|| JobKindId(kind())),
            cursor: cursor(),
            ..Default::default()
        };
        async move {
            let mut payload = Object::new();
            if let Some(scope) = &scope {
                payload.insert("scope_id", scope.clone());
            }
            if let Ok(Value::List(kinds)) = rpc
                .invoke_value("semantic.jobs.kinds", Value::Object(payload))
                .await
            {
                titles.set(
                    kinds
                        .iter()
                        .filter_map(|v| {
                            let Value::Object(v) = v else {
                                return None;
                            };
                            Some((
                                v.get("id")?.as_str()?.to_owned(),
                                v.get("title")?.as_str()?.to_owned(),
                            ))
                        })
                        .collect(),
                );
            }
            loop {
                let mut payload = query.to_object();
                if let Some(scope) = &scope {
                    payload.insert("scope_id", scope.clone());
                }
                match rpc
                    .invoke_value("semantic.jobs.list", Value::Object(payload))
                    .await
                {
                    Ok(value) => match JobListPage::from_value(&value) {
                        Ok(value) => {
                            page.set(value);
                            error.set(None);
                        }
                        Err(message) => error.set(Some(message)),
                    },
                    Err(err) => error.set(Some(err.to_string())),
                }
                dioxus_sdk_time::sleep(Duration::from_secs(2)).await;
            }
        }
    });
    let control = use_callback(move |(command, id): (&'static str, Option<String>)| {
        let rpc = rpc.clone();
        let scope = scope.clone();
        spawn(async move {
            let mut payload = Object::new();
            if let Some(scope) = scope {
                payload.insert("scope_id", scope);
            }
            if let Some(id) = id {
                payload.insert("id", id);
            }
            match rpc.invoke_value(command, Value::Object(payload)).await {
                Ok(_) => refresh += 1,
                Err(err) => error.set(Some(err.to_string())),
            }
        });
    });
    rsx! {
        section { class: "semantic-page",
            h1 { "Jobs" }
            p { "Job history stores status and progress. Restarted work is interrupted and cannot be resumed." }
            div {
                label { "Status "
                    select { value: status, onchange: move |event| { status.set(event.value()); cursor.set(None); },
                        option { value: "", "All" }
                        for status in JobStatus::ALL { option { value: status.as_str(), "{status.as_str()}" } }
                    }
                }
                label { " Kind " input { value: kind, oninput: move |event| { kind.set(event.value()); cursor.set(None); } } }
                button { onclick: move |_| control.call(("semantic.jobs.clear_completed", None)), "Clear completed" }
            }
            if let Some(message) = error() { p { role: "alert", "{message}" } }
            table {
                thead { tr { th { "Kind / ID" } th { "Status" } th { "Progress" } th { "Created / Finished" } th { "Error" } th { "Actions" } } }
                tbody {
                    for record in page().records {
                        tr { key: "{record.id.0}",
                            td { "{titles().get(&record.kind.0).cloned().unwrap_or_else(|| record.kind.0.clone())}" br {} small { "{record.id.0}" } }
                            td { "{record.status.as_str()}" }
                            td { "{progress_text(&record.progress)}" }
                            td { "{timestamp(record.created_at)}" if let Some(finished) = record.finished_at { br {} "{timestamp(finished)}" } }
                            td { if let Some(err) = record.error { "{err.code}: {err.message}" } }
                            td { if !record.status.is_terminal() { button { onclick: move |_| control.call(("semantic.jobs.cancel", Some(record.id.0.clone()))), "Cancel" } } }
                        }
                    }
                }
            }
            button { disabled: cursor().is_none(), onclick: move |_| cursor.set(None), "First page" }
            button { disabled: page().next_cursor.is_none(), onclick: move |_| cursor.set(page().next_cursor), "Next page" }
        }
    }
}
fn progress_text(progress: &JobProgress) -> String {
    let counter = match progress.total {
        Some(total) => format!("{} / {total}", progress.completed),
        None => progress.completed.to_string(),
    };
    format!(
        "{} {} {}",
        progress.phase.as_deref().unwrap_or(""),
        counter,
        progress.unit.as_deref().unwrap_or("")
    )
}
fn timestamp(value: semantic_data::DateTime) -> String {
    time::OffsetDateTime::from(value).to_string()
}
