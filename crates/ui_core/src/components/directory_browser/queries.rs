use std::collections::BTreeSet;

pub(super) use semantic_base::directory_query::{
    ATTR_RELATION_RELATION, ATTR_RELATION_TO, sql_ident, sql_string,
};
use semantic_base::directory_query::{
    DirectoryChildFilter, DirectoryQueryPage, DirectorySort as QuerySort, directories_query,
    directory_children_query, directory_links_query, directory_parent_query,
    directory_tree_items_query,
};
use semantic_data::bundles::directory::{
    ATTR_CREATED_AT, ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, ATTR_TITLE,
    ATTR_UPDATED_AT, DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
};
use semantic_data::filestore::ATTR_PARENT;

use super::types::DirectorySort;

pub(super) const ENTITIES_COLLECTION: &str = semantic_data::builtin::DEFAULT_COLLECTION;

pub(super) fn root_query() -> String {
    directories_query(DirectoryQueryPage::All)
}

pub(super) fn directory_nodes_query() -> String {
    directory_links_query(DirectoryQueryPage::All)
}

pub(super) fn file_tree_items_query() -> String {
    directory_tree_items_query(DirectoryQueryPage::All)
}

pub(super) fn child_links_query(parent_id: &str, child_ids: &[String]) -> String {
    if child_ids.is_empty() {
        return qualified_query(format!(
            "SELECT n.id AS id, n.{relation_to} AS directory_to FROM {entities} AS n WHERE 1 = 0",
            entities = sql_ident(ENTITIES_COLLECTION),
            relation_to = sql_ident(ATTR_RELATION_TO),
        ));
    }
    qualified_query(format!(
        "SELECT n.id AS id, n.{relation_to} AS directory_to, n.{node_order} AS directory_order FROM {entities} AS n WHERE n.{relation_relation} = {node_relation} AND n.{node_from} = {parent_id} AND n.{relation_to} IN ({child_ids}) ORDER BY n.{node_order} ASC, n.id ASC",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        parent_id = sql_string(parent_id),
        child_ids = sql_string_list(child_ids),
    ))
}

#[allow(dead_code)]
pub(super) fn child_ids_query(parent_id: &str) -> String {
    qualified_query(format!(
        "SELECT n.{relation_to} AS directory_to FROM {entities} AS n WHERE n.{relation_relation} = {node_relation} AND n.{node_from} = {parent_id} ORDER BY n.{node_order} ASC, n.id ASC",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        parent_id = sql_string(parent_id),
    ))
}

pub(super) fn parent_links_query(child_id: &str) -> String {
    qualified_query(format!(
        "SELECT n.id AS id, n.{node_from} AS directory_from, n.{node_order} AS directory_order FROM {entities} AS n WHERE n.{relation_relation} = {node_relation} AND n.{relation_to} = {child_id} ORDER BY n.{node_order} ASC, n.id ASC",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        child_id = sql_string(child_id),
    ))
}

pub(super) fn parent_count_query(child_id: &str) -> String {
    qualified_query(format!(
        "SELECT COUNT(*) AS parent_count FROM {entities} AS n WHERE n.{relation_relation} = {node_relation} AND n.{relation_to} = {child_id}",
        entities = sql_ident(ENTITIES_COLLECTION),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        child_id = sql_string(child_id),
    ))
}

pub(super) fn directory_outgoing_links_query(directory_id: &str) -> String {
    qualified_query(format!(
        "SELECT n.id AS id, n.{relation_to} AS directory_to, n.{node_order} AS directory_order FROM {entities} AS n WHERE n.{relation_relation} = {node_relation} AND n.{node_from} = {directory_id} ORDER BY n.{node_order} ASC, n.id ASC",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        directory_id = sql_string(directory_id),
    ))
}

pub(super) fn max_child_order_query(parent_id: &str) -> String {
    qualified_query(format!(
        "SELECT MAX(n.{node_order}) AS max_order FROM {entities} AS n WHERE n.{relation_relation} = {node_relation} AND n.{node_from} = {parent_id}",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        node_order = sql_ident(ATTR_DIRECTORY_NODE_ORDER),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        parent_id = sql_string(parent_id),
    ))
}

