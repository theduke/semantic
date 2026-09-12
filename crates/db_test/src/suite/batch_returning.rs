use super::*;
use semantic_db_core::{BatchReply, BatchReturn, BatchReturnErrorReason, EntityChangeKind};

pub(super) async fn test_batch_returning(db: &Db) {
    let collection = "shared_batch_returning";
    db.create_collection(collection, CollectionKind::Polymorphic)
        .await
        .unwrap();
    let upsert = |id: &str, label: &str| BatchOperation::Upsert {
        collection: collection.into(),
        id: id.into(),
        object: Object::from_iter([
            ("id".into(), Value::String(id.into())),
            ("returning_value".into(), Value::String(label.into())),
        ]),
    };
    let delete = |id: &str| BatchOperation::DeleteById {
        collection: collection.into(),
        id: id.into(),
    };
    db.execute_batch(Batch::new().with_op(upsert("old", "old")))
        .await
        .unwrap();
    let result = db
        .execute_batch_returning(
            Batch::new()
                .with_op(upsert("new", "first"))
                .with_op(upsert("new", "final"))
                .with_op(upsert("temporary", "temporary"))
                .with_op(delete("temporary"))
                .with_op(delete("old")),
            BatchReturn::Projection {
                fields: vec!["returning_value".into()],
            },
        )
        .await
        .unwrap();
    let BatchReply::Projection {
        rows,
        changes,
        stats,
    } = result
    else {
        panic!("projection reply")
    };
    assert_eq!(stats.upserted, 3);
    assert_eq!(stats.deleted, 2);
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].id, "new");
    assert_eq!(changes[1].kind, EntityChangeKind::Delete);
    assert_eq!(
        rows[0].object,
        Object::from_iter([("returning_value".into(), Value::String("final".into()))])
    );
    let error = db
        .execute_batch_returning(
            Batch::new().with_op(upsert("new", "invalid reply")),
            BatchReturn::Projection {
                fields: vec!["missing".into()],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        DbError::BatchReturn {
            reason: BatchReturnErrorReason::UnknownField,
            ..
        }
    ));
    assert_eq!(
        db.get(collection, "new")
            .await
            .unwrap()
            .unwrap()
            .object
            .get("returning_value"),
        Some(&Value::String("final".into()))
    );
    assert!(
        matches!(db.execute_batch_returning(Batch::new().with_op(upsert("new", "final")), BatchReturn::Changes).await.unwrap(), BatchReply::Changes { changes, .. } if changes.is_empty())
    );
    assert!(matches!(
        db.execute_batch_returning(Batch::new(), BatchReturn::Stats)
            .await
            .unwrap(),
        BatchReply::Stats { .. }
    ));
}
