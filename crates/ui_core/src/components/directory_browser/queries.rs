use std::collections::BTreeSet;

use super::types::DirectorySort;
use crate::query_ast::{
    all, any, binary, field, ilike, order, projection, select, string, wildcard,
};
use semantic_base::directory_query::{
    DirectoryChildFilter, DirectoryQueryPage, DirectorySort as QuerySort, directories_query_ast,
    directory_children_query_ast, directory_links_query_ast, directory_parent_query_ast,
    directory_tree_items_query_ast,
};
use semantic_data::attr::{
    ATTR_CREATED_AT, ATTR_PARENT, ATTR_RELATION_RELATION, ATTR_RELATION_TO, ATTR_TITLE,
    ATTR_UPDATED_AT,
};
use semantic_data::bundles::directory::{
    ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, DIRECTORY_CLASS_ID,
    DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
};
use semantic_data::query::{
    AggregateOp, BinaryOp, Expr, FunctionArg, JoinCondition, JoinQuery, JoinSource, JoinType,
    SelectQuery, SortDirection, UnaryOp,
};

pub(super) const ENTITIES_COLLECTION: &str = semantic_data::builtin::DEFAULT_COLLECTION;

pub(super) fn root_query() -> SelectQuery {
    directories_query_ast(DirectoryQueryPage::All)
}
pub(super) fn directory_nodes_query() -> SelectQuery {
    directory_links_query_ast(DirectoryQueryPage::All)
}
pub(super) fn file_tree_items_query() -> SelectQuery {
    directory_tree_items_query_ast(DirectoryQueryPage::All)
}

fn eq(alias: &str, name: &str, value: &str) -> Expr {
    binary(BinaryOp::Eq, field(&[alias, name]), string(value))
}
fn node_relation() -> Expr {
    eq("n", ATTR_RELATION_RELATION, DIRECTORY_NODE_RELATION_ID)
}
fn node_orders() -> Vec<semantic_data::query::OrderBy> {
    vec![
        order(&["n", ATTR_DIRECTORY_NODE_ORDER], SortDirection::Asc),
        order(&["n", "id"], SortDirection::Asc),
    ]
}
fn node_projection() -> Vec<semantic_data::query::QueryField> {
    vec![
        projection(field(&["n", "id"]), "id"),
        projection(field(&["n", ATTR_RELATION_TO]), "directory_to"),
        projection(field(&["n", ATTR_DIRECTORY_NODE_ORDER]), "directory_order"),
    ]
}
fn in_list(expr: Expr, values: impl IntoIterator<Item = impl AsRef<str>>, negated: bool) -> Expr {
    Expr::InList {
        expr: Box::new(expr),
        list: values
            .into_iter()
            .map(|value| string(value.as_ref()))
            .collect(),
        negated,
    }
}
fn in_query(expr: Expr, query: SelectQuery, negated: bool) -> Expr {
    let expr = binary(BinaryOp::In, expr, Expr::Subquery(Box::new(query)));
    if negated {
        Expr::Unary {
            op: UnaryOp::Not,
            expr: Box::new(expr),
        }
    } else {
        expr
    }
}

pub(super) fn child_links_query(parent_id: &str, child_ids: &[String]) -> SelectQuery {
    select("n")
        .with_projection(node_projection())
        .with_predicate(all([
            node_relation(),
            eq("n", ATTR_DIRECTORY_NODE_FROM, parent_id),
            in_list(field(&["n", ATTR_RELATION_TO]), child_ids, false),
        ]))
        .with_order_by(node_orders())
}

#[allow(dead_code)]
pub(super) fn child_ids_query(parent_id: &str) -> SelectQuery {
    select("n")
        .with_projection(vec![projection(
            field(&["n", ATTR_RELATION_TO]),
            "directory_to",
        )])
        .with_predicate(all([
            node_relation(),
            eq("n", ATTR_DIRECTORY_NODE_FROM, parent_id),
        ]))
        .with_order_by(node_orders())
}

pub(super) fn parent_links_query(child_id: &str) -> SelectQuery {
    select("n")
        .with_projection(vec![
            projection(field(&["n", "id"]), "id"),
            projection(field(&["n", ATTR_DIRECTORY_NODE_FROM]), "directory_from"),
            projection(field(&["n", ATTR_DIRECTORY_NODE_ORDER]), "directory_order"),
        ])
        .with_predicate(all([node_relation(), eq("n", ATTR_RELATION_TO, child_id)]))
        .with_order_by(node_orders())
}

