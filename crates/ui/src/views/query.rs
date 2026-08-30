use std::{collections::BTreeSet, rc::Rc, time::Instant};

use dioxus::dioxus_core::Task;
use dioxus::prelude::*;
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    value::{Object, Value},
};
use semantic_ui_core::{
    components::{EmptyState, ErrorState, InlineNotice, NoticeVariant, RefreshingIndicator},
    use_active_scope_id, use_rpc_client,
};

use crate::components::{PageHeader, QueryEditor, value_string};

const DEFAULT_LIMIT: usize = 100;
const MAX_ROWS: usize = 500;
const MAX_COLUMNS: usize = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
struct QueryRequest {
    generation: u64,
    sql: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Activity {
    #[default]
    Idle,
    Running(QueryRequest),
    Cancelled(QueryRequest),
    Succeeded,
    Failed {
        request: QueryRequest,
        error: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct QuerySuccess {
    request: QueryRequest,
    output: QueryOutput,
    duration_ms: u128,
}

#[derive(Clone, Debug, PartialEq)]
enum QueryOutput {
    Rows(QueryRows),
    NonRow { kind: String, fields: Object },
    Unexpected(Value),
}

#[derive(Clone, Debug, PartialEq)]
struct QueryRows {
    rows: Rc<[Object]>,
    total: usize,
    malformed: usize,
}

#[component]
pub fn QueryPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut draft =
        use_signal(|| format!("SELECT * FROM {DEFAULT_COLLECTION} LIMIT {DEFAULT_LIMIT}"));
    let mut validation_error = use_signal(|| None::<String>);
    let mut generation = use_signal(|| 0_u64);
    let mut submitted = use_signal(|| None::<QueryRequest>);
    let mut activity = use_signal(Activity::default);
    let mut last_success = use_signal(|| None::<Rc<QuerySuccess>>);
    let mut active_task = use_signal(|| None::<Task>);

    let submit = use_callback({
        let client = client.clone();
        let scope_id = scope_id.clone();
        move |()| {
            let sql = match validate_read_only_sql(&draft.read()) {
                Ok(sql) => sql,
                Err(error) => {
                    validation_error.set(Some(error));
                    return;
                }
            };
            validation_error.set(None);
            if let Some(task) = active_task.take() {
                task.cancel();
            }
            let id = generation().saturating_add(1);
            generation.set(id);
            let request = QueryRequest {
                generation: id,
                sql,
            };
            submitted.set(Some(request.clone()));
            activity.set(Activity::Running(request.clone()));

            let client = client.clone();
            let scope_id = scope_id.clone();
            let started = Instant::now();
            let task_request = request.clone();
            let task = spawn(async move {
                let response = run_query(client, scope_id, task_request.sql.clone()).await;
                if *generation.peek() != task_request.generation {
                    return;
                }
                active_task.set(None);
                match response {
                    Ok(value) => {
                        last_success.set(Some(Rc::new(QuerySuccess {
                            request: task_request,
                            output: parse_query_output(value),
                            duration_ms: started.elapsed().as_millis(),
                        })));
                        activity.set(Activity::Succeeded);
                    }
                    Err(error) => activity.set(Activity::Failed {
                        request: task_request,
                        error,
                    }),
                }
            });
            active_task.set(Some(task));
        }
    });

    let cancel = use_callback(move |()| {
        if let Some(task) = active_task.take() {
            task.cancel();
        }
        generation.set(generation().saturating_add(1));
        if let Some(request) = submitted.read().clone() {
            activity.set(Activity::Cancelled(request));
        }
    });

    let state = activity.read().clone();
    let running = matches!(state, Activity::Running(_));
    let retained = last_success.read().clone();
    let scope_label = scope_id.as_deref().unwrap_or("default");

    rsx! {
        section { class: "semantic-query semantic-route-stack",
            PageHeader {
                title: "Query workbench",
                description: format!("Run read-only SQL against scope '{scope_label}'. Results stay local to this page."),
            }
            QueryEditor {
                draft: draft.read().clone(),
                error: validation_error.read().clone(),
                title: "SQL editor",
                description: format!("One SELECT statement only. Start with LIMIT {DEFAULT_LIMIT}; this UI retains at most {MAX_ROWS} returned rows."),
                label: "Read-only SQL query",
                editor_id: "semantic-query-workbench-sql",
                running,
                show_clear: false,
                on_change: move |value| {
                    draft.set(value);
                    validation_error.set(None);
                },
                on_run: move |_| submit.call(()),
                on_cancel: move |_| cancel.call(()),
                on_clear: move |_| {},
            }
            section { class: "semantic-query__results", aria_busy: running, aria_label: "Query results",
                div { class: "semantic-query__results-heading",
                    h2 { "Results" }
                    if let Activity::Running(request) = &state {
                        code { class: "semantic-query__generation", "Run #{request.generation}" }
                    }
                }
                match &state {
                    Activity::Idle => rsx! { EmptyState {
                        title: "Ready to query",
                        description: "Review the SQL, then run it with the button or Ctrl/Command+Enter.",
                    } },
                    Activity::Running(_) => rsx! { RefreshingIndicator {
                        label: if retained.is_some() { "Running query; previous results remain visible" } else { "Running query" },
                    } },
                    Activity::Cancelled(request) => rsx! { InlineNotice {
                        variant: NoticeVariant::Warning,
                        title: "Query cancelled",
                        message: format!("Run #{} was cancelled. The editor and last successful result were preserved.", request.generation),
                    } },
                    Activity::Failed { error, .. } => rsx! { ErrorState {
                        title: "Query failed",
                        message: error.clone(),
                        retry_label: "Run again",
                        on_retry: move |_| submit.call(()),
                    } },
                    Activity::Succeeded => rsx! {},
                }
                if let Some(success) = retained {
                    QueryResultView {
                        success,
                        retained: running || matches!(state, Activity::Failed { .. }),
                    }
                }
            }
        }
    }
}

#[component]
fn QueryResultView(success: Rc<QuerySuccess>, retained: bool) -> Element {
    match &success.output {
        QueryOutput::Rows(rows) if rows.total == 0 => rsx! {
            QueryResultMeta { success: success.clone(), retained }
            EmptyState { title: "Query completed with no rows", description: "The SELECT ran successfully but did not match any records." }
        },
        QueryOutput::Rows(rows) => rsx! {
            QueryResultMeta { success: success.clone(), retained }
            QueryRowsView { data: Rc::new(rows.clone()) }
        },
        QueryOutput::NonRow { kind, fields } => rsx! {
            QueryResultMeta { success: success.clone(), retained }
            InlineNotice {
                variant: NoticeVariant::Warning,
                title: "Non-row response",
                message: format!("The backend returned a '{kind}' outcome; read-only mode expected SELECT rows."),
            }
            pre { class: "semantic-query__raw-result", tabindex: "0", "{fields:#?}" }
        },
        QueryOutput::Unexpected(value) => rsx! {
            QueryResultMeta { success: success.clone(), retained }
            InlineNotice {
                variant: NoticeVariant::Error,
                title: "Unexpected response",
                message: "The backend response did not match the query result contract.",
            }
            pre { class: "semantic-query__raw-result", tabindex: "0", "{value:#?}" }
        },
    }
}

#[component]
fn QueryResultMeta(success: Rc<QuerySuccess>, retained: bool) -> Element {
    let count = match &success.output {
        QueryOutput::Rows(rows) => Some(rows.total),
        _ => None,
    };
    rsx! { div { class: "semantic-query__result-meta", role: "status",
        span { "Run #{success.request.generation}" }
        if let Some(count) = count { span { "{count} row(s) returned" } }
        span { "{success.duration_ms} ms" }
        if retained { span { class: "semantic-query__stale-label", "Previous successful result" } }
    } }
}

#[component]
fn QueryRowsView(data: Rc<QueryRows>) -> Element {
    let column_rows = data.rows.clone();
    let columns = use_memo(move || derive_columns(&column_rows));
    let columns = columns.read().clone();
    let column_count = derive_column_count(&data.rows);
    let bounded =
        data.total > data.rows.len() || column_count > columns.len() || data.malformed > 0;
    rsx! {
        if bounded { InlineNotice {
            variant: NoticeVariant::Warning,
            title: "Result display is bounded",
            message: format!("Showing {} of {} rows and {} of {} columns. {} non-object row(s) could not be displayed.", data.rows.len(), data.total, columns.len(), column_count, data.malformed),
        } }
        div { class: "semantic-query-table-scroll", role: "region", aria_label: "Scrollable query result table", tabindex: "0",
            table { class: "semantic-query-table",
                caption { class: "semantic-visually-hidden", "Query result rows" }
                thead { tr { for column in columns.iter() { th { scope: "col", "{column}" } } } }
                tbody { for (index, row) in data.rows.iter().enumerate() { tr { key: "{index}",
                    for column in columns.iter() { td {
                        if let Some(value) = row.get(column) { QueryValue { value: value.clone() } }
                        else { span { class: "semantic-query-value semantic-query-value--missing", title: "No value in this row", "—" } }
                    } }
                } } }
            }
        }
    }
}

#[component]
fn QueryValue(value: Value) -> Element {
    match value {
        Value::Void => {
            rsx! { span { class: "semantic-query-value semantic-query-value--null", title: "No value", "void" } }
        }
        Value::Null => {
            rsx! { span { class: "semantic-query-value semantic-query-value--null", "null" } }
        }
        Value::Bool(value) => rsx! { code { class: "semantic-query-value", "{value}" } },
        Value::String(value) => rsx! { span { class: "semantic-query-value", "{value}" } },
        Value::Bytes(value) => {
            let length = value.len();
            rsx! { span { class: "semantic-query-value", "{length} bytes" } }
        }
        Value::List(value) => {
            rsx! { StructuredValue { label: format!("{} item(s)", value.len()), value: Value::List(value) } }
        }
        Value::Object(value) => {
            rsx! { StructuredValue { label: format!("{} field(s)", value.len()), value: Value::Object(value) } }
        }
        Value::Map(value) => {
            rsx! { StructuredValue { label: format!("{} entry/entries", value.len()), value: Value::Map(value) } }
        }
        other => {
            let display = value_string(&other);
            rsx! { code { class: "semantic-query-value", "{display}" } }
        }
    }
}

#[component]
fn StructuredValue(label: String, value: Value) -> Element {
    let preview = bounded_debug(&value);
    rsx! { details { class: "semantic-query-value semantic-query-value--structured",
        summary { "{label}" }
        pre { "{preview}" }
    } }
}

fn parse_query_output(value: Value) -> QueryOutput {
    let Value::Object(mut object) = value else {
        return QueryOutput::Unexpected(value);
    };
    let kind = object
        .remove("kind")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    if kind != "select" {
        return QueryOutput::NonRow {
            kind,
            fields: object,
        };
    }
    let Some(Value::List(values)) = object.remove("rows") else {
        return QueryOutput::Unexpected(Value::Object(object));
    };
    let total = values.len();
    let mut malformed = 0;
    let rows = values
        .into_iter()
        .filter_map(|value| match value {
            Value::Object(row) => Some(row),
            _ => {
                malformed += 1;
                None
            }
        })
        .take(MAX_ROWS)
        .collect::<Vec<_>>()
        .into();
    QueryOutput::Rows(QueryRows {
        rows,
        total,
        malformed,
    })
}

fn derive_columns(rows: &[Object]) -> Rc<[String]> {
    rows.iter()
        .flat_map(|row| row.keys().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_COLUMNS)
        .collect::<Vec<_>>()
        .into()
}

fn derive_column_count(rows: &[Object]) -> usize {
    rows.iter()
        .flat_map(|row| row.keys())
        .collect::<BTreeSet<_>>()
        .len()
}

fn bounded_debug(value: &Value) -> String {
    const MAX_CHARS: usize = 600;
    let debug = format!("{value:#?}");
    if debug.chars().count() <= MAX_CHARS {
        debug
    } else {
        format!("{}…", debug.chars().take(MAX_CHARS).collect::<String>())
    }
}

/// Conservative client-side guard. The RPC/database remains the security boundary.
fn validate_read_only_sql(sql: &str) -> std::result::Result<String, String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("Enter a SELECT query first.".to_string());
    }
    let sanitized = sanitize_sql(trimmed)?;
    let statement = sanitized.trim();
    let statement = match statement.find(';') {
        Some(separator)
            if !statement[separator + 1..].trim().is_empty()
                || statement[..separator].contains(';') =>
        {
            return Err("Only one SELECT statement is allowed.".to_string());
        }
        Some(separator) => &statement[..separator],
        None => statement,
    };
    let words = statement
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if words.first().map(String::as_str) != Some("select") {
        return Err("The workbench accepts one read-only SELECT statement only.".to_string());
    }
    const REJECTED: &[&str] = &[
        "alter", "attach", "call", "copy", "create", "delete", "detach", "drop", "execute",
        "grant", "insert", "into", "merge", "pragma", "replace", "revoke", "truncate", "update",
        "vacuum",
    ];
    if let Some(keyword) = words.iter().find(|word| REJECTED.contains(&word.as_str())) {
        return Err(format!(
            "The keyword '{keyword}' is not allowed in read-only SQL."
        ));
    }
    Ok(trimmed.trim_end_matches(';').trim_end().to_string())
}

fn sanitize_sql(sql: &str) -> std::result::Result<String, String> {
    let mut output = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\'' | '"' => {
                let quote = character;
                output.push(' ');
                let mut closed = false;
                while let Some(character) = chars.next() {
                    if character == quote {
                        if chars.peek() == Some(&quote) {
                            chars.next();
                            continue;
                        }
                        closed = true;
                        break;
                    }
                }
                if !closed {
                    return Err("The SQL contains an unterminated quoted value.".to_string());
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                for character in chars.by_ref() {
                    if character == '\n' {
                        output.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                let mut closed = false;
                for character in chars.by_ref() {
                    if previous == '*' && character == '/' {
                        closed = true;
                        break;
                    }
                    previous = character;
                }
                if !closed {
                    return Err("The SQL contains an unterminated comment.".to_string());
                }
                output.push(' ');
            }
            _ => output.push(character),
        }
    }
    Ok(output)
}

async fn run_query(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    query: String,
) -> std::result::Result<Value, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert("query", Value::String(query));
    payload.insert("format", Value::String("sql".to_string()));
    client
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await
        .map_err(|error| error.to_string())
}
