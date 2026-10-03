use dioxus::prelude::*;
use semantic_data::{
    attr::ATTR_PARENT,
    value::{Object, Value},
};

use crate::{
    EntityComments,
    query_ast::{all, any, binary, field, order, query_payload, select, wildcard},
    ui_catalog::{EntityTarget, use_ui_catalog},
    use_active_scope_id, use_rpc_client,
};

use super::card::EntityLink;

const PAGE_SIZE: usize = 10;
const RELATION_EDGES_COLLECTION: &str = "__semantic.relationship_edges";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssociationKind {
    Children,
    Relations,
}

impl AssociationKind {
    fn label(self) -> &'static str {
        match self {
            Self::Children => "Children",
            Self::Relations => "Relations",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AssociationQueryKey {
    scope_id: Option<String>,
    target: EntityTarget,
    kind: AssociationKind,
    page: usize,
}

#[derive(Clone)]
struct AssociationResponse {
    key: AssociationQueryKey,
    result: std::result::Result<Vec<Object>, String>,
}

#[component]
pub(super) fn EntityAssociations(target: EntityTarget) -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut selected = use_signal(|| None::<AssociationKind>);
    let mut show_comments = use_signal(|| false);
    let mut page = use_signal(|| 0usize);
    let mut resource = use_resource({
        let target = target.clone();
        let client = client.clone();
        let scope_id = scope_id.clone();
        move || {
            let kind = selected();
            let page_number = page();
            let target = target.clone();
            let client = client.clone();
            let scope_id = scope_id.clone();
            async move {
                let kind = kind?;
                let key = AssociationQueryKey {
                    scope_id,
                    target,
                    kind,
                    page: page_number,
                };
                let result = load_page(client, &key).await;
                Some(AssociationResponse { key, result })
            }
        }
    });

    let current_key = selected().map(|kind| AssociationQueryKey {
        scope_id,
        target: target.clone(),
        kind,
        page: page(),
    });
    let response = resource.read().clone().flatten();
    let result = response
        .as_ref()
        .filter(|response| current_key.as_ref() == Some(&response.key))
        .map(|response| &response.result);
    let loading = selected().is_some()
        && (*resource.state().read() == UseResourceState::Pending || result.is_none());
    let error = result.and_then(|result| result.as_ref().err());
    let rows = result.and_then(|result| result.as_ref().ok());
    let has_more = rows.is_some_and(|rows| rows.len() > PAGE_SIZE);

    rsx! {
        section { class: "semantic-entity-associations", aria_label: "Related data",
            div { class: "semantic-entity-associations__tabs", role: "group", aria_label: "Explore related data",
                for kind in [AssociationKind::Children, AssociationKind::Relations] {
                    button {
                        type: "button",
                        class: "semantic-entity-associations__tab",
                        aria_pressed: selected() == Some(kind),
                        onclick: move |_| {
                            show_comments.set(false);
                            page.set(0);
                            selected.set((selected() != Some(kind)).then_some(kind));
                        },
                        "{kind.label()}"
                    }
                }
                button {
                    type: "button",
                    class: "semantic-entity-associations__tab",
                    aria_pressed: show_comments(),
                    onclick: move |_| {
                        selected.set(None);
                        show_comments.toggle();
                    },
                    "Comments"
                }
            }
            if show_comments() {
                div { class: "semantic-entity-associations__panel",
                    EntityComments { target: target.clone() }
                }
            }
            if let Some(kind) = selected() {
                div {
                    class: "semantic-entity-associations__panel",
                    role: "region",
                    aria_label: "{kind.label()}",
                    aria_busy: loading,
                    if loading {
                        p { class: "semantic-entity-associations__message", role: "status", "Loading {kind.label().to_lowercase()}…" }
                    } else if let Some(error) = error {
                        div { class: "semantic-entity-associations__error", role: "alert",
                            p { "Could not load {kind.label().to_lowercase()}. {error}" }
                            dxcomp::Button {
                                variant: dxcomp::ButtonVariant::Outline,
                                size: dxcomp::ButtonSize::Sm,
                                onclick: move |_| resource.restart(),
                                "Try again"
                            }
                        }
                    } else if let Some(rows) = rows {
                        if rows.is_empty() {
                            p { class: "semantic-entity-associations__message",
                                if page() > 0 {
                                    "No more results on this page."
                                } else if kind == AssociationKind::Children {
                                    "No children yet."
                                } else {
                                    "No relations yet."
                                }
                            }
                        } else {
                            ul { class: "semantic-entity-associations__list",
                                for row in rows.iter().take(PAGE_SIZE) {
                                    li { class: "semantic-entity-associations__item",
                                        if kind == AssociationKind::Children {
                                            ChildRow { object: row.clone(), collection: target.collection.clone() }
                                        } else {
                                            RelationRow { object: row.clone(), target: target.clone() }
                                        }
                                    }
                                }
                            }
                        }
                        if page() > 0 || has_more {
                            nav { class: "semantic-entity-associations__pagination", aria_label: "{kind.label()} pages",
                                dxcomp::Button {
                                    variant: dxcomp::ButtonVariant::Outline,
                                    size: dxcomp::ButtonSize::Sm,
                                    disabled: page() == 0,
                                    onclick: move |_| page.set(page().saturating_sub(1)),
                                    "Previous"
                                }
                                span { class: "semantic-entity-associations__page", aria_live: "polite", "Page {page() + 1}" }
                                dxcomp::Button {
                                    variant: dxcomp::ButtonVariant::Outline,
                                    size: dxcomp::ButtonSize::Sm,
                                    disabled: !has_more,
                                    onclick: move |_| page.set(page().saturating_add(1)),
                                    "Next"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ChildRow(object: Object, collection: Option<String>) -> Element {
    let catalog = use_ui_catalog();
    let id = object.get("id").and_then(Value::as_str).unwrap_or_default();
    let class_name = catalog
        .object_class(&object)
        .map(|class| class.meta.title.as_deref().unwrap_or(&class.name))
        .or_else(|| object.get("type").and_then(Value::as_str));
    let title = catalog.entity_title(&object);
    let show_id = title != id;
    rsx! {
        div { class: "semantic-entity-associations__item-main",
            EntityLink { target: EntityTarget::new(collection, id), text: title }
            if let Some(class_name) = class_name {
                span { class: "semantic-entity-associations__item-type", "{class_name}" }
            }
        }
        if show_id {
            code { class: "semantic-entity-associations__item-id", "{id}" }
        }
    }
}

#[component]
fn RelationRow(object: Object, target: EntityTarget) -> Element {
    let catalog = use_ui_catalog();
    let relation_id = object
        .get("relation")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let destination = object
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let relation_name = catalog
        .snapshot()
        .relationships
        .iter()
        .find(|stored| stored.relationship.id == relation_id)
        .map(|stored| {
            stored
                .relationship
                .meta
                .title
                .as_deref()
                .unwrap_or(&stored.relationship.name)
        })
        .unwrap_or(relation_id);
    let direction = if source == target.id && destination == target.id {
        "Self relation"
    } else if source == target.id {
        "Outgoing"
    } else {
        "Incoming"
    };
    rsx! {
        div { class: "semantic-entity-associations__item-main",
            span { class: "semantic-entity-associations__relation-name", "{relation_name}" }
            span { class: "semantic-entity-associations__relation-direction", "{direction}" }
        }
        div { class: "semantic-entity-associations__relation-endpoints",
            EntityLink { target: EntityTarget::new(target.collection.clone(), source), text: source.to_string() }
            span { aria_hidden: "true", "→" }
            EntityLink { target: EntityTarget::new(target.collection.clone(), destination), text: destination.to_string() }
        }
    }
}

async fn load_page(
    client: semantic_rpc::RpcClient,
    key: &AssociationQueryKey,
) -> std::result::Result<Vec<Object>, String> {
    let payload = query_payload(
        association_query(key),
        key.scope_id.as_deref(),
        Some(association_params(key)),
    );
    let response = client
        .invoke_value("semantic.db.query", payload)
        .await
        .map_err(|error| error.to_string())?;
    let Value::Object(response) = response else {
        return Err("query response must be an object".to_string());
    };
    let Some(Value::List(rows)) = response.get("rows") else {
        return Err("query response missing rows".to_string());
    };
    rows.iter()
        .map(|row| match row {
            Value::Object(object) => Ok(object.clone()),
            _ => Err("query response contains an invalid row".to_string()),
        })
        .collect()
}

fn association_query(key: &AssociationQueryKey) -> semantic_data::query::SelectQuery {
    use semantic_data::query::{BinaryOp, Expr, SortDirection};
    let offset = key.page.saturating_mul(PAGE_SIZE);
    let limit = PAGE_SIZE + 1;
    match key.kind {
        AssociationKind::Children => select("child")
            .with_collection(key.target.collection_or_default())
            .with_projection(vec![wildcard("child")])
            .with_predicate(binary(
                BinaryOp::Eq,
                field(&["child", ATTR_PARENT]),
                Expr::parameter("entity_id"),
            ))
            .with_order_by(vec![
                order(
                    &["child", semantic_data::attr::ATTR_TITLE],
                    SortDirection::Asc,
                ),
                order(&["child", "id"], SortDirection::Asc),
            ])
            .with_limit(limit)
            .with_offset(offset),
        AssociationKind::Relations => select("edge")
            .with_collection(RELATION_EDGES_COLLECTION)
            .with_projection(vec![wildcard("edge")])
            .with_predicate(all([
                binary(
                    BinaryOp::Eq,
                    field(&["edge", "depth"]),
                    Expr::parameter("direct_depth"),
                ),
                any(["source", "target"].map(|name| {
                    binary(
                        BinaryOp::Eq,
                        field(&["edge", name]),
                        Expr::parameter("entity_id"),
                    )
                })),
            ]))
            .with_order_by(
                ["relation", "source", "target"]
                    .map(|name| order(&["edge", name], SortDirection::Asc))
                    .to_vec(),
            )
            .with_limit(limit)
            .with_offset(offset),
    }
}

fn association_params(key: &AssociationQueryKey) -> Object {
    let mut params = Object::new();
    params.insert("entity_id", Value::String(key.target.id.clone()));
    if key.kind == AssociationKind::Relations {
        params.insert("direct_depth", Value::U64(1));
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::{
        attr::{ATTR_RELATION_RELATION, ATTR_RELATION_TO},
        builtin::DEFAULT_COLLECTION,
        bundles::directory::{
            ATTR_DIRECTORY_NODE_FROM, DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID,
            DIRECTORY_NODE_RELATION_ID,
        },
        query::QueryInput,
    };
    use semantic_db_core::{Db, QueryResult};
    use semantic_db_kv::{MemoryBackend, open_memory};

    #[test]
    fn queries_bind_ids_and_fetch_one_extra_row_for_pagination() {
        let key = AssociationQueryKey {
            scope_id: None,
            target: EntityTarget::new(Some("notes".into()), "O'Brien"),
            kind: AssociationKind::Children,
            page: 2,
        };
        let children = association_query(&key);
        assert_eq!(children.collection.as_deref(), Some("notes"));
        assert_eq!(
            children.predicate,
            Some(binary(
                semantic_data::query::BinaryOp::Eq,
                field(&["child", ATTR_PARENT]),
                semantic_data::query::Expr::parameter("entity_id")
            ))
        );
        assert_eq!(
            children.limit,
            Some(semantic_data::query::Expr::from(11usize))
        );
        assert_eq!(children.offset, semantic_data::query::Expr::from(20usize));
        assert_eq!(
            association_params(&key).get("entity_id"),
            Some(&Value::String("O'Brien".into()))
        );

        let relation_key = AssociationQueryKey {
            kind: AssociationKind::Relations,
            ..key
        };
        let relations = association_query(&relation_key);
        assert_eq!(
            relations.collection.as_deref(),
            Some(RELATION_EDGES_COLLECTION)
        );
        assert_eq!(
            relations.limit,
            Some(semantic_data::query::Expr::from(11usize))
        );
        assert_eq!(relations.offset, semantic_data::query::Expr::from(20usize));
        assert_eq!(
            association_params(&relation_key).get("direct_depth"),
            Some(&Value::U64(1))
        );
    }

    fn query_input(key: &AssociationQueryKey) -> QueryInput {
        QueryInput::ast_with_params(
            association_query(key),
            association_params(key).into_iter().collect(),
        )
    }

    #[tokio::test]
    async fn pages_children_and_finds_direct_relations_in_both_directions() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package()).await.unwrap();

        for id in ["hub", "left", "right"] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            object.insert("type", Value::String(DIRECTORY_CLASS_ID.into()));
            object.insert("title", Value::String(id.into()));
            db.insert(DEFAULT_COLLECTION, id, object).await.unwrap();
        }
        for index in 0..11 {
            let id = format!("child-{index:02}");
            let mut object = Object::new();
            object.insert("id", Value::String(id.clone()));
            object.insert("type", Value::String(DIRECTORY_CLASS_ID.into()));
            object.insert(ATTR_PARENT, Value::String("hub".into()));
            object.insert("title", Value::String(id.clone()));
            db.insert(DEFAULT_COLLECTION, &id, object).await.unwrap();
        }
        for (id, source, destination) in [("out", "hub", "right"), ("in", "left", "hub")] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            object.insert("type", Value::String(DIRECTORY_NODE_CLASS_ID.into()));
            object.insert(
                ATTR_RELATION_RELATION,
                Value::String(DIRECTORY_NODE_RELATION_ID.into()),
            );
            object.insert(ATTR_DIRECTORY_NODE_FROM, Value::String(source.into()));
            object.insert(ATTR_RELATION_TO, Value::String(destination.into()));
            db.insert(DEFAULT_COLLECTION, id, object).await.unwrap();
        }

        let key = AssociationQueryKey {
            scope_id: None,
            target: EntityTarget::default_collection("hub"),
            kind: AssociationKind::Children,
            page: 0,
        };
        let QueryResult::Select(first_page) = db.query(query_input(&key)).await.unwrap() else {
            panic!("children query must select rows");
        };
        assert_eq!(first_page.len(), PAGE_SIZE + 1);
        let QueryResult::Select(second_page) = db
            .query(query_input(&AssociationQueryKey {
                page: 1,
                ..key.clone()
            }))
            .await
            .unwrap()
        else {
            panic!("children query must select rows");
        };
        assert_eq!(second_page.len(), 1);

        let relation_key = AssociationQueryKey {
            kind: AssociationKind::Relations,
            ..key
        };
        let QueryResult::Select(first_relations_page) =
            db.query(query_input(&relation_key)).await.unwrap()
        else {
            panic!("relations query must select rows");
        };
        assert_eq!(first_relations_page.len(), PAGE_SIZE + 1);
        let QueryResult::Select(relations) = db
            .query(query_input(&AssociationQueryKey {
                page: 1,
                ..relation_key
            }))
            .await
            .unwrap()
        else {
            panic!("relations query must select rows");
        };
        let endpoints = relations
            .iter()
            .map(|row| {
                (
                    row.get("source").and_then(Value::as_str).unwrap(),
                    row.get("target").and_then(Value::as_str).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            endpoints,
            vec![("child-10", "hub"), ("hub", "right"), ("left", "hub")]
        );
    }
}
