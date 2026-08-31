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
        ResultDensity, StructuredQuery, StructuredQueryBuilder, compile_structured_predicate,
        decode_structured_query, encode_structured_query, fields_for_collection,
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
    filters: Option<String>,
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
    let decoded_filters = filters.as_deref().map(decode_structured_query).transpose();
    let route_filter_error = decoded_filters.as_ref().err().cloned();
    let applied_filters = decoded_filters
        .as_ref()
        .ok()
        .and_then(|query| query.clone())
        .unwrap_or_default();
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
    let query_fields = use_memo({
        let collection_name = collection_name.clone();
        move || {
            catalog_signal
                .read()
                .as_ref()
                .map(|catalog| fields_for_collection(catalog, &collection_name))
                .unwrap_or_default()
        }
    });

    let applied_query = match &decoded_filters {
        Err(error) if !custom_sql => Err(error.clone()),
        _ => resolve_applied_query(
            &collection_name,
            page_size,
            page,
            sql.as_deref(),
            (!custom_sql).then_some(&applied_filters),
            &query_fields(),
        ),
    };
    let draft_source = match sql.as_deref() {
        Some(reference) => load_sql_reference(reference).unwrap_or_default(),
        None => default_query(&collection_name, page_size, page),
    };
    let mut sql_input = use_signal(|| draft_source.clone());
    let mut sql_error = use_signal(|| None::<String>);
    let mut advanced_open = use_signal(|| custom_sql);
    let mut filters_open = use_signal(|| filters.is_some() && !custom_sql);
    let mut filter_draft = use_signal(|| applied_filters.clone());
    let mut filter_error = use_signal(|| route_filter_error.clone());
    let mut grid_columns = use_signal(|| 1_usize);
    let mut density = use_signal(ResultDensity::default);

    use_effect(use_reactive(
        (
            &draft_source,
            &custom_sql,
            &applied_filters,
            &route_filter_error,
        ),
        move |(draft_source, custom_sql, applied_filters, route_filter_error)| {
            sql_input.set(draft_source);
            sql_error.set(None);
            advanced_open.set(custom_sql);
            filter_draft.set(applied_filters);
            filter_error.set(route_filter_error);
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
                        filters_open: *filters_open.read(),
                        active_filter_count: applied_filters.active_count(),
                        show_filters: true,
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
                                    None,
                                ));
                            }
                        },
                        on_display_mode_change: {
                            let collection = collection.clone();
                            let filters = filters.clone();
                            let sql = sql.clone();
                            move |mode| {
                                navigator().push(browse_route(
                                    collection.clone(),
                                    Some(view_string(mode)),
                                    Some(renderer_string(renderer_mode)),
                                    Some(page),
                                    Some(page_size),
                                    filters.clone(),
                                    sql.clone(),
                                ));
                            }
                        },
                        on_renderer_change: {
                            let collection = collection.clone();
                            let filters = filters.clone();
                            let sql = sql.clone();
                            move |renderer| {
                                navigator().push(browse_route(
                                    collection.clone(),
                                    Some(view_string(display_mode)),
                                    Some(renderer_string(renderer)),
                                    Some(page),
                                    Some(page_size),
                                    filters.clone(),
                                    sql.clone(),
                                ));
                            }
                        },
                        on_grid_columns_change: move |columns: usize| grid_columns.set(columns.clamp(1, 3)),
                        on_density_change: move |next_density: ResultDensity| density.set(next_density),
                        on_advanced_open_change: move |open: bool| {
                            advanced_open.set(open);
                            if open { filters_open.set(false); }
                        },
                        on_filters_open_change: move |open: bool| {
                            filters_open.set(open);
                            if open { advanced_open.set(false); }
                        },
                    }
                },
                advanced: if *filters_open.read() {
                    Some(rsx! {
                        div { class: "semantic-query-builder-panel",
                            StructuredQueryBuilder {
                                draft: filter_draft(),
                                fields: query_fields(),
                                error: filter_error(),
                                on_change: move |next| {
                                    filter_draft.set(next);
                                    filter_error.set(None);
                                },
                            }
                            div { class: "semantic-query-builder__actions",
                                dxcomp::Button {
                                    onclick: {
                                        let collection = collection.clone();
                                        move |_| {
                                            let draft = filter_draft();
                                            match compile_structured_predicate(&draft, &query_fields(), None, &["title"]) {
                                                Ok(_) if draft.active_count() == 0 => {
                                                    navigator().push(browse_route(
                                                        collection.clone(), Some(view_string(display_mode)), Some(renderer_string(renderer_mode)),
                                                        Some(0), Some(page_size), None, None,
                                                    ));
                                                }
                                                Ok(_) => match encode_structured_query(&draft) {
                                                    Ok(encoded) => {
                                                        navigator().push(browse_route(
                                                            collection.clone(), Some(view_string(display_mode)), Some(renderer_string(renderer_mode)),
                                                            Some(0), Some(page_size), Some(encoded), None,
                                                        ));
                                                    }
                                                    Err(error) => filter_error.set(Some(error)),
                                                },
                                                Err(error) => filter_error.set(Some(error)),
                                            }
                                        }
                                    },
                                    "Apply filters"
                                }
                                dxcomp::Button {
                                    variant: dxcomp::ButtonVariant::Outline,
                                    onclick: {
                                        let collection = collection.clone();
                                        move |_| {
                                            navigator().push(browse_route(
                                                collection.clone(), Some(view_string(display_mode)), Some(renderer_string(renderer_mode)),
                                                Some(0), Some(page_size), None, None,
                                            ));
                                        }
                                    },
                                    "Clear filters"
                                }
                            }
                        }
                    })
                } else if *advanced_open.read() {
                    Some(rsx! { QueryEditor {
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
                                            None,
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
                                    None,
                                ));
                            }
                        },
                    } })
                } else { None },

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
                    } else if applied_filters.active_count() > 0 {
                        EmptyState {
                            title: "No entities match these filters",
                            description: "Edit or clear the visual filters to broaden the result set.",
                            action_label: "Edit filters",
                            on_action: move |_| filters_open.set(true),
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
                                let filters = filters.clone();
                                move |next_page| {
                                    navigator().push(browse_route(
                                        collection.clone(),
                                        Some(view_string(display_mode)),
                                        Some(renderer_string(renderer_mode)),
                                        Some(clamp_page(next_page)),
                                        Some(page_size),
                                        filters.clone(),
                                        None,
                                    ));
                                }
                            },
                            on_page_size_change: {
                                let collection = collection.clone();
                                let filters = filters.clone();
                                move |next_page_size| {
                                    navigator().push(browse_route(
                                        collection.clone(),
                                        Some(view_string(display_mode)),
                                        Some(renderer_string(renderer_mode)),
                                        Some(0),
                                        Some(clamp_page_size(next_page_size)),
                                        filters.clone(),
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
    filters: Option<String>,
    sql: Option<String>,
) -> Route {
    Route::BrowsePage {
        collection,
        view,
        renderer,
        page,
        page_size,
        filters,
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
    filters: Option<&StructuredQuery>,
    fields: &[crate::components::QueryField],
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
        None => {
            let predicate = filters
                .map(|filters| compile_structured_predicate(filters, fields, None, &["title"]))
                .transpose()?
                .flatten();
            Ok(collection_query(
                collection,
                page_size,
                page,
                predicate.as_deref(),
            ))
        }
    }
}

fn default_query(collection: &str, page_size: usize, page: usize) -> String {
    collection_query(collection, page_size, page, None)
}

fn collection_query(
    collection: &str,
    page_size: usize,
    page: usize,
    predicate: Option<&str>,
) -> String {
    let mut predicates = Vec::new();
    if collection == DEFAULT_COLLECTION {
        predicates.push(format!("type != '{RELATION_CLASS_ID}'"));
    }
    if let Some(predicate) = predicate {
        predicates.push(format!("({predicate})"));
    }
    let where_clause = if predicates.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", predicates.join(" AND "))
    };

    format!(
        "SELECT * FROM {}{} LIMIT {} OFFSET {}",
        sql_ident(collection),
        where_clause,
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
    use crate::components::{
        FilterGroup, FilterNode, FilterOperator, FilterRule, QueryField, QueryFieldKind,
    };

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

    #[test]
    fn structured_filters_keep_normal_pagination_and_default_constraints() {
        let filters = StructuredQuery {
            root: FilterGroup {
                children: vec![FilterNode::Rule(FilterRule {
                    field: "score".to_string(),
                    operator: FilterOperator::Greater,
                    values: vec!["7".to_string()],
                })],
                ..Default::default()
            },
            ..Default::default()
        };
        let fields = vec![QueryField {
            name: "score".to_string(),
            label: "Score".to_string(),
            description: None,
            kind: QueryFieldKind::SignedInteger,
            deprecated: false,
        }];
        let query = resolve_applied_query(DEFAULT_COLLECTION, 25, 2, None, Some(&filters), &fields)
            .unwrap();
        assert!(query.contains("type != 'semantic:relation'"));
        assert!(query.contains("(\"score\" > 7)"));
        assert!(query.ends_with("LIMIT 25 OFFSET 50"));
    }
}