pub(super) fn parent_count_query(child_id: &str) -> SelectQuery {
    select("n")
        .with_projection(vec![projection(
            Expr::Aggregate {
                op: AggregateOp::Count,
                distinct: false,
                arg: Box::new(FunctionArg::Wildcard),
            },
            "parent_count",
        )])
        .with_predicate(all([node_relation(), eq("n", ATTR_RELATION_TO, child_id)]))
}

pub(super) fn directory_outgoing_links_query(directory_id: &str) -> SelectQuery {
    select("n")
        .with_projection(node_projection())
        .with_predicate(all([
            node_relation(),
            eq("n", ATTR_DIRECTORY_NODE_FROM, directory_id),
        ]))
        .with_order_by(node_orders())
}

pub(super) fn max_child_order_query(parent_id: &str) -> SelectQuery {
    select("n")
        .with_projection(vec![projection(
            Expr::Aggregate {
                op: AggregateOp::Max,
                distinct: false,
                arg: Box::new(FunctionArg::Expr(field(&["n", ATTR_DIRECTORY_NODE_ORDER]))),
            },
            "max_order",
        )])
        .with_predicate(all([
            node_relation(),
            eq("n", ATTR_DIRECTORY_NODE_FROM, parent_id),
        ]))
}

pub(super) fn addable_entities_query(parent_id: &str, search: &str, limit: usize) -> SelectQuery {
    let excluded = select("n")
        .with_projection(vec![projection(
            field(&["n", ATTR_RELATION_TO]),
            ATTR_RELATION_TO,
        )])
        .with_predicate(all([
            node_relation(),
            eq("n", ATTR_DIRECTORY_NODE_FROM, parent_id),
        ]));
    select("e")
        .with_projection(vec![wildcard("e")])
        .with_predicate(all([
            binary(
                BinaryOp::NotEq,
                field(&["e", "type"]),
                string(DIRECTORY_NODE_CLASS_ID),
            ),
            binary(BinaryOp::NotEq, field(&["e", "id"]), string(parent_id)),
            in_query(field(&["e", "id"]), excluded, true),
            search_predicate(search),
        ]))
        .with_order_by(vec![
            order(&["e", "title"], SortDirection::Asc),
            order(&["e", "id"], SortDirection::Asc),
        ])
        .with_limit(limit)
}

#[allow(dead_code)]
pub(super) fn entity_autocomplete_query(
    search: &str,
    excluded_ids: &BTreeSet<String>,
) -> SelectQuery {
    let mut predicates = vec![
        binary(
            BinaryOp::NotEq,
            field(&["e", "type"]),
            string(DIRECTORY_NODE_CLASS_ID),
        ),
        search_predicate(search),
    ];
    if !excluded_ids.is_empty() {
        predicates.push(in_list(field(&["e", "id"]), excluded_ids, true));
    }
    select("e")
        .with_projection(vec![wildcard("e")])
        .with_predicate(all(predicates))
        .with_order_by(vec![
            order(&["e", "title"], SortDirection::Asc),
            order(&["e", "id"], SortDirection::Asc),
        ])
        .with_limit(50usize)
}