pub(super) fn addable_entities_query(parent_id: &str, search: &str, limit: usize) -> String {
    let excluded = format!(
        "SELECT n.{relation_to} FROM {entities} AS n WHERE n.{relation_relation} = {node_relation} AND n.{node_from} = {parent_id}",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_from = sql_ident(ATTR_DIRECTORY_NODE_FROM),
        relation_relation = sql_ident(ATTR_RELATION_RELATION),
        relation_to = sql_ident(ATTR_RELATION_TO),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        parent_id = sql_string(parent_id),
    );
    let predicates = search_predicate(search);
    qualified_query(format!(
        "SELECT e.* FROM {entities} AS e WHERE e.type != {node_class} AND e.id != {parent_id} AND e.id NOT IN ({excluded}) AND ({predicates}) ORDER BY e.title ASC, e.id ASC LIMIT {limit}",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_class = sql_string(DIRECTORY_NODE_CLASS_ID),
        parent_id = sql_string(parent_id),
        excluded = excluded,
        predicates = predicates,
        limit = limit,
    ))
}

#[allow(dead_code)]
pub(super) fn entity_autocomplete_query(search: &str, excluded_ids: &BTreeSet<String>) -> String {
    let predicates = search_predicate(search);
    let exclusion = if excluded_ids.is_empty() {
        String::new()
    } else {
        format!(" AND e.id NOT IN ({})", sql_string_set(excluded_ids))
    };
    qualified_query(format!(
        "SELECT e.* FROM {entities} AS e WHERE e.type != {node_class}{exclusion} AND ({predicates}) ORDER BY e.title ASC, e.id ASC LIMIT 50",
        entities = sql_ident(ENTITIES_COLLECTION),
        node_class = sql_string(DIRECTORY_NODE_CLASS_ID),
        exclusion = exclusion,
        predicates = predicates,
    ))
}

