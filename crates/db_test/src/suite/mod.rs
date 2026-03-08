use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::{
    CompareOp, Db, DeleteQuery, Expr, Operand, Predicate, QueryResult, SelectQuery,
    TextQueryFormat, UpdateQuery, catalog::CollectionKind,
};

pub async fn test_db(db: &Db) {
    test_schema_registration(db).await;
    test_select_query(db).await;
    test_sql_insert_query(db).await;
    test_update_query(db).await;
    test_delete_query(db).await;
    test_text_query_formats(db).await;
}

async fn test_schema_registration(db: &Db) {
    db.create_collection("shared_suite_schema", CollectionKind::Untyped)
        .await
        .expect("schema registration should succeed");

    let catalog = db
        .catalog()
        .await
        .expect("catalog fetch should succeed after schema registration");
    let collection = catalog
        .collection_by_name("shared_suite_schema")
        .expect("registered collection should be present in catalog");
    assert!(matches!(collection.kind, CollectionKind::Untyped));
}

async fn test_select_query(db: &Db) {
    db.create_collection("shared_suite_select", CollectionKind::Untyped)
        .await
        .expect("select test collection creation should succeed");

    db.insert("shared_suite_select", "evt-a", row("evt-a", "music", 10))
        .await
        .expect("first row insert should succeed");
    db.insert("shared_suite_select", "evt-b", row("evt-b", "video", 2))
        .await
        .expect("second row insert should succeed");

    let rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_select")
                .with_predicate(Predicate::Compare {
                    op: CompareOp::Eq,
                    left: Operand::Field(FieldPath::from_fields(["kind"])),
                    right: Operand::Literal(Value::String("music".to_string())),
                }),
        )
        .await
        .expect("select query should succeed");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("id"), Some(&Value::String("evt-a".to_string())));
    assert_eq!(rows[0].get("score"), Some(&Value::I64(10)));
}

async fn test_update_query(db: &Db) {
    db.create_collection("shared_suite_update", CollectionKind::Untyped)
        .await
        .expect("update test collection creation should succeed");
    db.insert("shared_suite_update", "item-1", row("item-1", "music", 1))
        .await
        .expect("update test row insert should succeed");

    let stats = db
        .update_where(
            UpdateQuery::new()
                .with_collection("shared_suite_update")
                .with_predicate(Predicate::Compare {
                    op: CompareOp::Eq,
                    left: Operand::Field(FieldPath::from_fields(["id"])),
                    right: Operand::Literal(Value::String("item-1".to_string())),
                })
                .set(
                    FieldPath::from_fields(["score"]),
                    Expr::Operand(Operand::Literal(Value::I64(99))),
                ),
        )
        .await
        .expect("update query should succeed");

    assert_eq!(stats.matched, 1);
    assert_eq!(stats.affected, 1);

    let row = db
        .get("shared_suite_update", "item-1")
        .await
        .expect("row fetch should succeed")
        .expect("row should still exist after update");
    assert_eq!(row.object.get("score"), Some(&Value::I64(99)));
}

async fn test_sql_insert_query(db: &Db) {
    if !db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        return;
    }

    db.create_collection("shared_suite_sql_insert", CollectionKind::Untyped)
        .await
        .expect("sql insert test collection creation should succeed");

    let _ = db
        .query_text(
            TextQueryFormat::Sql,
            "INSERT INTO shared_suite_sql_insert (id, kind, score) VALUES ('sql-ins-a', 'music', 4), ('sql-ins-b', 'video', 9)",
        )
        .await
        .expect("sql insert query should succeed");

    let result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id, kind, score FROM shared_suite_sql_insert ORDER BY id",
        )
        .await
        .expect("sql select query should succeed after insert");
    let QueryResult::Select(rows) = result else {
        panic!("sql select query should return SELECT rows");
    };

    assert_eq!(
        rows.len(),
        2,
        "sql insert should persist both inserted rows"
    );
    assert_eq!(
        rows[0].get("id"),
        Some(&Value::String("sql-ins-a".to_string()))
    );
    assert_eq!(
        rows[0].get("kind"),
        Some(&Value::String("music".to_string()))
    );
    assert_eq!(rows[0].get("score"), Some(&Value::I64(4)));
    assert_eq!(
        rows[1].get("id"),
        Some(&Value::String("sql-ins-b".to_string()))
    );
    assert_eq!(
        rows[1].get("kind"),
        Some(&Value::String("video".to_string()))
    );
    assert_eq!(rows[1].get("score"), Some(&Value::I64(9)));
}

async fn test_delete_query(db: &Db) {
    db.create_collection("shared_suite_delete", CollectionKind::Untyped)
        .await
        .expect("delete test collection creation should succeed");
    db.insert("shared_suite_delete", "gone-1", row("gone-1", "temp", 0))
        .await
        .expect("delete test row insert should succeed");
    db.insert("shared_suite_delete", "keep-1", row("keep-1", "stable", 0))
        .await
        .expect("delete control row insert should succeed");

    let deleted = db
        .delete_where(
            DeleteQuery::new()
                .with_collection("shared_suite_delete")
                .with_predicate(Predicate::Compare {
                    op: CompareOp::Eq,
                    left: Operand::Field(FieldPath::from_fields(["kind"])),
                    right: Operand::Literal(Value::String("temp".to_string())),
                }),
        )
        .await
        .expect("delete query should succeed");

    assert_eq!(deleted, 1);
    assert!(
        db.get("shared_suite_delete", "gone-1")
            .await
            .expect("deleted row lookup should succeed")
            .is_none()
    );
    assert!(
        db.get("shared_suite_delete", "keep-1")
            .await
            .expect("remaining row lookup should succeed")
            .is_some()
    );
}

async fn test_text_query_formats(db: &Db) {
    let catalog = db.catalog().await.expect("catalog fetch should succeed");
    assert!(
        catalog.collection_by_name("entities").is_some(),
        "default entities collection should be created during db initialization",
    );
    db.insert("entities", "fmt-a", row("fmt-a", "music", 7))
        .await
        .expect("text query row insert should succeed");
    db.insert("entities", "fmt-b", row("fmt-b", "video", 1))
        .await
        .expect("text query control row insert should succeed");

    for format in db.supported_text_query_formats() {
        let query = match format {
            TextQueryFormat::Sql => "SELECT id, score FROM entities WHERE kind = 'music'",
            TextQueryFormat::Prql => "filter kind == \"music\" | select {id, score}",
        };
        let result = db
            .query_text(format, query)
            .await
            .expect("text query execution should succeed");
        let QueryResult::Select(rows) = result else {
            panic!("text query should return SELECT rows");
        };
        assert_eq!(
            rows.len(),
            1,
            "text query format {format:?} should return exactly one row"
        );
        assert_eq!(
            rows[0].get("id"),
            Some(&Value::String("fmt-a".to_string())),
            "text query format {format:?} should return the matching row",
        );
    }
}

fn row(id: &str, kind: &str, score: i64) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(id.to_string()));
    row.insert("kind", Value::String(kind.to_string()));
    row.insert("score", Value::I64(score));
    row
}
