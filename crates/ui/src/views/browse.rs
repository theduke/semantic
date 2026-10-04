use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::query_ast::{QueryRequest, combine, order};
use dioxus::prelude::*;
use semantic_data::{attr::ATTR_CREATED_AT, builtin::DEFAULT_COLLECTION};
use semantic_data::{
    query::{BinaryOp, Expr, SelectQuery, SortDirection},
    value::{Object, Value},
};
use semantic_ui_core::{
    EntityDisplayMode, EntityDisplayRenderer,
    components::{EmptyState, InlineNotice, LoadingSkeleton, NoticeVariant, RefreshingIndicator},
    use_active_scope_id, use_rpc_client, use_ui_catalog_context,
};

use super::listing::{listing_predicate, listing_sql_predicate};
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
#[derive(Clone, Debug, PartialEq)]
struct BrowseQueryKey {
    scope_id: Option<String>,
    collection: String,
    query: std::result::Result<QueryRequest, String>,
    page: usize,
    page_size: usize,
    display_mode: EntityDisplayMode,
    renderer: EntityDisplayRenderer,
}

#[derive(Clone)]
struct BrowseQueryResponse {
    key: BrowseQueryKey,
    result: std::result::Result<Vec<Object>, String>,
    has_more: bool,
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
    let read_only = crate::virtual_collections::use_collection_read_only(&collection_name);
    let decoded_filters = filters.as_deref().map(decode_structured_query).transpose();
    let route_filter_error = decoded_filters.as_ref().err().cloned();
    let applied_filters = decoded_filters
        .as_ref()
        .ok()
        .and_then(|query| query.clone())
        .unwrap_or_default();
    let collections = crate::virtual_collections::use_collection_names();
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