pub(super) fn child_query(
    parent_id: &str,
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> String {
    directory_children_query(
        parent_id,
        DirectoryChildFilter::All,
        query_sort(sort),
        DirectoryQueryPage::new(limit, offset),
    )
}

pub(super) fn child_directories_query(parent_id: &str, limit: usize, offset: usize) -> String {
    directory_children_query(
        parent_id,
        DirectoryChildFilter::Directories,
        QuerySort::Order,
        DirectoryQueryPage::new(limit, offset),
    )
}

pub(super) fn parent_query(child_id: &str) -> String {
    directory_parent_query(child_id)
}

/// Returns every existing entity referenced as a canonical semantic parent.
///
/// Pagination happens in SQL and the uncorrelated `IN` subquery both deduplicates
/// high fan-out parents and excludes dangling parent IDs.
pub(super) fn semantic_parent_roots_query(
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> String {
    qualified_query(format!(
        "SELECT parent.* FROM {entities} AS parent WHERE parent.id IN (SELECT DISTINCT child.{parent_attr} FROM {entities} AS child) ORDER BY {order_by} LIMIT {limit} OFFSET {offset}",
        entities = sql_ident(ENTITIES_COLLECTION),
        parent_attr = sql_ident(ATTR_PARENT),
        order_by = semantic_order_by(sort, "parent"),
    ))
}

pub(super) fn semantic_children_query(
    parent_id: &str,
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> String {
    qualified_query(format!(
        "SELECT child.* FROM {entities} AS child WHERE child.{parent_attr} = {parent_id} ORDER BY {order_by} LIMIT {limit} OFFSET {offset}",
        entities = sql_ident(ENTITIES_COLLECTION),
        parent_attr = sql_ident(ATTR_PARENT),
        parent_id = sql_string(parent_id),
        order_by = semantic_order_by(sort, "child"),
    ))
}

/// Returns semantic children that can be navigated further in the tree.
///
/// The parent-ID lookup is intentionally uncorrelated so filtering happens
/// before pagination without relying on correlated subquery support.
pub(super) fn semantic_navigable_children_query(
    parent_id: &str,
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> String {
    qualified_query(format!(
        "SELECT child.* FROM {entities} AS child WHERE child.{parent_attr} = {parent_id} AND (child.type = {directory_class} OR child.id IN (SELECT DISTINCT descendant.{parent_attr} FROM {entities} AS descendant)) ORDER BY {order_by} LIMIT {limit} OFFSET {offset}",
        entities = sql_ident(ENTITIES_COLLECTION),
        parent_attr = sql_ident(ATTR_PARENT),
        parent_id = sql_string(parent_id),
        directory_class = sql_string(DIRECTORY_CLASS_ID),
        order_by = semantic_order_by(sort, "child"),
    ))
}

pub(super) fn semantic_parent_query(child_id: &str) -> String {
    qualified_query(format!(
        "SELECT parent.* FROM {entities} AS child INNER JOIN {entities}._ AS parent ON child.{parent_attr} = parent.id WHERE child.id = {child_id} LIMIT 1",
        entities = sql_ident(ENTITIES_COLLECTION),
        parent_attr = sql_ident(ATTR_PARENT),
        child_id = sql_string(child_id),
    ))
}

pub(super) fn semantic_parent_ids_for_candidates_query(candidate_ids: &[String]) -> String {
    if candidate_ids.is_empty() {
        return qualified_query(format!(
            "SELECT child.{parent_attr} AS semantic_parent FROM {entities} AS child WHERE 1 = 0",
            entities = sql_ident(ENTITIES_COLLECTION),
            parent_attr = sql_ident(ATTR_PARENT),
        ));
    }
    qualified_query(format!(
        "SELECT DISTINCT child.{parent_attr} AS semantic_parent FROM {entities} AS child WHERE child.{parent_attr} IN ({candidate_ids}) ORDER BY semantic_parent ASC",
        entities = sql_ident(ENTITIES_COLLECTION),
        parent_attr = sql_ident(ATTR_PARENT),
        candidate_ids = sql_string_list(candidate_ids),
    ))
}

fn qualified_query(query: String) -> String {
    format!("{query} FORMAT qualified")
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

fn semantic_order_by(sort: DirectorySort, alias: &str) -> String {
    match sort {
        DirectorySort::Order | DirectorySort::TitleAsc => {
            format!("{alias}.title ASC, {alias}.id ASC")
        }
        DirectorySort::TitleDesc => format!("{alias}.title DESC, {alias}.id ASC"),
        DirectorySort::TypeAsc => {
            format!("{alias}.type ASC, {alias}.title ASC, {alias}.id ASC")
        }
        DirectorySort::CreatedAtDesc => format!(
            "{alias}.{created_at} DESC, {alias}.title ASC, {alias}.id ASC",
            created_at = sql_ident(ATTR_CREATED_AT),
        ),
        DirectorySort::UpdatedAtDesc => format!(
            "{alias}.{updated_at} DESC, {alias}.title ASC, {alias}.id ASC",
            updated_at = sql_ident(ATTR_UPDATED_AT),
        ),
        DirectorySort::IdAsc => format!("{alias}.id ASC"),
    }
}

fn search_predicate(search: &str) -> String {
    let pattern = sql_string(&format!("%{}%", search));
    format!(
        "e.id ILIKE {pattern} OR e.title ILIKE {pattern} OR e.{title_attr} ILIKE {pattern} OR e.type ILIKE {pattern}",
        title_attr = sql_ident(ATTR_TITLE),
    )
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| sql_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[allow(dead_code)]
fn sql_string_set(values: &BTreeSet<String>) -> String {
    values
        .iter()
        .map(|value| sql_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use semantic_data::bundles::directory::{
        ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, ATTR_TITLE, DIRECTORY_CLASS_ID,
        DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
    };
    use semantic_data::query::{Batch, BatchOperation, QueryInput};
    use semantic_data::value::{Object, Value};
    use semantic_db_core::{Db, QueryResult};
    use semantic_db_kv::{KvBackend, KvDb};

    use super::*;

    #[test]
    fn sql_string_escapes_quotes() {
        assert_eq!(sql_string("a'b"), "'a''b'");
    }

    #[test]
    fn sql_ident_quotes_non_plain_identifiers() {
        assert_eq!(sql_ident("entities"), "entities");
        assert_eq!(
            sql_ident("semantic:base:directory_node"),
            "\"semantic:base:directory_node\""
        );
    }

    #[test]
    fn root_query_uses_directory_class() {
        let query = root_query();
        assert!(query.contains(DIRECTORY_CLASS_ID));
        assert!(query.contains(ENTITIES_COLLECTION));
        assert!(query.contains("d.type"));
        assert!(!query.contains("NOT IN"));
    }

    #[tokio::test]
    async fn root_query_finds_directory_entities() {
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "dir1", "Dir1").await;

        let result = db
            .query(QueryInput::sql(root_query()))
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "dir1", "Dir1").await;
        insert_directory(&db, "parent", "Parent").await;
        insert_directory_node(&db, "node-1", "parent", "dir1", 10).await;

        let result = db
            .query(QueryInput::sql(directory_nodes_query()))
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_directory_node(&db, "node-1", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::sql(root_query()))
            .await
            .expect("root query should run");
        let QueryResult::Select(rows) = result else {
            panic!("root query should return select rows");
        };

        assert_eq!(row_ids(&rows), vec!["child-dir", "parent"]);
    }

    #[tokio::test]
    async fn root_query_ignores_non_directory_entities() {
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "dir1", "Dir1").await;
        insert_entity(&db, "person1", "semantic:base:person", "Person 1").await;

        let result = db
            .query(QueryInput::sql(root_query()))
            .await
            .expect("root query should run");
        let QueryResult::Select(rows) = result else {
            panic!("root query should return select rows");
        };

        assert_eq!(row_ids(&rows), vec!["dir1"]);
    }

    #[tokio::test]
    async fn child_query_finds_items_for_parent_directory() {
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_entity(&db, "child-item", "semantic:base:person", "Child Item").await;
        insert_directory_node(&db, "node-1", "parent", "child-item", 20).await;
        insert_directory_node(&db, "node-2", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::sql(child_query(
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_entity(&db, "child-item", "semantic:base:person", "Child Item").await;
        insert_directory_node(&db, "node-1", "parent", "child-item", 20).await;
        insert_directory_node(&db, "node-2", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::sql(child_directories_query("parent", 200, 0)))
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_directory_node(&db, "node-1", "parent", "child-dir", 10).await;

        let result = db
            .query(QueryInput::sql(parent_query("child-dir")))
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent-a", "Parent A").await;
        insert_directory(&db, "parent-b", "Parent B").await;
        insert_directory(&db, "child-dir", "Child Dir").await;
        insert_directory_node(&db, "node-high", "parent-a", "child-dir", 20).await;
        insert_directory_node(&db, "node-low", "parent-b", "child-dir", 10).await;

        let result = db
            .query(QueryInput::sql(parent_query("child-dir")))
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
    fn child_query_filters_from_parent() {
        let query = child_query("parent'1", DirectorySort::Order, 101, 0);
        assert!(query.contains("n.\"semantic:base:directory_node:from\" = 'parent''1'"));
        assert!(query.contains("n.\"semantic:base:directory_node:order\" ASC"));
    }

    #[test]
    fn child_links_query_escapes_parent_and_child_ids() {
        let query = child_links_query("parent'1", &["child'1".to_string(), "child2".to_string()]);
        assert!(query.contains("n.\"semantic:base:directory_node:from\" = 'parent''1'"));
        assert!(query.contains("n.\"semantic:relation:to\" IN ('child''1', 'child2')"));
    }

    #[test]
    fn entity_autocomplete_query_escapes_search_and_exclusions() {
        let excluded_ids = BTreeSet::from(["existing'1".to_string(), "existing2".to_string()]);
        let query = entity_autocomplete_query("Ada's", &excluded_ids);
        assert!(query.contains("ILIKE '%Ada''s%'"));
        assert!(query.contains("e.id ILIKE '%Ada''s%'"));
        assert!(query.contains("e.\"semantic:title\" ILIKE '%Ada''s%'"));
        assert!(query.contains("e.id NOT IN ('existing''1', 'existing2')"));
        assert!(query.contains(DIRECTORY_NODE_CLASS_ID));
        assert!(query.contains("LIMIT 50"));
    }

    #[test]
    fn addable_entities_query_excludes_parent_and_existing_children() {
        let query = addable_entities_query("parent'1", "needle", 25);
        assert!(query.contains("e.id != 'parent''1'"));
        assert!(query.contains("e.id NOT IN (SELECT"));
        assert!(query.contains("n.\"semantic:base:directory_node:from\" = 'parent''1'"));
        assert!(query.contains("LIMIT 25"));
    }

    #[test]
    fn parent_count_and_outgoing_queries_use_directory_relation_fields() {
        let parent_count = parent_count_query("child'1");
        assert!(parent_count.contains("COUNT(*) AS parent_count"));
        assert!(parent_count.contains("n.\"semantic:relation:to\" = 'child''1'"));
        assert!(parent_count.contains(DIRECTORY_NODE_RELATION_ID));

        let outgoing = directory_outgoing_links_query("dir'1");
        assert!(outgoing.contains("n.\"semantic:base:directory_node:from\" = 'dir''1'"));
        assert!(outgoing.contains("directory_to"));
    }

    #[tokio::test]
    async fn child_links_query_returns_matching_parent_child_links() {
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_entity(&db, "child-a", "semantic:base:person", "Child A").await;
        insert_entity(&db, "child-b", "semantic:base:person", "Child B").await;
        insert_directory_node(&db, "node-a", "parent", "child-a", 1).await;
        insert_directory_node(&db, "node-b", "parent", "child-b", 2).await;

        let result = db
            .query(QueryInput::sql(child_links_query(
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");
        insert_directory(&db, "parent", "Parent").await;
        insert_entity(&db, "existing", "semantic:base:person", "Needle Existing").await;
        insert_entity(&db, "candidate", "semantic:base:person", "Needle Candidate").await;
        insert_directory_node(&db, "node-existing", "parent", "existing", 1).await;

        let result = db
            .query(QueryInput::sql(addable_entities_query(
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
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
            .query(QueryInput::sql(addable_entities_query(
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
            .query(QueryInput::sql(addable_entities_query(
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
        assert!(title.contains("ORDER BY child.title DESC, child.id ASC"));
        let id = child_query("parent", DirectorySort::IdAsc, 10, 0);
        assert!(id.contains("ORDER BY child.id ASC"));
        let semantic = semantic_children_query("parent", DirectorySort::Order, 10, 0);
        assert!(semantic.contains("ORDER BY child.title ASC, child.id ASC"));
        let roots = semantic_parent_roots_query(DirectorySort::Order, 10, 0);
        assert!(roots.contains("parent.id IN (SELECT DISTINCT"));
        assert!(!roots.contains("EXISTS"));
    }

    #[tokio::test]
    async fn semantic_parent_queries_discover_and_page_direct_relationships() {
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
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
            .query(QueryInput::sql(semantic_parent_roots_query(
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
            .query(QueryInput::sql(semantic_parent_roots_query(
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
            .query(QueryInput::sql(semantic_children_query(
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
            .query(QueryInput::sql(semantic_parent_ids_for_candidates_query(
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
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
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
        assert!(!query.contains("EXISTS"));
        let QueryResult::Select(children) = db
            .query(QueryInput::sql(query))
            .await
            .expect("navigable semantic children query should run")
        else {
            panic!("navigable semantic children query should select rows");
        };
        assert_eq!(row_ids(&children), vec!["nested"]);
    }

    #[tokio::test]
    async fn semantic_parent_queries_handle_cycles_and_escaped_ids() {
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
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
            .query(QueryInput::sql(semantic_children_query(
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
            .query(QueryInput::sql(semantic_parent_query("cycle-a")))
            .await
            .expect("semantic parent query should run")
        else {
            panic!("semantic parent query should select rows");
        };
        assert_eq!(row_ids(&parent), vec!["cycle-b"]);

        let QueryResult::Select(quoted_children) = db
            .query(QueryInput::sql(semantic_children_query(
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
