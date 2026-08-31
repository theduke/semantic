use std::{collections::BTreeSet, time::Duration};

use dioxus::prelude::*;
use dioxus_icons::lucide::Search;
use futures::future::join_all;
use regex::RegexBuilder;
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    value::{Object, Value},
};
use semantic_rpc::RpcClient;
use semantic_ui_core::{
    ENTITY_TITLE_FIELDS, EntityTarget, UiCatalog, entity_title, use_active_scope_id,
    use_rpc_client, use_ui_catalog,
};

use super::{IconButton, IconButtonSize};
use crate::app::entity_route;

const SEARCH_RESULT_LIMIT: usize = 12;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(180);
const RESULTS_ID: &str = "semantic-global-search-results";

#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchCollection {
    name: String,
    title_fields: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct SearchResult {
    target: EntityTarget,
    title: String,
    class_name: String,
}

#[derive(Clone, Debug, PartialEq)]
struct SearchResponse {
    pattern: String,
    result: std::result::Result<Vec<SearchResult>, String>,
}

#[component]
pub fn GlobalSearch() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let catalog = use_ui_catalog();
    let navigator = use_navigator();
    let mut open = use_signal(|| false);
    let mut query = use_signal(String::new);
    let mut active_index = use_signal(|| 0_usize);

    use_global_search_shortcut(EventHandler::new(move |_| open.set(true)));

    let collections = search_collections(&catalog);
    let search_catalog = catalog.clone();
    let response = use_resource(move || {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let catalog = search_catalog.clone();
        let collections = collections.clone();
        let pattern = query().trim().to_string();
        async move {
            if pattern.is_empty() {
                return SearchResponse {
                    pattern,
                    result: Ok(Vec::new()),
                };
            }
            dioxus_sdk_time::sleep(SEARCH_DEBOUNCE).await;
            let result = search_entities(client, scope_id, &catalog, &collections, &pattern).await;
            SearchResponse { pattern, result }
        }
    });

    let current_query = query().trim().to_string();
    let state = response.read().clone();
    let current_response = state
        .as_ref()
        .filter(|response| response.pattern == current_query);
    let results = current_response
        .and_then(|response| response.result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let error = current_response.and_then(|response| response.result.as_ref().err().cloned());
    let loading = !current_query.is_empty()
        && (current_response.is_none() || *response.state().read() == UseResourceState::Pending);
    let selected_index = active_index().min(results.len().saturating_sub(1));
    let active_descendant = (!results.is_empty()).then(|| result_dom_id(selected_index));

    let mut close_dialog = move || {
        open.set(false);
        query.set(String::new());
        active_index.set(0);
    };

    rsx! {
        IconButton {
            label: "Search entities".to_string(),
            tooltip: Some("Search entities (Ctrl+S)".to_string()),
            size: IconButtonSize::Small,
            on_click: move |_| open.set(true),
            Search { width: 18, height: 18 }
        }

        if open() {
            dxcomp::Dialog {
                class: "semantic-global-search",
                open: true,
                on_open_change: move |next_open: bool| {
                    if !next_open {
                        close_dialog();
                    }
                },
                div { class: "semantic-global-search__heading",
                    div {
                        dxcomp::DialogTitle { "Search entities" }
                        dxcomp::DialogDescription {
                            "Find an entity in any collection by ID or title."
                        }
                    }
                    kbd { class: "semantic-global-search__shortcut", "Ctrl S" }
                }

                div { class: "semantic-global-search__input-wrap",
                    Search { width: 20, height: 20 }
                    input {
                        class: "semantic-global-search__input",
                        r#type: "search",
                        value: query(),
                        placeholder: "Search by ID or title…",
                        autocomplete: "off",
                        autofocus: true,
                        role: "combobox",
                        aria_label: "Search entities by ID or title",
                        aria_autocomplete: "list",
                        aria_controls: RESULTS_ID,
                        aria_expanded: !results.is_empty(),
                        aria_activedescendant: active_descendant,
                        aria_describedby: "semantic-global-search-help semantic-global-search-status",
                        oninput: move |event: FormEvent| {
                            query.set(event.value());
                            active_index.set(0);
                        },
                        onkeydown: move |event: KeyboardEvent| match event.key() {
                            Key::ArrowDown if !results.is_empty() => {
                                event.prevent_default();
                                active_index.set((active_index() + 1).min(results.len() - 1));
                            }
                            Key::ArrowUp if !results.is_empty() => {
                                event.prevent_default();
                                active_index.set(active_index().saturating_sub(1));
                            }
                            Key::Enter if !results.is_empty() => {
                                event.prevent_default();
                                let route = entity_route(&results[selected_index].target);
                                close_dialog();
                                navigator.push(route);
                            }
                            _ => {}
                        },
                    }
                    if loading {
                        span {
                            class: "semantic-global-search__spinner",
                            aria_label: "Searching",
                        }
                    }
                }

                p {
                    id: "semantic-global-search-help",
                    class: "semantic-global-search__help",
                    "Case-insensitive regular expressions are supported."
                }
                p {
                    id: "semantic-global-search-status",
                    class: "semantic-visually-hidden",
                    role: "status",
                    aria_live: "polite",
                    if loading {
                        "Searching"
                    } else if error.is_some() {
                        "Search failed"
                    } else if current_query.is_empty() {
                        "Enter a search pattern"
                    } else {
                        "{results.len()} results"
                    }
                }

                div { class: "semantic-global-search__body",
                    if let Some(error) = error {
                        div { class: "semantic-global-search__message", role: "alert",
                            strong { "Search unavailable" }
                            span { "{error}" }
                        }
                    } else if current_query.is_empty() {
                        div { class: "semantic-global-search__message",
                            strong { "Search across your workspace" }
                            span { "Try a title, an entity ID, or a regular expression such as " code { "^note-" } "." }
                        }
                    } else if !loading && results.is_empty() {
                        div { class: "semantic-global-search__message",
                            strong { "No matching entities" }
                            span { "Try a broader pattern or check the expression syntax." }
                        }
                    } else {
                        ul {
                            id: RESULTS_ID,
                            class: "semantic-global-search__results",
                            role: "listbox",
                            aria_label: "Entity search results",
                            for (index, result) in results.iter().enumerate() {
                                li {
                                    id: result_dom_id(index),
                                    key: "{result.target.collection_or_default()}:{result.target.id}",
                                    role: "option",
                                    aria_selected: index == selected_index,
                                    button {
                                        r#type: "button",
                                        class: "semantic-global-search__result",
                                        onmouseenter: move |_| active_index.set(index),
                                        onclick: {
                                            let target = result.target.clone();
                                            move |_| {
                                                let route = entity_route(&target);
                                                close_dialog();
                                                navigator.push(route);
                                            }
                                        },
                                        span { class: "semantic-global-search__result-main",
                                            strong { title: result.title.clone(), "{result.title}" }
                                            span { class: "semantic-global-search__class", "{result.class_name}" }
                                        }
                                        span { class: "semantic-global-search__result-meta",
                                            code { title: result.target.id.clone(), "{result.target.id}" }
                                            if !result.target.is_default_collection() {
                                                span { aria_hidden: "true", "·" }
                                                span { "{result.target.collection_or_default()}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                div { class: "semantic-global-search__footer", aria_hidden: "true",
                    span { kbd { "↑" } kbd { "↓" } " navigate" }
                    span { kbd { "Enter" } " open" }
                    span { kbd { "Esc" } " close" }
                }
            }
        }
    }
}

fn use_global_search_shortcut(on_trigger: EventHandler<()>) {
    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                r#"
                if (window.__semanticGlobalSearchKeyHandler) {
                    window.removeEventListener('keydown', window.__semanticGlobalSearchKeyHandler);
                }
                window.__semanticGlobalSearchKeyHandler = (event) => {
                    if (event.repeat || !(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== 's') return;
                    event.preventDefault();
                    dioxus.send('open');
                };
                window.addEventListener('keydown', window.__semanticGlobalSearchKeyHandler);
            "#,
            );
            while eval.recv::<String>().await.is_ok() {
                on_trigger.call(());
            }
        });
    });
    use_drop(|| {
        _ = document::eval(
            r#"
            window.removeEventListener('keydown', window.__semanticGlobalSearchKeyHandler);
            delete window.__semanticGlobalSearchKeyHandler;
        "#,
        );
    });
}

fn search_collections(catalog: &UiCatalog) -> Vec<SearchCollection> {
    let mut collections = catalog
        .collections()
        .map(|collection| {
            let declared_fields = collection
                .field_ids
                .iter()
                .map(|field| field.canonical_field.as_str())
                .collect::<BTreeSet<_>>();
            let title_fields = ENTITY_TITLE_FIELDS
                .iter()
                .filter(|field| declared_fields.contains(**field))
                .map(|field| (*field).to_string())
                .collect();
            SearchCollection {
                name: collection.name.clone(),
                title_fields,
            }
        })
        .collect::<Vec<_>>();
    if !collections
        .iter()
        .any(|collection| collection.name == DEFAULT_COLLECTION)
    {
        collections.push(SearchCollection {
            name: DEFAULT_COLLECTION.to_string(),
            title_fields: Vec::new(),
        });
    }
    collections.sort_by(|left, right| left.name.cmp(&right.name));
    collections
}

async fn search_entities(
    client: RpcClient,
    scope_id: Option<String>,
    catalog: &UiCatalog,
    collections: &[SearchCollection],
    pattern: &str,
) -> std::result::Result<Vec<SearchResult>, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|error| format!("Invalid regular expression: {error}"))?;

    let searches = collections.iter().cloned().map(|collection| {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let query = search_query(&collection, pattern);
        async move {
            run_search_query(client, scope_id, query)
                .await
                .map(|rows| (collection.name, rows))
        }
    });
    let responses = join_all(searches).await;
    let mut results = Vec::new();
    for response in responses {
        let (collection, rows) = response?;
        for object in rows {
            let Some(id) = object.get("id").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let class_name = catalog
                .object_class(&object)
                .map(|class| class.name.clone())
                .or_else(|| {
                    object
                        .get("type")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "Unclassified".to_string());
            let title = entity_title(&object, Some(&id), Some(&class_name));
            results.push(SearchResult {
                target: EntityTarget::new(
                    (collection != DEFAULT_COLLECTION).then_some(collection.clone()),
                    id,
                ),
                title,
                class_name,
            });
        }
    }

    results.sort_by(|left, right| {
        search_rank(left, pattern)
            .cmp(&search_rank(right, pattern))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            .then_with(|| left.target.id.cmp(&right.target.id))
    });
    results.dedup_by(|left, right| left.target == right.target);
    results.truncate(SEARCH_RESULT_LIMIT);
    Ok(results)
}

fn search_query(collection: &SearchCollection, pattern: &str) -> String {
    let mut fields = vec!["id"];
    fields.extend(collection.title_fields.iter().map(String::as_str));
    let pattern = escape_sql_string(pattern);
    let predicate = fields
        .into_iter()
        .map(|field| format!("{} ~* '{pattern}'", quote_sql_ident(field)))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!(
        "SELECT * FROM {} WHERE ({predicate}) LIMIT {SEARCH_RESULT_LIMIT}",
        quote_sql_ident(&collection.name),
    )
}

async fn run_search_query(
    client: RpcClient,
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
        .map_err(|error| error.to_string())?;
    let Value::Object(response) = response else {
        return Err("Search response must be an object".to_string());
    };
    let Some(Value::List(rows)) = response.get("rows") else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .filter_map(|row| match row {
            Value::Object(object) => Some(object.clone()),
            _ => None,
        })
        .collect())
}

fn search_rank(result: &SearchResult, pattern: &str) -> (u8, usize) {
    if result.target.id.eq_ignore_ascii_case(pattern) {
        return (0, 0);
    }
    if result.title.eq_ignore_ascii_case(pattern) {
        return (1, 0);
    }
    let Ok(regex) = RegexBuilder::new(pattern).case_insensitive(true).build() else {
        return (4, usize::MAX);
    };
    if let Some(found) = regex.find(&result.target.id) {
        return (2, found.start());
    }
    if let Some(found) = regex.find(&result.title) {
        return (3, found.start());
    }
    (4, usize::MAX)
}

fn quote_sql_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

fn result_dom_id(index: usize) -> String {
    format!("semantic-global-search-result-{index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_uses_case_insensitive_regex_for_id_and_available_title_fields() {
        let collection = SearchCollection {
            name: "people".to_string(),
            title_fields: vec!["semantic:title".to_string(), "display_name".to_string()],
        };

        assert_eq!(
            search_query(&collection, "^Ada('s)?$"),
            "SELECT * FROM \"people\" WHERE (\"id\" ~* '^Ada(''s)?$' OR \"semantic:title\" ~* '^Ada(''s)?$' OR \"display_name\" ~* '^Ada(''s)?$') LIMIT 12"
        );
    }

    #[test]
    fn exact_id_matches_rank_before_title_matches() {
        let id_match = SearchResult {
            target: EntityTarget::default_collection("ada"),
            title: "Someone else".to_string(),
            class_name: "Person".to_string(),
        };
        let title_match = SearchResult {
            target: EntityTarget::default_collection("person-1"),
            title: "Ada".to_string(),
            class_name: "Person".to_string(),
        };

        assert!(search_rank(&id_match, "ada") < search_rank(&title_match, "ada"));
    }
}
