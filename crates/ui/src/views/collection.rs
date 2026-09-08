use std::rc::Rc;

use dioxus::prelude::*;
use semantic_data::value::{Object, Value};
use semantic_ui_core::{
    EntityDisplayMode, EntityDisplayRenderer,
    components::{EmptyState, InlineNotice, LoadingSkeleton, NoticeVariant, RefreshingIndicator},
    use_active_scope_id, use_rpc_client, use_ui_catalog_context,
};

use crate::{
    components::{
        DataToolbar, EntityExplorer, EntityResults, PageHeader, Pagination, ResultDensity,
    },
    views::Route,
};

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CollectionQueryKey {
    scope_id: Option<String>,
    collection: String,
    page: usize,
    page_size: usize,
}

#[derive(Clone)]
struct CollectionQueryPage {
    rows: Rc<[Object]>,
    has_more: bool,
}

#[derive(Clone)]
struct CollectionQueryResponse {
    key: CollectionQueryKey,
    result: std::result::Result<CollectionQueryPage, String>,
}

#[component]
pub fn CollectionPage(collection: String) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog_signal = use_ui_catalog_context().catalog_signal();
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

    // Route currently owns only collection identity. These view preferences stay
    // local until Collection gains query parameters without changing Route shape.
    let mut page = use_signal(|| 0_usize);
    let mut page_collection = use_signal(|| collection.clone());
    let mut page_size = use_signal(|| DEFAULT_PAGE_SIZE);
    let mut display_mode = use_signal(|| EntityDisplayMode::Card);
    let mut renderer = use_signal(|| EntityDisplayRenderer::Custom);
    let mut grid_columns = use_signal(|| 1_usize);
    let mut density = use_signal(ResultDensity::default);

    let page_scope = page_collection.read().clone();
    let current_page = if page_scope == collection {
        clamp_page(*page.read())
    } else {
        0
    };
    let current_page_size = clamp_page_size(*page_size.read());
    let current_display_mode = *display_mode.read();
    let current_renderer = *renderer.read();

    use_effect(use_reactive(&collection, move |next_collection| {
        if page_collection.peek().as_str() != next_collection.as_str() {
            page.set(0);
            page_collection.set(next_collection);
        }
    }));

    let query_key = use_memo(use_reactive(
        &CollectionQueryKey {
            scope_id: scope_id.clone(),
            collection: collection.clone(),
            page: current_page,
            page_size: current_page_size,
        },
        |key| key,
    ));
    let mut resource = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            let key = query_key();
            async move {
                let result = query_collection_page(client, &key).await;
                CollectionQueryResponse { key, result }
            }
        }
    });

    // A response is only rendered when its complete reactive key is current.
    // Dioxus cancels the prior Resource future when this key changes; retaining
    // the key on the response is the additional stale-completion guard.
    let current_key = query_key();
    let loading = *resource.state().read() == UseResourceState::Pending;
    let response = resource.read().clone();
    let current_response = response
        .as_ref()
        .filter(|response| response.key == current_key)
        .cloned();
    let visible_page = current_response
        .as_ref()
        .and_then(|response| response.result.as_ref().ok().cloned());
    let error = (!loading)
        .then(|| current_response.as_ref())
        .flatten()
        .and_then(|response| response.result.as_ref().err().cloned());
    let successful_empty = !loading
        && current_response
            .as_ref()
            .and_then(|response| response.result.as_ref().ok())
            .is_some_and(|page| page.rows.is_empty());
    let result_summary = visible_page.as_ref().map(|result| {
        let noun = if result.rows.len() == 1 {
            "record"
        } else {
            "records"
        };
        if result.has_more {
            format!("{} {noun} shown; more are available.", result.rows.len())
        } else {
            format!("{} {noun} shown.", result.rows.len())
        }
    });

    rsx! {
        div { class: "semantic-page semantic-collection",
            PageHeader {
                title: collection.clone(),
                description: "Browse this collection's records with bounded, explicit pagination.",
                breadcrumbs: rsx! {
                    Link { to: Route::CatalogPage, "Catalog" }
                    span { aria_hidden: "true", "/" }
                    span { "{collection}" }
                },
                actions: rsx! {
                    Link {
                        class: "semantic-button-link",
                        to: Route::CreateEntityPage,
                        "Create entity (choose collection)"
                    }
                    Link {
                        class: "semantic-button-link semantic-button-link--secondary",
                        to: Route::UploadPage,
                        "Upload files"
                    }
                    Link {
                        class: "semantic-button-link semantic-button-link--secondary",
                        to: advanced_browse_route(collection.clone(), current_page_size),
                        "Advanced browse"
                    }
                }
            }

            EntityExplorer {
                refreshing: loading && visible_page.is_some(),
                toolbar: rsx! {
                    DataToolbar {
                        collection: collection.clone(),
                        collections: collections(),
                        display_mode: current_display_mode,
                        renderer: current_renderer,
                        grid_columns: *grid_columns.read(),
                        density: *density.read(),
                        advanced_open: false,
                        custom_query: false,
                        show_advanced: false,
                        toolbar_label: "Collection records controls",
                        on_collection_change: move |next_collection: String| {
                            navigator().push(Route::CollectionPage {
                                collection: next_collection,
                            });
                        },
                        on_display_mode_change: move |mode: EntityDisplayMode| display_mode.set(mode),
                        on_renderer_change: move |next_renderer: EntityDisplayRenderer| renderer.set(next_renderer),
                        on_grid_columns_change: move |columns: usize| grid_columns.set(columns.clamp(1, 3)),
                        on_density_change: move |next_density: ResultDensity| density.set(next_density),
                        on_advanced_open_change: move |_: bool| {},
                    }
                },

                if loading {
                    if visible_page.is_some() {
                        RefreshingIndicator { label: "Refreshing collection records" }
                    } else {
                        LoadingSkeleton { label: "Loading collection records", line_count: 5 }
                    }
                }

                if let Some(error) = error.clone() {
                    InlineNotice {
                        title: "Could not load this collection",
                        message: error,
                        variant: NoticeVariant::Error,
                        action_label: "Retry",
                        on_action: move |_| resource.restart(),
                    }
                }

                if successful_empty {
                    if current_page == 0 {
                        EmptyState {
                            title: "This collection is empty",
                            description: "Create an entity or upload files to add the first record.",
                            action_label: "Create entity (choose collection)",
                            on_action: move |_| {
                                navigator().push(Route::CreateEntityPage);
                            }
                        }
                    } else {
                        EmptyState {
                            title: "No records on this page",
                            description: "The collection changed or the previous page is now the last available page.",
                            action_label: "Go to previous page",
                            on_action: move |_| page.set(current_page.saturating_sub(1)),
                        }
                    }
                } else if let Some(result) = visible_page {
                    div { class: "semantic-collection__result-summary",
                        span { "{result_summary.as_deref().unwrap_or_default()}" }
                        dxcomp::Button {
                            size: dxcomp::ButtonSize::Sm,
                            variant: dxcomp::ButtonVariant::Ghost,
                            disabled: loading,
                            onclick: move |_| resource.restart(),
                            "Refresh"
                        }
                    }
                    EntityResults {
                        rows: result.rows.to_vec(),
                        display_mode: current_display_mode,
                        renderer: current_renderer,
                        collection: Some(collection.clone()),
                        grid_columns: *grid_columns.read(),
                        density: *density.read(),
                    }
                    Pagination {
                        page: current_page,
                        page_size: current_page_size,
                        has_more: result.has_more,
                        disabled: loading,
                        on_page_change: move |next_page| page.set(clamp_page(next_page)),
                        on_page_size_change: move |next_page_size| {
                            page.set(0);
                            page_size.set(clamp_page_size(next_page_size));
                        },
                    }
                } else if !loading && error.is_none() {
                    InlineNotice {
                        title: "Collection results changed",
                        message: "The current request was replaced before it completed. Retry to load this collection.",
                        variant: NoticeVariant::Warning,
                        action_label: "Retry",
                        on_action: move |_| resource.restart(),
                    }
                }
            }
        }
    }
}

