use super::*;
use semantic_db_core::{BatchReply, BatchReturn};

pub(super) async fn test_compact_id_execution(db: &Db) {
    let collection = "shared_compact";
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertCollection {
                name: collection.into(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            })
            .with_op(DdlOperation::UpsertIndex {
                name: "compact_label_unique".into(),
                collection: collection.into(),
                field: "compact_label".into(),
                unique: true,
            })
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "test:compact_owner".into(),
                    name: "compact_owner".into(),
                    ty: Type::new(TypeKind::Ref(TypeRef::new("test:compact_person"))),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: "test:compact_person".into(),
                    name: "CompactPerson".into(),
                    inherits: None,
                    extends: vec![],
                    strict_schema: false,
                    creatable_in_ui: None,
                    attributes: BTreeMap::from([(
                        "compact_owner".into(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "test:compact_owner".into(),
                            },
                            required: false,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    )]),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            }),
    )
    .await
    .unwrap();
    let upsert = |id: &str, label: &str, owner: Option<&str>| {
        let mut object = Object::new();
        object.insert("id", id.to_string());
        object.insert("compact_label", label.to_string());
        object.insert("type", "test:compact_person".to_string());
        if let Some(owner) = owner {
            object.insert("test:compact_owner", owner.to_string());
        }
        BatchOperation::Upsert {
            collection: collection.into(),
            id: id.into(),
            object,
        }
    };
    let delete = |id: &str| BatchOperation::DeleteById {
        collection: collection.into(),
        id: id.into(),
    };
    // A forward target and a final-state unique-key swap must both work.
    db.execute_batch_returning(
        Batch::new()
            .with_op(upsert("dependent", "first", Some("target")))
            .with_op(upsert("target", "second", None)),
        BatchReturn::Stats,
    )
    .await
    .unwrap();
    let reply = db
        .execute_batch_returning(
            Batch::new()
                .with_op(upsert("dependent", "second", Some("target")))
                .with_op(upsert("target", "first", None)),
            BatchReturn::Changes,
        )
        .await
        .unwrap();
    assert!(matches!(reply, BatchReply::Changes { changes, .. } if changes.len() == 2));
    assert!(
        db.execute_batch_returning(Batch::new().with_op(delete("target")), BatchReturn::Stats)
            .await
            .is_err()
    );
    assert!(db.get(collection, "target").await.unwrap().is_some());
    assert!(
        db.execute_batch_returning(
            Batch::new().with_op(upsert("dependent", "first", Some("target"))),
            BatchReturn::Stats
        )
        .await
        .is_err()
    );
    let reply = db
        .execute_batch_returning(
            Batch::new()
                .with_op(delete("target"))
                .with_op(delete("dependent")),
            BatchReturn::Changes,
        )
        .await
        .unwrap();
    assert!(
        matches!(reply, BatchReply::Changes { stats, changes } if stats.deleted == 2 && changes.len() == 2)
    );
}
