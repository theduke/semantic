use super::*;
use semantic_data::schema::{Constraint, ForeignKeyRef};
use semantic_db_core::BatchReturn;

/// Run on a fresh managed database: activation deliberately applies to the database.
pub async fn test_validation(db: &Db) {
    let class = |id: &str, attributes| ClassType {
        id: id.into(),
        name: id.into(),
        inherits: None,
        extends: vec![],
        strict_schema: false,
        creatable_in_ui: None,
        attributes,
        constraints: vec![],
        meta: Meta::default(),
    };
    let attr = AttributeType {
        id: "validation:target".into(),
        name: "Target".into(),
        ty: Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        })),
        constraints: vec![Constraint::ForeignKey(ForeignKeyRef {
            to: TypeRef::new("validation:Person"),
            fields: vec!["id".into()],
        })],
        meta: Meta::default(),
    };
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute { attribute: attr })
            .with_op(DdlOperation::UpsertClass {
                class: class(
                    "validation:Holder",
                    BTreeMap::from([(
                        "target".into(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "validation:target".into(),
                            },
                            required: true,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    )]),
                ),
            })
            .with_op(DdlOperation::UpsertClass {
                class: class("validation:Person", BTreeMap::new()),
            }),
    )
    .await
    .unwrap();
    let row = |id: &str, class: &str, target: Option<&str>| {
        let mut object = Object::new();
        object.insert("id", id.to_string());
        object.insert("type", class.to_string());
        if let Some(target) = target {
            object.insert("validation:target", target.to_string());
        }
        BatchOperation::Upsert {
            collection: semantic_db_core::DEFAULT_COLLECTION.into(),
            id: id.into(),
            object,
        }
    };
    db.execute_batch(Batch::new().with_op(row("owner", "validation:Holder", None)))
        .await
        .unwrap();
    let report = db.validation_preflight().await.unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].error.rule, "required");
    assert!(db.activate_validation().await.is_err());
    db.execute_batch(
        Batch::new()
            .with_op(row("owner", "validation:Holder", Some("target")))
            .with_op(row("target", "validation:Person", None)),
    )
    .await
    .unwrap();
    assert!(db.validation_preflight().await.unwrap().is_empty());
    db.activate_validation().await.unwrap();
    for mode in [BatchReturn::Stats, BatchReturn::Dataset] {
        let err = db
            .execute_batch_returning(
                Batch::new().with_op(BatchOperation::DeleteById {
                    collection: semantic_db_core::DEFAULT_COLLECTION.into(),
                    id: "target".into(),
                }),
                mode.clone(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DbError::Validation(ref e) if e.rule == "reference"),
            "{err:?}"
        );
        let err = db
            .execute_batch_returning(
                Batch::new().with_op(row("invalid", "validation:Holder", Some("missing"))),
                mode,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, DbError::Validation(ref e) if e.attribute == "validation:target"),
            "{err:?}"
        );
        assert!(db.get(None::<&str>, "invalid").await.unwrap().is_none());
    }
}