fn advanced_browse_route(collection: String, page_size: usize) -> Route {
    Route::BrowsePage {
        collection: Some(collection),
        view: Some("cards".to_string()),
        renderer: Some("custom".to_string()),
        page: Some(0),
        page_size: Some(clamp_page_size(page_size)),
        filters: None,
        sql: None,
    }
}

async fn query_collection_page(
    client: semantic_rpc::RpcClient,
    key: &CollectionQueryKey,
) -> std::result::Result<CollectionQueryPage, String> {
    let mut payload = Object::new();
    if let Some(scope_id) = key.scope_id.clone() {
        payload.insert("scope_id", Value::String(scope_id));
    }
    payload.insert(
        "query",
        Value::String(collection_page_query(
            &key.collection,
            key.page,
            key.page_size,
        )),
    );
    payload.insert("format", Value::String("sql".to_string()));
    let response = client
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await
        .map_err(|err| err.to_string())?;
    let Value::Object(object) = response else {
        return Err("query response must be an object".to_string());
    };
    let Some(Value::List(values)) = object.get("rows") else {
        return Ok(CollectionQueryPage {
            rows: Rc::from([]),
            has_more: false,
        });
    };
    let mut rows = values
        .iter()
        .filter_map(|row| match row {
            Value::Object(row) => Some(row.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let page_size = clamp_page_size(key.page_size);
    let has_more = rows.len() > page_size;
    rows.truncate(page_size);
    Ok(CollectionQueryPage {
        rows: rows.into(),
        has_more,
    })
}

fn collection_page_query(collection: &str, page: usize, page_size: usize) -> String {
    let page = clamp_page(page);
    let page_size = clamp_page_size(page_size);
    format!(
        "SELECT * FROM {} LIMIT {} OFFSET {}",
        sql_identifier(collection),
        page_size.saturating_add(1),
        page.saturating_mul(page_size)
    )
}

fn sql_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_page_query_quotes_identifiers_and_fetches_a_sentinel_row() {
        assert_eq!(
            collection_page_query("media:with\"quote", 2, 50),
            "SELECT * FROM \"media:with\"\"quote\" LIMIT 51 OFFSET 100"
        );
    }

    #[test]
    fn collection_page_query_clamps_unbounded_route_state() {
        assert_eq!(
            collection_page_query("entities", usize::MAX, usize::MAX),
            "SELECT * FROM \"entities\" LIMIT 201 OFFSET 200000000"
        );
    }
}
