use semantic_data::bundles::directory::{
    DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
};

use super::types::DirectorySort;

pub(super) const ENTITIES_COLLECTION: &str = "entities";

pub(super) fn root_query(limit: usize, offset: usize) -> String {
    format!(
        "SELECT d.* FROM {entities} AS d WHERE d.type = {directory_class} AND d.id NOT IN (SELECT n.to FROM {node_collection} AS n WHERE n.relation = {node_relation}) ORDER BY d.title ASC, d.id ASC LIMIT {limit} OFFSET {offset}",
        entities = sql_ident(ENTITIES_COLLECTION),
        directory_class = sql_string(DIRECTORY_CLASS_ID),
        node_collection = sql_ident(DIRECTORY_NODE_CLASS_ID),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
    )
}

pub(super) fn child_query(
    parent_id: &str,
    sort: DirectorySort,
    limit: usize,
    offset: usize,
) -> String {
    format!(
        "SELECT child.*, n.order AS directory_order FROM {node_collection} AS n INNER JOIN {entities}._ AS child ON n.to = child.id WHERE n.relation = {node_relation} AND n.from = {parent_id} ORDER BY {order_by} LIMIT {limit} OFFSET {offset}",
        node_collection = sql_ident(DIRECTORY_NODE_CLASS_ID),
        entities = sql_ident(ENTITIES_COLLECTION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        parent_id = sql_string(parent_id),
        order_by = sort_order_by(sort),
    )
}

pub(super) fn child_directories_query(parent_id: &str, limit: usize, offset: usize) -> String {
    format!(
        "SELECT child.*, n.order AS directory_order FROM {node_collection} AS n INNER JOIN {entities}._ AS child ON n.to = child.id WHERE n.relation = {node_relation} AND n.from = {parent_id} AND child.type = {directory_class} ORDER BY n.order ASC, child.title ASC, child.id ASC LIMIT {limit} OFFSET {offset}",
        node_collection = sql_ident(DIRECTORY_NODE_CLASS_ID),
        entities = sql_ident(ENTITIES_COLLECTION),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        parent_id = sql_string(parent_id),
        directory_class = sql_string(DIRECTORY_CLASS_ID),
    )
}

pub(super) fn parent_query(child_id: &str) -> String {
    format!(
        "SELECT n.from FROM {node_collection} AS n WHERE n.relation = {node_relation} AND n.to = {child_id} ORDER BY n.order ASC, n.id ASC LIMIT 1",
        node_collection = sql_ident(DIRECTORY_NODE_CLASS_ID),
        node_relation = sql_string(DIRECTORY_NODE_RELATION_ID),
        child_id = sql_string(child_id),
    )
}

pub(super) fn sort_order_by(sort: DirectorySort) -> &'static str {
    match sort {
        DirectorySort::Order => "n.order ASC, child.title ASC, child.id ASC",
        DirectorySort::TitleAsc => "child.title ASC, child.id ASC",
        DirectorySort::TitleDesc => "child.title DESC, child.id ASC",
        DirectorySort::TypeAsc => "child.type ASC, child.title ASC, child.id ASC",
        DirectorySort::CreatedAtDesc => "child.created_at DESC, child.title ASC, child.id ASC",
        DirectorySort::UpdatedAtDesc => "child.updated_at DESC, child.title ASC, child.id ASC",
        DirectorySort::IdAsc => "child.id ASC",
    }
}

pub(super) fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn sql_ident(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use semantic_data::bundles::directory::{
        DIRECTORY_CLASS_ID, DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
    };
    use semantic_data::query::QueryInput;
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
    fn root_query_uses_directory_and_directory_node() {
        let query = root_query(101, 20);
        assert!(query.contains(DIRECTORY_CLASS_ID));
        assert!(query.contains(DIRECTORY_NODE_CLASS_ID));
        assert!(query.contains("NOT IN"));
        assert!(query.contains("LIMIT 101 OFFSET 20"));
    }

    #[tokio::test]
    async fn root_query_finds_parentless_directory_entity() {
        let db = Db::new(KvBackend::new(KvDb::in_memory()));
        db.upsert_package(semantic_base::package())
            .await
            .expect("base package should register");

        insert_directory(&db, "dir1", "Dir1").await;

        let result = db
            .query(QueryInput::sql(root_query(200, 0)))
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
        insert_node_collection_ref_target(&db, "child-item", "semantic:base:person", "Child Item")
            .await;
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
        insert_node_collection_ref_target(&db, "child-item", "semantic:base:person", "Child Item")
            .await;
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
            rows[0].get("from"),
            Some(&Value::String("parent".to_string()))
        );
    }

    #[test]
    fn child_query_filters_from_parent() {
        let query = child_query("parent'1", DirectorySort::Order, 101, 0);
        assert!(query.contains("n.from = 'parent''1'"));
        assert!(query.contains("n.order ASC"));
    }

    #[test]
    fn sort_options_map_to_stable_ordering() {
        assert_eq!(
            sort_order_by(DirectorySort::TitleDesc),
            "child.title DESC, child.id ASC"
        );
        assert_eq!(sort_order_by(DirectorySort::IdAsc), "child.id ASC");
    }

    async fn insert_directory(db: &Db, id: &str, title: &str) {
        insert_entity(db, id, DIRECTORY_CLASS_ID, title).await;
        insert_node_collection_ref_target(db, id, DIRECTORY_CLASS_ID, title).await;
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

    async fn insert_node_collection_ref_target(db: &Db, id: &str, ty: &str, title: &str) {
        let mut entity = Object::new();
        entity.insert("id", Value::String(id.to_string()));
        entity.insert("type", Value::String(ty.to_string()));
        entity.insert("title", Value::String(title.to_string()));
        db.insert(DIRECTORY_NODE_CLASS_ID, id, entity)
            .await
            .expect("directory node ref target should insert");
    }

    async fn insert_directory_node(db: &Db, id: &str, parent: &str, child: &str, order: u64) {
        let mut node = Object::new();
        node.insert("id", Value::String(id.to_string()));
        node.insert("type", Value::String(DIRECTORY_NODE_CLASS_ID.to_string()));
        node.insert(
            "relation",
            Value::String(DIRECTORY_NODE_RELATION_ID.to_string()),
        );
        node.insert("from", Value::String(parent.to_string()));
        node.insert("to", Value::String(child.to_string()));
        node.insert("order", Value::U64(order));
        db.insert(DIRECTORY_NODE_CLASS_ID, id, node)
            .await
            .expect("directory node should insert");
    }

    fn row_ids(rows: &[Object]) -> Vec<&str> {
        rows.iter()
            .map(|row| row.get("id").and_then(Value::as_str).expect("row id"))
            .collect()
    }
}
