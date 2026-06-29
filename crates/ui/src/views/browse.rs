use std::cell::RefCell;
use std::collections::BTreeMap;

use dioxus::prelude::*;
use semantic_data::builtin::DEFAULT_COLLECTION;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{
    EntityDisplayMode, EntityDisplayRenderer, EntityList, use_active_scope_id, use_rpc_client,
};

use crate::views::Route;

const DEFAULT_VIEW: &str = "cards";
const DEFAULT_PAGE_SIZE: usize = 50;
const RELATION_CLASS_ID: &str = "semantic:relation";

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
    let collection_name = collection
        .clone()
        .unwrap_or_else(|| DEFAULT_COLLECTION.to_string());
    let display_mode = parse_display_mode(view.as_deref());
    let renderer_mode = parse_renderer(renderer.as_deref());
    let page = page.unwrap_or(0);
    let page_size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    let stored_sql = sql.as_deref().and_then(load_sql);
    let initial_sql = stored_sql
        .clone()
        .unwrap_or_else(|| default_query(&collection_name, page_size, page));
    let mut sql_input = use_signal(|| initial_sql);
    let query = match sql.as_deref() {
        Some(hash) => stored_sql
            .clone()
            .ok_or_else(|| format!("No SQL text found in local storage for hash {hash}")),
        None => Ok(default_query(&collection_name, page_size, page)),
    };
    let resource = use_resource({
        let client = client.clone();
        let scope_id = scope_id.clone();
        let query = query.clone();
        move || {
            let client = client.clone();
            let scope_id = scope_id.clone();
            let query = query.clone();
            async move {
                match query {
                    Ok(query) => run_query(client, scope_id, query).await,
                    Err(err) => Err(err),
                }
            }
        }
    });
    let custom_sql = sql.is_some();
    let mut show_filters = use_signal(|| false);
    let mut grid_columns = use_signal(|| 1_usize);
    let filters_open = *show_filters.read();
    let grid_columns_value = *grid_columns.read();
    let next_disabled = match resource.read_unchecked().as_ref() {
        Some(Ok(rows)) => custom_sql || rows.len() < page_size,
        _ => true,
    };

    rsx! {
        section { class: "semantic-browse",
            header { class: "semantic-browse__header",
                div {
                    h2 { "Browse entities" }
                    p { "Inspect records and tune the SQL behind the result set." }
                }
                div { class: "semantic-browse__header-meta",
                    span { "Collection" }
                    code { "{collection_name}" }
                }
            }
            div { class: "semantic-browse__results",
                div { class: "semantic-browse__toolbar",
                    div { class: "semantic-browse__toggle-group", "aria-label": "Filters",
                        dxcomp::Button {
                            size: dxcomp::ButtonSize::Sm,
                            variant: if filters_open { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            onclick: move |_| {
                                let next = !*show_filters.read();
                                show_filters.set(next);
                            },
                            if filters_open {
                                "Hide filters"
                            } else if custom_sql {
                                "Edit filter"
                            } else {
                                "Filter"
                            }
                        }
                    }
                    div { class: "semantic-browse__toggle-group", "aria-label": "Result view",
                        dxcomp::Button {
                            size: dxcomp::ButtonSize::Sm,
                            variant: if display_mode == EntityDisplayMode::Card { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            onclick: push_browse(collection.clone(), EntityDisplayMode::Card, renderer_mode, page, page_size, sql.clone()),
                            "Cards"
                        }
                        dxcomp::Button {
                            size: dxcomp::ButtonSize::Sm,
                            variant: if display_mode == EntityDisplayMode::Table { dxcomp::ButtonVariant::Primary } else { dxcomp::ButtonVariant::Outline },
                            onclick: push_browse(collection.clone(), EntityDisplayMode::Table, renderer_mode, page, page_size, sql.clone()),
                            "Table"
                        }
                    }
                    label { class: "semantic-browse__page-size",
                        span { "Grid" }
                        select {
                            disabled: display_mode != EntityDisplayMode::Card,
                            value: "{grid_columns_value}",
                            onchange: move |event: FormEvent| {
                                let columns = event.value().parse::<usize>().unwrap_or(1).clamp(1, 3);
                                grid_columns.set(columns);
                            },
                            option { value: "1", "1 column" }
                            option { value: "2", "2 columns" }
                            option { value: "3", "3 columns" }
                        }
                    }
                    label { class: "semantic-browse__page-size",
                        span { "Rows" }
                        select {
                            value: "{page_size}",
                            onchange: {
                                let collection = collection.clone();
                                let view = view_string(display_mode);
                                let renderer = renderer_string(renderer_mode);
                                let sql = sql.clone();
                                move |event: FormEvent| {
                                    let next_page_size = event.value().parse::<usize>().unwrap_or(DEFAULT_PAGE_SIZE);
                                    navigator().push(browse_route(
                                        collection.clone(),
                                        Some(view.clone()),
                                        Some(renderer.clone()),
                                        Some(0),
                                        Some(next_page_size),
                                        sql.clone(),
                                    ));
                                }
                            },
                            option { value: "25", "25" }
                            option { value: "50", "50" }
                            option { value: "100", "100" }
                        }
                    }
                    div { class: "semantic-browse__pagination",
                        dxcomp::Button {
                            disabled: custom_sql || page == 0,
                            size: dxcomp::ButtonSize::Sm,
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: push_browse(collection.clone(), display_mode, renderer_mode, page.saturating_sub(1), page_size, sql.clone()),
                            "Previous"
                        }
                        span { "Page {page + 1}" }
                        dxcomp::Button {
                            disabled: next_disabled,
                            size: dxcomp::ButtonSize::Sm,
                            variant: dxcomp::ButtonVariant::Outline,
                            onclick: push_browse(collection.clone(), display_mode, renderer_mode, page + 1, page_size, sql.clone()),
                            "Next"
                        }
                    }
                }
                if filters_open {
                    div { class: "semantic-browse__query-card",
                        dxcomp::Card {
                            dxcomp::CardHeader {
                                dxcomp::CardTitle { "Query" }
                                dxcomp::CardDescription {
                                    if custom_sql {
                                        "Custom SQL is active."
                                    } else {
                                        "Default paged collection query."
                                    }
                                }
                            }
                            dxcomp::CardContent {
                                div { class: "semantic-browse__query-editor",
                                    dxcomp::Textarea {
                                        value: "{sql_input}",
                                        oninput: move |event: FormEvent| sql_input.set(event.value())
                                    }
                                }
                            }
                            dxcomp::CardFooter {
                                div { class: "semantic-browse__actions",
                                    dxcomp::Button {
                                        onclick: {
                                            let collection = collection.clone();
                                            let view = view_string(display_mode);
                                            let renderer = renderer_string(renderer_mode);
                                            move |_| {
                                                let raw_sql = sql_input.read().trim().to_string();
                                                if raw_sql.is_empty() {
                                                    navigator().push(browse_route(
                                                        collection.clone(),
                                                        Some(view.clone()),
                                                        Some(renderer.clone()),
                                                        Some(page),
                                                        Some(page_size),
                                                        None,
                                                    ));
                                                    return;
                                                }
                                                let hash = stable_hash_hex(&raw_sql);
                                                store_sql(&hash, &raw_sql);
                                                navigator().push(browse_route(
                                                    collection.clone(),
                                                    Some(view.clone()),
                                                    Some(renderer.clone()),
                                                    Some(page),
                                                    Some(page_size),
                                                    Some(hash),
                                                ));
                                            }
                                        },
                                        "Apply"
                                    }
                                    dxcomp::Button {
                                        variant: dxcomp::ButtonVariant::Secondary,
                                        onclick: {
                                            let collection = collection.clone();
                                            let view = view_string(display_mode);
                                            let renderer = renderer_string(renderer_mode);
                                            move |_| {
                                                let raw_sql = sql_input.read().trim().to_string();
                                                show_filters.set(false);
                                                if raw_sql.is_empty() {
                                                    navigator().push(browse_route(
                                                        collection.clone(),
                                                        Some(view.clone()),
                                                        Some(renderer.clone()),
                                                        Some(page),
                                                        Some(page_size),
                                                        None,
                                                    ));
                                                    return;
                                                }
                                                let hash = stable_hash_hex(&raw_sql);
                                                store_sql(&hash, &raw_sql);
                                                navigator().push(browse_route(
                                                    collection.clone(),
                                                    Some(view.clone()),
                                                    Some(renderer.clone()),
                                                    Some(page),
                                                    Some(page_size),
                                                    Some(hash),
                                                ));
                                            }
                                        },
                                        "Apply and hide"
                                    }
                                    dxcomp::Button {
                                        variant: dxcomp::ButtonVariant::Outline,
                                        onclick: {
                                            let collection = collection.clone();
                                            let view = view_string(display_mode);
                                            let renderer = renderer_string(renderer_mode);
                                            move |_| {
                                                navigator().push(browse_route(
                                                    collection.clone(),
                                                    Some(view.clone()),
                                                    Some(renderer.clone()),
                                                    Some(0),
                                                    Some(page_size),
                                                    None,
                                                ));
                                            }
                                        },
                                        "Clear"
                                    }
                                }
                            }
                        }
                    }
                }
                match &*resource.read_unchecked() {
                    Some(Ok(rows)) => {
                        let row_count = rows.len();
                        rsx! {
                            div { class: "semantic-browse__result-section semantic-browse__result-section--grid-{grid_columns_value}",
                                div { class: "semantic-browse__result-bar",
                                    span { "{row_count} rows returned" }
                                }
                                EntityList {
                                    objects: rows.clone(),
                                    display_mode,
                                    renderer: renderer_mode,
                                    collection: collection.clone()
                                }
                                div { class: "semantic-browse__pagination semantic-browse__pagination--bottom",
                                    dxcomp::Button {
                                        disabled: custom_sql || page == 0,
                                        size: dxcomp::ButtonSize::Sm,
                                        variant: dxcomp::ButtonVariant::Outline,
                                        onclick: push_browse(
                                            collection.clone(),
                                            display_mode,
                                            renderer_mode,
                                            page.saturating_sub(1),
                                            page_size,
                                            sql.clone(),
                                        ),
                                        "Previous"
                                    }
                                    span { "Page {page + 1}" }
                                    dxcomp::Button {
                                        disabled: next_disabled,
                                        size: dxcomp::ButtonSize::Sm,
                                        variant: dxcomp::ButtonVariant::Outline,
                                        onclick: push_browse(
                                            collection.clone(),
                                            display_mode,
                                            renderer_mode,
                                            page + 1,
                                            page_size,
                                            sql.clone(),
                                        ),
                                        "Next"
                                    }
                                }
                            }
                        }
                    },
                    Some(Err(err)) => rsx! {
                        div { class: "semantic-browse__state semantic-error", "{err}" }
                    },
                    None => rsx! {
                        div { class: "semantic-browse__state semantic-loading", "Loading entities..." }
                    },
                }
            }
        }
    }
}

fn push_browse(
    collection: Option<String>,
    display_mode: EntityDisplayMode,
    renderer: EntityDisplayRenderer,
    page: usize,
    page_size: usize,
    sql: Option<String>,
) -> EventHandler<MouseEvent> {
    EventHandler::new(move |_| {
        navigator().push(browse_route(
            collection.clone(),
            Some(view_string(display_mode)),
            Some(renderer_string(renderer)),
            Some(page),
            Some(page_size),
            sql.clone(),
        ));
    })
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
        page_size,
        page.saturating_mul(page_size)
    )
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

fn stable_hash_hex(sql: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in sql.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
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
}
