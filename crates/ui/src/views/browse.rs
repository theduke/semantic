use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::builtin::DEFAULT_COLLECTION;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{
    EntityDisplayMode, EntityDisplayRenderer,
    components::{EmptyState, InlineNotice, LoadingSkeleton, NoticeVariant, RefreshingIndicator},
    use_active_scope_id, use_rpc_client, use_ui_catalog_context,
};

use crate::{
    components::{
        DataToolbar, EntityExplorer, EntityResults, PageHeader, Pagination, QueryEditor,
        ResultDensity,
    },
    views::Route,
};

const DEFAULT_VIEW: &str = "cards";
const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE: usize = 1_000_000;
const MAX_PORTABLE_SQL_BYTES: usize = 1_500;
const INLINE_SQL_PREFIX: &str = "inline:";
const RELATION_CLASS_ID: &str = "semantic:relation";

#[derive(Clone, Debug, PartialEq, Eq)]
struct BrowseQueryKey {
    scope_id: Option<String>,
    collection: String,
    query: std::result::Result<String, String>,
    page: usize,
    page_size: usize,
    display_mode: EntityDisplayMode,
    renderer: EntityDisplayRenderer,
}

#[derive(Clone)]
struct BrowseQueryResponse {
    key: BrowseQueryKey,
    result: std::result::Result<Rc<[Object]>, String>,
}