pub(super) fn child_query(
    parent_id: &str,
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> SelectQuery {
    directory_children_query_ast(
        parent_id,
        DirectoryChildFilter::All,
        query_sort(sort),
        DirectoryQueryPage::new(limit, offset),
    )
}
pub(super) fn child_directories_query(parent_id: &str, limit: usize, offset: usize) -> SelectQuery {
    directory_children_query_ast(
        parent_id,
        DirectoryChildFilter::Directories,
        QuerySort::Order,
        DirectoryQueryPage::new(limit, offset),
    )
}
pub(super) fn parent_query(child_id: &str) -> SelectQuery {
    directory_parent_query_ast(child_id)
}

/// Every existing entity referenced as a canonical semantic parent.
/// The uncorrelated subquery deduplicates parents and excludes dangling IDs before pagination.
pub(super) fn semantic_parent_roots_query(
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> SelectQuery {
    let parents = select("child")
        .with_projection(vec![projection(
            field(&["child", ATTR_PARENT]),
            ATTR_PARENT,
        )])
        .with_distinct(true);
    select("parent")
        .with_projection(vec![wildcard("parent")])
        .with_predicate(in_query(field(&["parent", "id"]), parents, false))
        .with_order_by(semantic_order_by(sort, "parent"))
        .with_limit(limit)
        .with_offset(offset)
}

pub(super) fn semantic_children_query(
    parent_id: &str,
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> SelectQuery {
    select("child")
        .with_projection(vec![wildcard("child")])
        .with_predicate(eq("child", ATTR_PARENT, parent_id))
        .with_order_by(semantic_order_by(sort, "child"))
        .with_limit(limit)
        .with_offset(offset)
}

/// Filter navigable children before pagination using an uncorrelated parent-ID lookup.
pub(super) fn semantic_navigable_children_query(
    parent_id: &str,
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> SelectQuery {
    let parents = select("descendant")
        .with_projection(vec![projection(
            field(&["descendant", ATTR_PARENT]),
            ATTR_PARENT,
        )])
        .with_distinct(true);
    semantic_children_query(parent_id, sort, limit, offset).with_predicate(all([
        eq("child", ATTR_PARENT, parent_id),
        any([
            eq("child", "type", DIRECTORY_CLASS_ID),
            in_query(field(&["child", "id"]), parents, false),
        ]),
    ]))
}

pub(super) fn semantic_parent_query(child_id: &str) -> SelectQuery {
    select("child")
        .with_projection(vec![wildcard("parent")])
        .with_joins(vec![JoinQuery {
            source: JoinSource {
                collection: Some(ENTITIES_COLLECTION.into()),
                class: None,
            },
            alias: Some("parent".into()),
            join_type: JoinType::Inner,
            condition: JoinCondition::OnExpr(binary(
                BinaryOp::Eq,
                field(&["child", ATTR_PARENT]),
                field(&["parent", "id"]),
            )),
            predicate: None,
        }])
        .with_predicate(eq("child", "id", child_id))
        .with_limit(1usize)
}

pub(super) fn semantic_parent_ids_for_candidates_query(candidate_ids: &[String]) -> SelectQuery {
    select("child")
        .with_projection(vec![projection(
            field(&["child", ATTR_PARENT]),
            "semantic_parent",
        )])
        .with_distinct(true)
        .with_predicate(in_list(
            field(&["child", ATTR_PARENT]),
            candidate_ids,
            false,
        ))
        .with_order_by(vec![order(&["child", ATTR_PARENT], SortDirection::Asc)])
}

fn query_sort(sort: DirectorySort) -> QuerySort {
    match sort {
        DirectorySort::Order => QuerySort::Order,
        DirectorySort::TitleAsc => QuerySort::TitleAsc,
        DirectorySort::TitleDesc => QuerySort::TitleDesc,
        DirectorySort::TypeAsc => QuerySort::TypeAsc,
        DirectorySort::CreatedAtDesc => QuerySort::CreatedAtDesc,
        DirectorySort::UpdatedAtDesc => QuerySort::UpdatedAtDesc,
        DirectorySort::IdAsc => QuerySort::IdAsc,
    }
}
fn semantic_order_by(sort: DirectorySort, alias: &str) -> Vec<semantic_data::query::OrderBy> {
    use SortDirection::{Asc, Desc};
    match sort {
        DirectorySort::Order | DirectorySort::TitleAsc => {
            vec![order(&[alias, "title"], Asc), order(&[alias, "id"], Asc)]
        }
        DirectorySort::TitleDesc => {
            vec![order(&[alias, "title"], Desc), order(&[alias, "id"], Asc)]
        }
        DirectorySort::TypeAsc => vec![
            order(&[alias, "type"], Asc),
            order(&[alias, "title"], Asc),
            order(&[alias, "id"], Asc),
        ],
        DirectorySort::CreatedAtDesc => vec![
            order(&[alias, ATTR_CREATED_AT], Desc),
            order(&[alias, "title"], Asc),
            order(&[alias, "id"], Asc),
        ],
        DirectorySort::UpdatedAtDesc => vec![
            order(&[alias, ATTR_UPDATED_AT], Desc),
            order(&[alias, "title"], Asc),
            order(&[alias, "id"], Asc),
        ],
        DirectorySort::IdAsc => vec![order(&[alias, "id"], Asc)],
    }
}
fn search_predicate(search: &str) -> Expr {
    let pattern = format!("%{search}%");
    any(["id", "title", ATTR_TITLE, "type"]
        .into_iter()
        .map(|name| ilike(field(&["e", name]), &pattern)))
}

#[cfg(test)]
mod tests {
    use semantic_data::bundles::directory::{
        ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, DIRECTORY_CLASS_ID,
        DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
    };
    use semantic_data::query::{Batch, BatchOperation, QueryInput};
    use semantic_data::value::{Object, Value};
    use semantic_db_core::{Db, QueryResult};
    use semantic_db_kv::{MemoryBackend, open_memory};

    use super::*;

    fn contains_string(query: &SelectQuery, text: &str) -> bool {
        use semantic_data::value::IntoValue;
        fn walk(value: &Value, text: &str) -> bool {
            match value {
                Value::String(value) => value == text,
                Value::List(values) => values.iter().any(|value| walk(value, text)),
                Value::Object(values) => values.values().any(|value| walk(value, text)),
                _ => false,
            }
        }
        walk(&query.clone().into_value(), text)
    }

    #[test]
    fn root_query_uses_directory_class() {
        let query = root_query();
        assert_eq!(query.collection.as_deref(), Some(ENTITIES_COLLECTION));
        assert_eq!(query.source_alias.as_deref(), Some("d"));
        assert_eq!(
            query.predicate,
            Some(in_list(field(&["d", "type"]), [DIRECTORY_CLASS_ID], false))
        );
        assert_eq!(
            query.field_format,
            semantic_data::query::FieldFormat::Qualified
        );
    }

    #[tokio::test]
    async fn root_query_finds_directory_entities() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "dir1", "Dir1").await;

        let result = db
            .query(QueryInput::from(root_query()))
            .await
            .expect("root query should run");
        let QueryResult::Select(rows) = result else {
            panic!("root query should return select rows");
        };

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("id"), Some(&Value::String("dir1".to_string())));
        assert_eq!(
            rows[0].get("type"),
            Some(&Value::String(DIRECTORY_CLASS_ID.to_string()))
        );
        assert_eq!(
            rows[0].get(ATTR_TITLE),
            Some(&Value::String("Dir1".to_string()))
        );
        assert!(!rows[0].contains_key("title"));
    }

    #[tokio::test]
    async fn directory_nodes_query_returns_child_ids() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "dir1", "Dir1").await;
        insert_directory(&db, "parent", "Parent").await;
        insert_directory_node(&db, "node-1", "parent", "dir1", 10).await;

        let result = db
            .query(QueryInput::from(directory_nodes_query()))
            .await
            .expect("directory nodes query should run");
        let QueryResult::Select(rows) = result else {
            panic!("directory nodes query should return select rows");
        };

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("directory_to"),
            Some(&Value::String("dir1".to_string()))
        );
    }

    #[tokio::test]
    async fn root_query_includes_directory_with_parent_node_for_rust_filtering() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_directory_node(&db, "node-1", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::from(root_query()))
            .await
            .expect("root query should run");
        let QueryResult::Select(rows) = result else {
            panic!("root query should return select rows");
        };

        assert_eq!(row_ids(&rows), vec!["child-dir", "parent"]);
    }

    #[tokio::test]
    async fn root_query_ignores_non_directory_entities() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "dir1", "Dir1").await;
        insert_entity(&db, "person1", "semantic:base:person", "Person 1").await;

        let result = db
            .query(QueryInput::from(root_query()))
            .await
            .expect("root query should run");
        let QueryResult::Select(rows) = result else {
            panic!("root query should return select rows");
        };

        assert_eq!(row_ids(&rows), vec!["dir1"]);
    }

    #[tokio::test]
    async fn child_query_finds_items_for_parent_directory() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_entity(&db, "child-item", "semantic:base:person", "Child Item").await;
        insert_directory_node(&db, "node-1", "parent", "child-item", 20).await;
        insert_directory_node(&db, "node-2", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::from(child_query(
                "parent",
                DirectorySort::Order,
                200,
                0,
            )))
            .await
            .expect("child query should run");
        let QueryResult::Select(rows) = result else {
            panic!("child query should return select rows");
        };

        let ids = row_ids(&rows);
        assert_eq!(ids, vec!["child-dir", "child-item"]);
        assert_eq!(rows[0].get("directory_order"), Some(&Value::U64(10)));
        assert_eq!(rows[1].get("directory_order"), Some(&Value::U64(20)));
    }

    #[tokio::test]
    async fn child_directories_query_filters_to_directory_children() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_entity(&db, "child-item", "semantic:base:person", "Child Item").await;
        insert_directory_node(&db, "node-1", "parent", "child-item", 20).await;
        insert_directory_node(&db, "node-2", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::from(child_directories_query("parent", 200, 0)))
            .await
            .expect("child directories query should run");
        let QueryResult::Select(rows) = result else {
            panic!("child directories query should return select rows");
        };

        assert_eq!(row_ids(&rows), vec!["child-dir"]);
        assert_eq!(rows[0].get("directory_order"), Some(&Value::U64(10)));
    }

    #[tokio::test]
    async fn parent_query_finds_parent_directory_id() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_directory_node(&db, "node-1", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::from(parent_query("child-dir")))
            .await
            .expect("parent query should run");
        let QueryResult::Select(rows) = result else {
            panic!("parent query should return select rows");
        };

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("directory_from"),
            Some(&Value::String("parent".to_string()))
        );
    }

    #[tokio::test]
    async fn parent_query_returns_lowest_order_parent_node() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent-a", "Parent A").await;
        insert_directory(&db, "parent-b", "Parent B").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_directory_node(&db, "node-high", "parent-a", "child-dir", 20).await;
        insert_directory_node(&db, "node-low", "parent-b", "child-dir", 10).await;

        let result = db
            .query(QueryInput::from(parent_query("child-dir")))
            .await
            .expect("parent query should run");
        let QueryResult::Select(rows) = result else {
            panic!("parent query should return select rows");
        };

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("directory_from"),
            Some(&Value::String("parent-b".to_string()))
        );
        assert_eq!(rows[0].get("directory_order"), Some(&Value::U64(10)));
    }

    #[test]
    fn child_query_preserves_parent_identifier_and_ordering() {
        let query = child_query("parent'1", DirectorySort::Order, 101, 0);
        assert_eq!(query.limit, Some(Expr::from(101usize)));
        assert_eq!(
            query.order_by[0],
            order(&["n", ATTR_DIRECTORY_NODE_ORDER], SortDirection::Asc)
        );
        assert!(contains_string(&query, "parent'1"));
        assert!(!contains_string(&query, "parent''1"));
    }

    #[test]
    fn child_links_query_preserves_parent_and_child_ids() {
        let query = child_links_query("parent'1", &["child'1".to_string(), "child2".to_string()]);
        for value in ["parent'1", "child'1", "child2"] {
            assert!(contains_string(&query, value));
        }
    }

    #[test]
    fn entity_autocomplete_query_preserves_patterns_and_exclusions() {
        let excluded_ids = BTreeSet::from(["existing'1".to_string(), "existing2".to_string()]);
        let query = entity_autocomplete_query("Ada's", &excluded_ids);
        for value in [
            "%Ada's%",
            "existing'1",
            "existing2",
            DIRECTORY_NODE_CLASS_ID,
        ] {
            assert!(contains_string(&query, value));
        }
        assert_eq!(query.limit, Some(Expr::from(50usize)));
    }

    #[test]
    fn addable_entities_query_preserves_parent_and_limit() {
        let query = addable_entities_query("parent'1", "needle", 25);
        assert!(contains_string(&query, "parent'1"));
        assert_eq!(query.limit, Some(Expr::from(25usize)));
    }

    #[test]
    fn parent_count_and_outgoing_queries_use_directory_relation_fields() {
        let parent_count = parent_count_query("child'1");
        assert_eq!(
            parent_count.projection[0].alias.as_deref(),
            Some("parent_count")
        );
        assert!(matches!(
            *parent_count.projection[0].expr,
            Expr::Aggregate {
                op: AggregateOp::Count,
                ..
            }
        ));
        assert!(contains_string(&parent_count, "child'1"));
        assert!(contains_string(&parent_count, DIRECTORY_NODE_RELATION_ID));
        let outgoing = directory_outgoing_links_query("dir'1");
        assert!(contains_string(&outgoing, "dir'1"));
        assert_eq!(
            outgoing.projection[1].alias.as_deref(),
            Some("directory_to")
        );
    }

    #[tokio::test]
    async fn child_links_query_returns_matching_parent_child_links() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_entity(&db, "child-a", "semantic:base:person", "Child A").await;
        insert_entity(&db, "child-b", "semantic:base:person", "Child B").await;
        insert_directory_node(&db, "node-a", "parent", "child-a", 1).await;
        insert_directory_node(&db, "node-b", "parent", "child-b", 2).await;

        let result = db
            .query(QueryInput::from(child_links_query(
                "parent",
                &["child-b".to_string()],
            )))
            .await
            .expect("child links query should run");
        let QueryResult::Select(rows) = result else {
            panic!("child links query should return select rows");
        };

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("id"),
            Some(&Value::String("node-b".to_string()))
        );
        assert_eq!(
            rows[0].get("directory_to"),
            Some(&Value::String("child-b".to_string()))
        );
    }

    #[tokio::test]
    async fn addable_entities_query_filters_existing_children() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_entity(&db, "existing", "semantic:base:person", "Needle Existing").await;
        insert_entity(&db, "candidate", "semantic:base:person", "Needle Candidate").await;
        insert_directory_node(&db, "node-existing", "parent", "existing", 1).await;

        let result = db
            .query(QueryInput::from(addable_entities_query(
                "parent", "Needle", 50,
            )))
            .await
            .expect("addable entities query should run");
        let QueryResult::Select(rows) = result else {
            panic!("addable entities query should return select rows");
        };

        assert_eq!(row_ids(&rows), vec!["candidate"]);
    }

    #[tokio::test]
    async fn addable_entities_query_searches_id_and_attr_title() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_entity(&db, "find-by-id", "semantic:base:person", "Plain").await;

        let mut attr_title_entity = Object::new();
        attr_title_entity.insert("id", Value::String("attr-title-candidate".to_string()));
        attr_title_entity.insert("type", Value::String("semantic:base:person".to_string()));
        attr_title_entity.insert(ATTR_TITLE, Value::String("Find By Attribute".to_string()));
        db.insert(
            ENTITIES_COLLECTION,
            "attr-title-candidate",
            attr_title_entity,
        )
        .await
        .expect("entity should insert");

        let by_id = db
            .query(QueryInput::from(addable_entities_query(
                "parent",
                "find-by-id",
                50,
            )))
            .await
            .expect("addable entities query should run");
        let QueryResult::Select(by_id_rows) = by_id else {
            panic!("addable entities query should return select rows");
        };

        let by_attr_title = db
            .query(QueryInput::from(addable_entities_query(
                "parent",
                "Attribute",
                50,
            )))
            .await
            .expect("addable entities query should run");
        let QueryResult::Select(by_attr_title_rows) = by_attr_title else {
            panic!("addable entities query should return select rows");
        };

        assert_eq!(row_ids(&by_id_rows), vec!["find-by-id"]);
        assert_eq!(row_ids(&by_attr_title_rows), vec!["attr-title-candidate"]);
    }

    #[test]
    fn sort_options_map_to_stable_ordering() {
        let title = child_query("parent", DirectorySort::TitleDesc, 10, 0);
        assert_eq!(
            title.order_by,
            vec![
                order(&["child", "title"], SortDirection::Desc),
                order(&["child", "id"], SortDirection::Asc)
            ]
        );
        let id = child_query("parent", DirectorySort::IdAsc, 10, 0);
        assert_eq!(
            id.order_by,
            vec![order(&["child", "id"], SortDirection::Asc)]
        );
        let semantic = semantic_children_query("parent", DirectorySort::Order, 10, 0);
        assert_eq!(
            semantic.order_by,
            vec![
                order(&["child", "title"], SortDirection::Asc),
                order(&["child", "id"], SortDirection::Asc)
            ]
        );
        let roots = semantic_parent_roots_query(DirectorySort::Order, 10, 0);
        assert!(
            matches!(roots.predicate, Some(Expr::Binary { op: BinaryOp::In, right, .. }) if matches!(right.as_ref(), Expr::Subquery(query) if query.distinct))
        );
    }

    #[tokio::test]
    async fn semantic_parent_queries_discover_and_page_direct_relationships() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_entity(&db, "parent-a", "semantic:base:person", "Alpha").await;
        insert_entity(&db, "parent-b", "semantic:base:person", "Beta").await;
        insert_entity(&db, "leaf", "semantic:base:person", "Leaf").await;
        insert_parented_entity(&db, "child-a1", "Child A1", Some("parent-a")).await;
        insert_parented_entity(&db, "child-a2", "Child A2", Some("parent-a")).await;
        insert_parented_entity(&db, "nested", "Nested", Some("parent-b")).await;
        insert_parented_entity(&db, "grandchild", "Grandchild", Some("nested")).await;
        insert_parented_entity(&db, "unparented", "Unparented", None).await;

        let QueryResult::Select(first_page) = db
            .query(QueryInput::from(semantic_parent_roots_query(
                DirectorySort::Order,
                2,
                0,
            )))
            .await
            .expect("semantic roots query should run")
        else {
            panic!("semantic roots query should select rows");
        };
        let QueryResult::Select(second_page) = db
            .query(QueryInput::from(semantic_parent_roots_query(
                DirectorySort::Order,
                2,
                2,
            )))
            .await
            .expect("semantic roots query should page")
        else {
            panic!("semantic roots query should select rows");
        };
        assert_eq!(row_ids(&first_page), vec!["parent-a", "parent-b"]);
        assert_eq!(row_ids(&second_page), vec!["nested"]);

        let QueryResult::Select(children) = db
            .query(QueryInput::from(semantic_children_query(
                "parent-a",
                DirectorySort::Order,
                10,
                0,
            )))
            .await
            .expect("semantic children query should run")
        else {
            panic!("semantic children query should select rows");
        };
        assert_eq!(row_ids(&children), vec!["child-a1", "child-a2"]);

        let QueryResult::Select(classified) = db
            .query(QueryInput::from(semantic_parent_ids_for_candidates_query(
                &[
                    "parent-a".to_string(),
                    "leaf".to_string(),
                    "nested".to_string(),
                ],
            )))
            .await
            .expect("batch classification query should run")
        else {
            panic!("batch classification query should select rows");
        };
        let ids = classified
            .iter()
            .filter_map(|row| row.get("semantic_parent").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["nested", "parent-a"]);
    }

    #[tokio::test]
    async fn navigable_semantic_children_are_filtered_before_limit() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_entity(&db, "root", "semantic:base:person", "Root").await;
        let mut batch = Batch::new();
        for index in 0..201 {
            let id = format!("leaf-{index:03}");
            let mut leaf = Object::new();
            leaf.insert("id", Value::String(id.clone()));
            leaf.insert("type", Value::String("semantic:base:person".to_string()));
            leaf.insert("title", Value::String(format!("A leaf {index:03}")));
            leaf.insert(ATTR_PARENT, Value::String("root".to_string()));
            batch = batch.with_op(BatchOperation::Upsert {
                collection: ENTITIES_COLLECTION.to_string(),
                id,
                object: leaf,
            });
        }
        for (id, title, parent) in [
            ("nested", "Z nested", "root"),
            ("grandchild", "Grandchild", "nested"),
        ] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.to_string()));
            object.insert("type", Value::String("semantic:base:person".to_string()));
            object.insert("title", Value::String(title.to_string()));
            object.insert(ATTR_PARENT, Value::String(parent.to_string()));
            batch = batch.with_op(BatchOperation::Upsert {
                collection: ENTITIES_COLLECTION.to_string(),
                id: id.to_string(),
                object,
            });
        }
        db.execute_batch(batch)
            .await
            .expect("semantic hierarchy should insert");

        let query = semantic_navigable_children_query("root", DirectorySort::Order, 200, 0);
        let QueryResult::Select(children) = db
            .query(QueryInput::from(query))
            .await
            .expect("navigable semantic children query should run")
        else {
            panic!("navigable semantic children query should select rows");
        };
        assert_eq!(row_ids(&children), vec!["nested"]);
    }

    #[tokio::test]
    async fn semantic_parent_queries_handle_cycles_and_escaped_ids() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_parented_entities(
            &db,
            &[
                ("self", "Self", Some("self")),
                ("cycle-a", "Cycle A", Some("cycle-b")),
                ("cycle-b", "Cycle B", Some("cycle-a")),
            ],
        )
        .await;
        insert_entity(&db, "quote'parent", "semantic:base:person", "Quoted").await;
        insert_parented_entity(&db, "quoted-child", "Quoted Child", Some("quote'parent")).await;

        let QueryResult::Select(self_children) = db
            .query(QueryInput::from(semantic_children_query(
                "self",
                DirectorySort::Order,
                10,
                0,
            )))
            .await
            .expect("self-cycle query should run")
        else {
            panic!("self-cycle query should select rows");
        };
        assert_eq!(row_ids(&self_children), vec!["self"]);

        let QueryResult::Select(parent) = db
            .query(QueryInput::from(semantic_parent_query("cycle-a")))
            .await
            .expect("semantic parent query should run")
        else {
            panic!("semantic parent query should select rows");
        };
        assert_eq!(row_ids(&parent), vec!["cycle-b"]);

        let QueryResult::Select(quoted_children) = db
            .query(QueryInput::from(semantic_children_query(
                "quote'parent",
                DirectorySort::Order,
                10,
                0,
            )))
            .await
            .expect("escaped parent query should run")
        else {
            panic!("escaped parent query should select rows");
        };
        assert_eq!(row_ids(&quoted_children), vec!["quoted-child"]);
    }

    #[tokio::test]
    async fn empty_candidates_and_directory_aggregates_are_direct_asts() {
        let db = Db::new(MemoryBackend::new(open_memory().unwrap()));
        db.upsert_package(semantic_base::package()).await.unwrap();
        insert_directory(&db, "parent'1", "Parent").await;
        insert_entity(&db, "child'1", "semantic:base:person", "Child").await;
        insert_directory_node(&db, "node", "parent'1", "child'1", 7).await;
        for query in [
            child_links_query("parent'1", &[]),
            semantic_parent_ids_for_candidates_query(&[]),
        ] {
            let QueryResult::Select(rows) = db.query(query).await.unwrap() else {
                panic!("expected rows")
            };
            assert!(rows.is_empty());
        }
        let QueryResult::Select(count) = db.query(parent_count_query("child'1")).await.unwrap()
        else {
            panic!("expected rows")
        };
        assert_eq!(count.len(), 1);
        assert_eq!(count[0].get("parent_count"), Some(&Value::I64(1)));
        let QueryResult::Select(max) = db.query(max_child_order_query("parent'1")).await.unwrap()
        else {
            panic!("expected rows")
        };
        assert_eq!(max[0].get("max_order"), Some(&Value::U64(7)));
    }

    async fn insert_directory(db: &Db, id: &str, title: &str) {
        insert_entity(db, id, DIRECTORY_CLASS_ID, title).await;
    }

    async fn insert_entity(db: &Db, id: &str, ty: &str, title: &str) {
        let mut entity = Object::new();
        entity.insert("id", Value::String(id.to_string()));
        entity.insert("type", Value::String(ty.to_string()));
        entity.insert("title", Value::String(title.to_string()));
        db.insert(ENTITIES_COLLECTION, id, entity)
            .await
            .expect("entity should insert");
    }

    async fn insert_parented_entity(db: &Db, id: &str, title: &str, parent: Option<&str>) {
        insert_parented_entities(db, &[(id, title, parent)]).await;
    }

    async fn insert_parented_entities(db: &Db, entities: &[(&str, &str, Option<&str>)]) {
        let mut batch = Batch::new();
        for (id, title, parent) in entities {
            let mut object = Object::new();
            object.insert("id", Value::String((*id).to_string()));
            object.insert("type", Value::String("semantic:base:person".to_string()));
            object.insert("title", Value::String((*title).to_string()));
            if let Some(parent) = parent {
                object.insert(ATTR_PARENT, Value::String((*parent).to_string()));
            }
            batch = batch.with_op(BatchOperation::Upsert {
                collection: ENTITIES_COLLECTION.to_string(),
                id: (*id).to_string(),
                object,
            });
        }
        db.execute_batch(batch)
            .await
            .expect("parented entities should insert");
    }

    async fn insert_directory_node(db: &Db, id: &str, parent: &str, child: &str, order: u64) {
        let mut node = Object::new();
        node.insert("id", Value::String(id.to_string()));
        node.insert("type", Value::String(DIRECTORY_NODE_CLASS_ID.to_string()));
        node.insert(
            ATTR_RELATION_RELATION,
            Value::String(DIRECTORY_NODE_RELATION_ID.to_string()),
        );
        node.insert(ATTR_DIRECTORY_NODE_FROM, Value::String(parent.to_string()));
        node.insert(ATTR_RELATION_TO, Value::String(child.to_string()));
        node.insert(ATTR_DIRECTORY_NODE_ORDER, Value::U64(order));
        db.insert(ENTITIES_COLLECTION, id, node)
            .await
            .expect("directory node should insert");
    }

    fn row_ids(rows: &[Object]) -> Vec<&str> {
        rows.iter()
            .map(|row| row.get("id").and_then(Value::as_str).expect("row id"))
            .collect()
    }
}