    let listing_filter = listing_predicate(&collection_name, catalog_signal.read().as_ref());
    let applied_query = match &decoded_filters {
        Err(error) if !custom_sql => Err(error.clone()),
        _ => resolve_applied_query(
            &collection_name,
            page_size,
            page,
            sql.as_deref(),
            (!custom_sql).then_some(&applied_filters),
            &query_fields(),
            listing_filter.as_ref(),
        ),
    };
    let draft_source = match sql.as_deref() {
        Some(reference) => load_sql_reference(reference).unwrap_or_default(),
        None => default_query_sql(
            &collection_name,
            page_size,
            page,
            listing_sql_predicate(&collection_name, catalog_signal.read().as_ref()).as_deref(),
        ),
    };
    let mut sql_input = use_signal(|| draft_source.clone());
    let mut sql_error = use_signal(|| None::<String>);
    let mut advanced_open = use_signal(|| custom_sql);
    let mut filters_open = use_signal(|| filters.is_some() && !custom_sql);
    let mut filter_draft = use_signal(|| applied_filters.clone());
    let mut filter_error = use_signal(|| route_filter_error.clone());
    let mut grid_columns = use_signal(|| 2_usize);
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
    let mut results = use_signal(|| None::<BrowseQueryResponse>);
    let mut resource = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            let key = query_key();
            async move {
                let result = match &key.query {
                    Ok(query) => run_query(client, key.scope_id.clone(), query.clone()).await,
                    Err(error) => Err(error.clone()),
                };
                let has_more = result
                    .as_ref()
                    .is_ok_and(|rows| rows.len() == key.page_size);
                results.set(Some(BrowseQueryResponse {
                    key,
                    result,
                    has_more,
                }));
            }
        }
    });

    let current_key = query_key();
    let loading = *resource.state().read() == UseResourceState::Pending;
    let response = results.read();
    let current_response = response
        .as_ref()
        .filter(|response| response.key == current_key);
    let retained_rows = loading
        .then(|| {
            response
                .as_ref()
                .and_then(|response| response.result.as_ref().ok())
        })
        .flatten();
    let current_rows = current_response
        .as_ref()
        .and_then(|response| response.result.as_ref().ok());
    let rows = if loading {
        retained_rows
    } else {
        current_rows.or(retained_rows)
    };
    let error = (!loading)
        .then(|| current_response.as_ref())
        .flatten()
        .and_then(|response| response.result.as_ref().err().cloned());
    let successful_empty = !loading
        && current_response
            .as_ref()
            .and_then(|response| response.result.as_ref().ok())
            .is_some()
        && rows.as_ref().is_some_and(|rows| rows.is_empty());
    let has_more = current_response
        .as_ref()
        .is_some_and(|response| response.has_more);
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
                title: "Browse",
                description: "Find and explore your content.",
                breadcrumbs: rsx! {
                    Link { to: Route::HomePage, "Workspace" }
                    span { aria_hidden: "true", "/" }
                    span { "Browse" }
                },
                actions: rsx! {
                    if !read_only {
                        Link { class: "semantic-button-link", to: Route::CreateEntityPage, "New entity" }
                    }
                }
            }

            EntityExplorer {
                refreshing: loading && rows.is_some(),
                toolbar: rsx! {
                    DataToolbar {
                        collection: collection_name.clone(),
                        collections: collections.clone(),
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
                        title: "Raw SQL query",
                        description: "Run one read-only SELECT statement. SQL replaces filters and controls ordering and limits.",
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
                    } else if read_only {
                        EmptyState {
                            title: "This virtual collection is empty",
                            description: "Choose another collection or change the query to explore more records.",
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
                        on_delete: {
                            let displayed_key = response.as_ref().map(|response| response.key.clone());
                            move |target: semantic_ui_core::EntityTarget| {
                                if let Some(response) = results.write().as_mut()
                                    && Some(&response.key) == displayed_key.as_ref()
                                    && let Ok(rows) = &mut response.result
                                {
                                    rows.retain(|object| object.get("id").and_then(Value::as_str) != Some(target.id.as_str()));
                                }
                            }
                        },
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
    query: QueryRequest,
) -> std::result::Result<Vec<Object>, String> {
    let payload = query.payload(scope_id.as_deref());
    let response = client
        .invoke_value("semantic.db.query", payload)
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
    listing_filter: Option<&Expr>,
) -> std::result::Result<QueryRequest, String> {
    match reference {
        Some(reference) => {
            let query = load_sql_reference(reference).ok_or_else(|| format!("This SQL link depends on browser-local storage that is unavailable. Edit the query or return to the collection query. Reference: {reference}"))?;
            validate_read_only_sql(&query).map(QueryRequest::Sql)
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
                predicate.as_ref(),
                listing_filter,
            )
            .into())
        }
    }
}

// Text is a starting point for the explicit SQL editor, never the generated execution path.
fn default_query_sql(
    collection: &str,
    page_size: usize,
    page: usize,
    listing_filter: Option<&str>,
) -> String {
    let where_clause = listing_filter
        .map(|predicate| format!(" WHERE {predicate}"))
        .unwrap_or_default();
    format!(
        "SELECT * FROM {}{} ORDER BY {} DESC, id ASC LIMIT {} OFFSET {}",
        sql_ident(collection),
        where_clause,
        sql_ident(ATTR_CREATED_AT),
        clamp_page_size(page_size),
        clamp_page(page).saturating_mul(clamp_page_size(page_size))
    )
}

fn collection_query(
    collection: &str,
    page_size: usize,
    page: usize,
    predicate: Option<&Expr>,
    listing_filter: Option<&Expr>,
) -> SelectQuery {
    let mut query = SelectQuery::new()
        .with_collection(collection)
        .with_order_by(vec![
            order(None, ATTR_CREATED_AT, SortDirection::Desc),
            order(None, "id", SortDirection::Asc),
        ])
        .with_limit(clamp_page_size(page_size))
        .with_offset(clamp_page(page).saturating_mul(clamp_page_size(page_size)));
    query.predicate = combine(
        BinaryOp::And,
        listing_filter.into_iter().chain(predicate).cloned(),
    );
    query
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

    #[tokio::test]
    async fn generated_requests_use_ast_and_explicit_sql_stays_text() {
        let (client, calls) = crate::query_ast::tests::capture_client();
        let query = collection_query(
            "entities",
            25,
            0,
            None,
            listing_predicate("entities", None).as_ref(),
        );
        run_query(client.clone(), Some("scope".into()), query.clone().into())
            .await
            .unwrap();
        let sql = "SELECT * FROM entities LIMIT 10";
        let reference = store_sql_reference(sql);
        let raw =
            resolve_applied_query("entities", 25, 0, Some(&reference), None, &[], None).unwrap();
        run_query(client, Some("scope".into()), raw).await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        crate::query_ast::tests::assert_ast_call(&calls[0], &query, Some("scope"));
        let Value::Object(payload) = &calls[1].1 else {
            panic!("object")
        };
        assert_eq!(payload.get("query").and_then(Value::as_str), Some(sql));
        assert_eq!(payload.get("format").and_then(Value::as_str), Some("sql"));
        assert_eq!(
            payload.get("scope_id").and_then(Value::as_str),
            Some("scope")
        );
    }

    #[tokio::test]
    async fn listing_constraints_include_untyped_rows_and_apply_before_pagination() {
        use crate::query_ast::tests::{memory_db, row};
        let mut fixtures = (0..27)
            .map(|index| row(&format!("row-{index:02}"), []))
            .collect::<Vec<_>>();
        fixtures.extend([
            row(
                "0-hidden-id",
                [("type", Value::String("example:hidden".into()))],
            ),
            row("0-hidden-name", [("type", Value::String("hidden".into()))]),
        ]);
        let db = memory_db("query_test", fixtures).await;
        let mut snapshot = db.catalog().await.unwrap().to_storage_snapshot();
        let mut class = semantic_data::filestore::file_class();
        class.id = "example:hidden".into();
        class.name = "hidden".into();
        class.include_in_ui_listings = Some(false);
        snapshot
            .classes
            .push(semantic_db_core::catalog::StoredClass {
                lid: 100_000usize.into(),
                class,
            });
        let catalog = semantic_ui_core::UiCatalog::from_snapshot(snapshot);
        let predicate = listing_predicate("query_test", Some(&catalog));
        let query = collection_query("query_test", 25, 1, None, predicate.as_ref());
        let semantic_db_core::QueryResult::Select(rows) = db
            .query(semantic_data::query::QueryInput::from(query))
            .await
            .unwrap()
        else {
            panic!("AST rows")
        };
        assert_eq!(
            rows.iter()
                .map(|row| row.get("id").unwrap().as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["row-25", "row-26"]
        );
    }

    #[tokio::test]
    async fn default_relation_constraint_preserves_existing_null_behavior() {
        use crate::query_ast::tests::{memory_db, row};
        let db = memory_db(
            "query_test",
            [
                row("untyped", []),
                row(
                    "relation",
                    [(
                        "type",
                        Value::String(semantic_data::attr::RELATION_CLASS_ID.into()),
                    )],
                ),
                row(
                    "visible",
                    [("type", Value::String("example:article".into()))],
                ),
            ],
        )
        .await;
        let predicate = listing_predicate(DEFAULT_COLLECTION, None);
        let semantic_db_core::QueryResult::Select(rows) = db
            .query(semantic_data::query::QueryInput::from(collection_query(
                "query_test",
                25,
                0,
                None,
                predicate.as_ref(),
            )))
            .await
            .unwrap()
        else {
            panic!("AST rows")
        };
        assert_eq!(
            rows.iter()
                .map(|row| row.get("id").unwrap().as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["visible"]
        );
    }

    #[test]
    fn default_entities_query_excludes_relation_entities() {
        let predicate = listing_predicate(DEFAULT_COLLECTION, None);
        let query = collection_query(DEFAULT_COLLECTION, 50, 2, None, predicate.as_ref());
        assert_eq!(query.predicate, predicate);
        assert_eq!(query.limit, Some(50usize.into()));
        assert_eq!(query.offset, 100usize.into());
        assert_eq!(
            query.order_by,
            vec![
                order(None, ATTR_CREATED_AT, SortDirection::Desc),
                order(None, "id", SortDirection::Asc)
            ]
        );
    }

    #[test]
    fn default_non_entities_query_does_not_add_entity_type_filter() {
        let query = collection_query("events", 25, 1, None, None);
        assert_eq!(query.collection.as_deref(), Some("events"));
        assert!(query.predicate.is_none());
        assert_eq!(query.limit, Some(25usize.into()));
        assert_eq!(query.offset, 25usize.into());
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
    fn explicit_sql_can_include_unlisted_classes() {
        let query = "SELECT * FROM entities LIMIT 10";
        let reference = store_sql_reference(query);
        assert_eq!(
            resolve_applied_query(
                DEFAULT_COLLECTION,
                25,
                0,
                Some(&reference),
                None,
                &[],
                listing_predicate(DEFAULT_COLLECTION, None).as_ref()
            )
            .unwrap(),
            QueryRequest::Sql(query.into())
        );
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
            choices: Vec::new(),
        }];
        let listing_filter = listing_predicate(DEFAULT_COLLECTION, None).unwrap();
        let query = resolve_applied_query(
            DEFAULT_COLLECTION,
            25,
            2,
            None,
            Some(&filters),
            &fields,
            Some(&listing_filter),
        )
        .unwrap();
        let QueryRequest::Ast(query) = query else {
            panic!("generated AST")
        };
        assert_eq!(
            query.predicate,
            Some(crate::query_ast::binary(
                BinaryOp::And,
                listing_filter,
                crate::query_ast::binary(
                    BinaryOp::Gt,
                    crate::query_ast::field(None, "score"),
                    crate::query_ast::literal(Value::I64(7))
                )
            ))
        );
        assert_eq!(query.limit, Some(25usize.into()));
        assert_eq!(query.offset, 50usize.into());
        assert_eq!(
            query.order_by,
            vec![
                order(None, ATTR_CREATED_AT, SortDirection::Desc),
                order(None, "id", SortDirection::Asc)
            ]
        );
    }
}