#[component]
pub fn BrowsePage(
    collection: Option<String>,
    view: Option<String>,
    renderer: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
    sql: Option<String>,
) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog_signal = use_ui_catalog_context().catalog_signal();
    let collection_name = collection
        .clone()
        .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
    let display_mode = parse_display_mode(view.as_deref());
    let renderer_mode = parse_renderer(renderer.as_deref());
    let page = clamp_page(page.unwrap_or(0));
    let page_size = clamp_page_size(page_size.unwrap_or(DEFAULT_PAGE_SIZE));
    let custom_sql = sql.is_some();
    let collections = use_memo(move || -> Rc<[String]> {
        catalog_signal
            .read()
            .as_ref()
            .map(|catalog| {
                catalog
                    .collections()
                    .map(|collection| collection.name.clone())
                    .collect::<Vec<_>>()
                    .into()
            })
            .unwrap_or_default()
    });

    let applied_query = resolve_applied_query(&collection_name, page_size, page, sql.as_deref());
    let draft_source = match sql.as_deref() {
        Some(reference) => load_sql_reference(reference).unwrap_or_default(),
        None => default_query(&collection_name, page_size, page),
    };
    let mut sql_input = use_signal(|| draft_source.clone());
    let mut sql_error = use_signal(|| None::<String>);
    let mut advanced_open = use_signal(|| custom_sql);
    let mut grid_columns = use_signal(|| 1_usize);
    let mut density = use_signal(ResultDensity::default);

    use_effect(use_reactive(
        (&draft_source, &custom_sql),
        move |(draft_source, custom_sql)| {
            sql_input.set(draft_source);
            sql_error.set(None);
            advanced_open.set(custom_sql);
        },
    ));

    let query_key = use_memo(use_reactive(
        &BrowseQueryKey {
            scope_id: scope_id.clone(),
            collection: collection_name.clone(),
            query: applied_query,
            page,
            page_size,
            display_mode,
            renderer: renderer_mode,
        },
        |key| key,
    ));
    let mut resource = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            let key = query_key();
            async move {
                let result = match &key.query {
                    Ok(query) => run_query(client, key.scope_id.clone(), query.clone())
                        .await
                        .map(|rows| rows.into()),
                    Err(error) => Err(error.clone()),
                };
                BrowseQueryResponse { key, result }
            }
        }
    });

    let current_key = query_key();
    let loading = *resource.state().read() == UseResourceState::Pending;
    let response = resource.read().clone();
    let current_response = response
        .as_ref()
        .filter(|response| response.key == current_key)
        .cloned();
    let retained_rows = loading
        .then(|| {
            response
                .as_ref()
                .and_then(|response| response.result.as_ref().ok())
                .cloned()
        })
        .flatten();
    let current_rows = current_response
        .as_ref()
        .and_then(|response| response.result.as_ref().ok().map(|rows| rows.clone()));
    let rows = if loading {
        retained_rows.clone()
    } else {
        current_rows.clone().or(retained_rows.clone())
    };
    let error = (!loading)
        .then(|| current_response.as_ref())
        .flatten()
        .and_then(|response| response.result.as_ref().err().cloned());
    let successful_empty = !loading
        && current_response
            .as_ref()
            .and_then(|response| response.result.as_ref().ok())
            .is_some_and(|rows| rows.is_empty());
    let has_more = current_rows
        .as_ref()
        .is_some_and(|rows| rows.len() == page_size);
    let non_portable_sql = sql
        .as_deref()
        .is_some_and(|reference| !reference.starts_with(INLINE_SQL_PREFIX));
    let row_count_label = rows.as_ref().map(|rows| {
        format!(
            "{} {} shown",
            rows.len(),
            if rows.len() == 1 { "row" } else { "rows" }
        )
    });

    rsx! {
        div { class: "semantic-page semantic-browse",
            PageHeader {
                title: "Browse entities",
                description: "Explore a catalog collection with a predictable paged view, or switch intentionally to read-only SQL.",
                breadcrumbs: rsx! {
                    Link { to: Route::HomePage, "Workspace" }
                    span { aria_hidden: "true", "/" }
                    span { "Browse" }
                },
                actions: rsx! {
                    Link { class: "semantic-button-link", to: Route::CreateEntityPage, "Create entity" }
                    Link { class: "semantic-button-link semantic-button-link--secondary", to: Route::UploadPage, "Upload files" }
                }
            }

            EntityExplorer {
                refreshing: loading && rows.is_some(),
                toolbar: rsx! {
                    DataToolbar {
                        collection: collection_name.clone(),
                        collections: collections(),
                        display_mode,
                        renderer: renderer_mode,
                        grid_columns: *grid_columns.read(),
                        density: *density.read(),
                        advanced_open: *advanced_open.read(),
                        custom_query: custom_sql,
                        on_collection_change: {
                            let view = view_string(display_mode);
                            let renderer = renderer_string(renderer_mode);
                            move |next_collection: String| {
                                navigator().push(browse_route(
                                    Some(next_collection),
                                    Some(view.clone()),
                                    Some(renderer.clone()),
                                    Some(0),
                                    Some(page_size),
                                    None,
                                ));
                            }
                        },
                        on_display_mode_change: {
                            let collection = collection.clone();
                            let sql = sql.clone();
                            move |mode| {
                                navigator().push(browse_route(
                                    collection.clone(),
                                    Some(view_string(mode)),
                                    Some(renderer_string(renderer_mode)),
                                    Some(page),
                                    Some(page_size),
                                    sql.clone(),
                                ));
                            }
                        },
                        on_renderer_change: {
                            let collection = collection.clone();
                            let sql = sql.clone();
                            move |renderer| {
                                navigator().push(browse_route(
                                    collection.clone(),
                                    Some(view_string(display_mode)),
                                    Some(renderer_string(renderer)),
                                    Some(page),
                                    Some(page_size),
                                    sql.clone(),
                                ));
                            }
                        },
                        on_grid_columns_change: move |columns: usize| grid_columns.set(columns.clamp(1, 3)),
                        on_density_change: move |next_density: ResultDensity| density.set(next_density),
                        on_advanced_open_change: move |open: bool| advanced_open.set(open),
                    }
                },
                advanced: (*advanced_open.read()).then(|| rsx! {
                    QueryEditor {
                        draft: sql_input.read().clone(),
                        error: sql_error.read().clone(),
                        non_portable: non_portable_sql,
                        on_change: move |draft| {
                            sql_input.set(draft);
                            sql_error.set(None);
                        },
                        on_run: {
                            let collection = collection.clone();
                            move |_| {
                                let draft = sql_input.read().clone();
                                match validate_read_only_sql(&draft) {
                                    Ok(query) => {
                                        let reference = store_sql_reference(&query);
                                        navigator().push(browse_route(
                                            collection.clone(),
                                            Some(view_string(display_mode)),
                                            Some(renderer_string(renderer_mode)),
                                            Some(0),
                                            Some(page_size),
                                            Some(reference),
                                        ));
                                    }
                                    Err(error) => sql_error.set(Some(error)),
                                }
                            }
                        },
                        on_clear: {
                            let collection = collection.clone();
                            move |_| {
                                navigator().push(browse_route(
                                    collection.clone(),
                                    Some(view_string(display_mode)),
                                    Some(renderer_string(renderer_mode)),
                                    Some(0),
                                    Some(page_size),
                                    None,
                                ));
                            }
                        },
                    }
                }),

                if custom_sql {
                    InlineNotice {
                        title: "Advanced SQL controls this result set",
                        message: "Pagination is hidden because LIMIT, OFFSET, and ordering belong to the query.",
                        variant: NoticeVariant::Info,
                        action_label: "Edit query",
                        on_action: move |_| advanced_open.set(true),
                    }
                }

                if loading {
                    if rows.is_some() {
                        RefreshingIndicator { label: "Refreshing entity results" }
                    } else {
                        LoadingSkeleton { label: "Loading entity results", line_count: 5 }
                    }
                }

                if let Some(error) = error.clone() {
                    InlineNotice {
                        title: "Could not load these results",
                        message: error,
                        variant: NoticeVariant::Error,
                        action_label: "Retry",
                        on_action: move |_| resource.restart(),
                    }
                }

                if successful_empty {
                    if custom_sql {
                        EmptyState {
                            title: "No rows matched this query",
                            description: "Edit the SQL or return to the collection query.",
                            action_label: "Edit query",
                            on_action: move |_| advanced_open.set(true),
                        }
                    } else {
                        EmptyState {
                            title: "This collection is empty",
                            description: "Create an entity or choose another collection from the toolbar.",
                            action_label: "Create entity",
                            on_action: move |_| { navigator().push(Route::CreateEntityPage); },
                        }
                    }
                } else if let Some(rows) = rows {
                    div { class: "semantic-browse__result-summary",
                        span { "{row_count_label.as_deref().unwrap_or_default()}" }
                        if error.is_some() {
                            span { "Showing the last successful result." }
                        }
                    }
                    EntityResults {
                        rows: rows.clone(),
                        display_mode,
                        renderer: renderer_mode,
                        collection: Some(collection_name.clone()),
                        grid_columns: *grid_columns.read(),
                        density: *density.read(),
                    }
                    if !custom_sql {
                        Pagination {
                            page,
                            page_size,
                            has_more,
                            disabled: loading,
                            on_page_change: {
                                let collection = collection.clone();
                                move |next_page| {
                                    navigator().push(browse_route(
                                        collection.clone(),
                                        Some(view_string(display_mode)),
                                        Some(renderer_string(renderer_mode)),
                                        Some(clamp_page(next_page)),
                                        Some(page_size),
                                        None,
                                    ));
                                }
                            },
                            on_page_size_change: {
                                let collection = collection.clone();
                                move |next_page_size| {
                                    navigator().push(browse_route(
                                        collection.clone(),
                                        Some(view_string(display_mode)),
                                        Some(renderer_string(renderer_mode)),
                                        Some(0),
                                        Some(clamp_page_size(next_page_size)),
                                        None,
                                    ));
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

fn browse_route(
    collection: Option<String>,
    view: Option<String>,
    renderer: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
    sql: Option<String>,
) -> Route {
    Route::BrowsePage {
        collection,
        view,
        renderer,
        page,
        page_size,
        sql,
    }
}

async fn run_query(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    query: String,
) -> std::result::Result<Vec<Object>, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = scope_id {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert("query", Value::String(query));
    payload.insert("format", Value::String("sql".to_string()));
    let response = client
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())?;
    let Value::Object(object) = response else {
        return Err("query response must be an object".to_string());
    };
    let Some(Value::List(rows)) = object.get("rows") else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .filter_map(|row| match row {
            Value::Object(row) => Some(row.clone()),
            _ => None,
        })
        .collect())
}

fn resolve_applied_query(
    collection: &str,
    page_size: usize,
    page: usize,
    reference: Option<&str>,
) -> std::result::Result<String, String> {
    match reference {
        Some(reference) => {
            let query = load_sql_reference(reference).ok_or_else(|| {
                format!(
                    "This SQL link depends on browser-local storage that is unavailable. Edit the query or return to the collection query. Reference: {reference}"
                )
            })?;
            validate_read_only_sql(&query)
        }
        None => Ok(default_query(collection, page_size, page)),
    }
}

fn default_query(collection: &str, page_size: usize, page: usize) -> String {
    let relation_filter = if collection == DEFAULT_COLLECTION {
        format!(" WHERE type != '{RELATION_CLASS_ID}'")
    } else {
        String::new()
    };

    format!(
        "SELECT * FROM {}{} LIMIT {} OFFSET {}",
        sql_ident(collection),
        relation_filter,
        clamp_page_size(page_size),
        clamp_page(page).saturating_mul(clamp_page_size(page_size))
    )
}

fn clamp_page(page: usize) -> usize {
    page.min(MAX_PAGE)
}

fn clamp_page_size(page_size: usize) -> usize {
    match page_size {
        0..=25 => 25,
        26..=50 => 50,
        51..=100 => 100,
        _ => 200,
    }
}

fn sql_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn parse_display_mode(value: Option<&str>) -> EntityDisplayMode {
    match value {
        Some("table") => EntityDisplayMode::Table,
        _ => EntityDisplayMode::Card,
    }
}

fn parse_renderer(value: Option<&str>) -> EntityDisplayRenderer {
    match value {
        Some("table") => EntityDisplayRenderer::Table,
        _ => EntityDisplayRenderer::Custom,
    }
}

fn view_string(display_mode: EntityDisplayMode) -> String {
    match display_mode {
        EntityDisplayMode::Card => DEFAULT_VIEW.to_string(),
        EntityDisplayMode::Table => "table".to_string(),
    }
}

fn renderer_string(renderer: EntityDisplayRenderer) -> String {
    match renderer {
        EntityDisplayRenderer::Custom => "custom".to_string(),
        EntityDisplayRenderer::Table => "table".to_string(),
    }
}

/// Conservative client-side guard. The RPC/database remains the security boundary.
fn validate_read_only_sql(sql: &str) -> std::result::Result<String, String> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err("Enter a SELECT query first.".to_string());
    }

    let sanitized = sanitize_sql_for_validation(trimmed)?;
    let statement = sanitized.trim();
    let statement = match statement.find(';') {
        Some(separator) => {
            if !statement[separator + 1..].trim().is_empty() || statement[..separator].contains(';')
            {
                return Err("Only one SELECT statement is allowed.".to_string());
            }
            &statement[..separator]
        }
        None => statement,
    };
    let words = statement
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if words.first().map(String::as_str) != Some("select") {
        return Err("Browse accepts one read-only SELECT statement only.".to_string());
    }

    const REJECTED_KEYWORDS: &[&str] = &[
        "alter", "attach", "call", "copy", "create", "delete", "detach", "drop", "execute",
        "grant", "insert", "into", "merge", "pragma", "replace", "revoke", "truncate", "update",
        "vacuum",
    ];
    if let Some(keyword) = words
        .iter()
        .find(|word| REJECTED_KEYWORDS.contains(&word.as_str()))
    {
        return Err(format!(
            "The keyword '{keyword}' is not allowed in Browse read-only SQL."
        ));
    }

    Ok(trimmed.trim_end_matches(';').trim_end().to_string())
}

fn sanitize_sql_for_validation(sql: &str) -> std::result::Result<String, String> {
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

fn stable_hash_hex(sql: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in sql.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn store_sql_reference(sql: &str) -> String {
    let hash = stable_hash_hex(sql);
    store_sql(&hash, sql);
    if sql.len() <= MAX_PORTABLE_SQL_BYTES {
        format!("{INLINE_SQL_PREFIX}{sql}")
    } else {
        hash
    }
}

fn load_sql_reference(reference: &str) -> Option<String> {
    reference
        .strip_prefix(INLINE_SQL_PREFIX)
        .map(str::to_string)
        .or_else(|| load_sql(reference))
}

fn storage_key(hash: &str) -> String {
    format!("semantic:browse:sql:{hash}")
}

#[cfg(target_arch = "wasm32")]
fn store_sql(hash: &str, sql: &str) {
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(&storage_key(hash), sql);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn store_sql(hash: &str, sql: &str) {
    FALLBACK_SQL_STORAGE.with(|storage| {
        storage
            .borrow_mut()
            .insert(storage_key(hash), sql.to_string());
    });
}

#[cfg(target_arch = "wasm32")]
fn load_sql(hash: &str) -> Option<String> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(&storage_key(hash)).ok().flatten())
}

#[cfg(not(target_arch = "wasm32"))]
fn load_sql(hash: &str) -> Option<String> {
    FALLBACK_SQL_STORAGE.with(|storage| storage.borrow().get(&storage_key(hash)).cloned())
}

thread_local! {
    static FALLBACK_SQL_STORAGE: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_entities_query_excludes_relation_entities() {
        assert_eq!(
            default_query(DEFAULT_COLLECTION, 50, 2),
            "SELECT * FROM \"entities\" WHERE type != 'semantic:relation' LIMIT 50 OFFSET 100"
        );
    }

    #[test]
    fn default_non_entities_query_does_not_add_entity_type_filter() {
        assert_eq!(
            default_query("events", 25, 1),
            "SELECT * FROM \"events\" LIMIT 25 OFFSET 25"
        );
    }

    #[test]
    fn route_inputs_are_clamped_to_bounded_options() {
        assert_eq!(clamp_page(usize::MAX), MAX_PAGE);
        assert_eq!(clamp_page_size(0), 25);
        assert_eq!(clamp_page_size(26), 50);
        assert_eq!(clamp_page_size(75), 100);
        assert_eq!(clamp_page_size(usize::MAX), 200);
    }

    #[test]
    fn read_only_validation_rejects_mutations_and_multiple_statements() {
        assert!(validate_read_only_sql("DELETE FROM entities").is_err());
        assert!(validate_read_only_sql("SELECT * FROM entities; DELETE FROM entities").is_err());
        assert!(validate_read_only_sql("SELECT * INTO backup FROM entities").is_err());
        assert!(validate_read_only_sql("SELECT 'delete; update' FROM entities;").is_ok());
        assert!(validate_read_only_sql("-- context\n SELECT * FROM entities").is_ok());
    }

    #[test]
    fn short_sql_references_are_portable_and_legacy_hashes_still_load() {
        let query = "SELECT * FROM entities LIMIT 10";
        let reference = store_sql_reference(query);
        assert!(reference.starts_with(INLINE_SQL_PREFIX));
        assert_eq!(load_sql_reference(&reference).as_deref(), Some(query));

        let hash = stable_hash_hex(query);
        assert_eq!(load_sql_reference(&hash).as_deref(), Some(query));
    }
}
