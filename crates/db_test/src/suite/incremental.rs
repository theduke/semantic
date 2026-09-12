use super::*;

pub(super) async fn test_incremental_writes(db: &Db) {
    let collection = "shared_incremental";
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertCollection {
                name: collection.into(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            })
            .with_op(DdlOperation::UpsertIndex {
                name: "incremental_unique".into(),
                collection: collection.into(),
                field: "label".into(),
                unique: true,
            })
            .with_op(DdlOperation::UpsertRelationship {
                relationship: RelationType {
                    id: "shared_incremental_relation".into(),
                    name: "incremental".into(),
                    source_collection: collection.into(),
                    mode: RelationMode::External,
                    indexing_mode: RelationIndexingMode::Enabled,
                    meta: Meta::default(),
                },
            }),
    )
    .await
    .unwrap();
    let row = |id: &str, label: &str| {
        let mut object = Object::new();
        object.insert("id", Value::String(id.into()));
        object.insert("label", Value::String(label.into()));
        object.insert("from", Value::String("incremental-source".into()));
        object.insert("to", Value::String("incremental-target".into()));
        object
    };
    let upsert = |id: &str, label: &str| BatchOperation::Upsert {
        collection: collection.into(),
        id: id.into(),
        object: row(id, label),
    };
    db.execute_batch(
        Batch::new()
            .with_op(upsert("one", "first"))
            .with_op(upsert("two", "second")),
    )
    .await
    .unwrap();
    // Final-state uniqueness permits a swap; physical storage must remove old
    // index entries without erasing either surviving row or its contributions.
    db.execute_batch(
        Batch::new()
            .with_op(upsert("one", "second"))
            .with_op(upsert("two", "first")),
    )
    .await
    .unwrap();
    let labels = db
        .select(
            SelectQuery::new()
                .with_collection(collection)
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["label"]),
                    Value::String("first".into()),
                )),
        )
        .await
        .unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].get("id"), Some(&Value::String("two".into())));
    // An invalid final unique key must fail atomically.
    assert!(
        db.execute_batch(Batch::new().with_op(upsert("one", "first")))
            .await
            .is_err()
    );
    let delete = |id: &str| BatchOperation::DeleteById {
        collection: collection.into(),
        id: id.into(),
    };
    db.execute_batch(Batch::new().with_op(delete("one")))
        .await
        .unwrap();
    let edge_query = || SelectQuery::new().with_collection("__semantic.relationship_edges");
    let count_edges = |rows: Vec<Object>| {
        rows.iter()
            .filter(|row| {
                row.get("relation").and_then(Value::as_str) == Some("shared_incremental_relation")
            })
            .count()
    };
    assert_eq!(
        count_edges(db.select(edge_query()).await.unwrap()),
        1,
        "a duplicate source record still contributes the direct edge"
    );
    db.execute_batch(Batch::new().with_op(delete("two")))
        .await
        .unwrap();
    assert_eq!(count_edges(db.select(edge_query()).await.unwrap()), 0);
}
