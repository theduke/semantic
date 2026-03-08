use semantic_data::value::{FieldPath, Object, Value};

use crate::{CompareOp, Db, Operand, Predicate, SelectQuery, UpdateQuery, catalog::CollectionKind};

pub async fn test_db(db: &Db) {
    test_schema_registration(db).await;
    test_select_query(db).await;
    test_update_query(db).await;
    test_delete_query(db).await;
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
                    crate::Expr::Operand(Operand::Literal(Value::I64(99))),
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
            crate::DeleteQuery::new()
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

fn row(id: &str, kind: &str, score: i64) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(id.to_string()));
    row.insert("kind", Value::String(kind.to_string()));
    row.insert("score", Value::I64(score));
    row
}
