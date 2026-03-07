use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::{
    CollectionKind, CompareOp, Database, Expr, Operand, Predicate, SelectQuery, UpdateQuery,
};
use semantic_db_redb::RedbBackend;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> std::result::Result<(), semantic_db_core::DbError> {
    let path = std::env::temp_dir().join(format!(
        "semantic-redb-example-{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos()
    ));

    let backend = RedbBackend::open(&path)?;
    let db = Database::new(backend);
    db.create_collection("users", CollectionKind::Untyped)
        .await?;

    let mut alice = Object::new();
    alice.insert("username", Value::String("alice".to_string()));
    alice.insert("role", Value::String("admin".to_string()));
    db.insert("users", "u1", alice).await?;

    let mut bob = Object::new();
    bob.insert("username", Value::String("bob".to_string()));
    bob.insert("role", Value::String("member".to_string()));
    db.insert("users", "u2", bob).await?;

    let admins = db
        .query(
            "users",
            SelectQuery::new().with_predicate(Predicate::Compare {
                op: CompareOp::Eq,
                left: Operand::Field(FieldPath::from_fields(["role"])),
                right: Operand::Literal(Value::String("admin".to_string())),
            }),
        )
        .await?;
    assert_eq!(admins.len(), 1);
    assert_eq!(
        admins[0].get("username"),
        Some(&Value::String("alice".to_string()))
    );

    db.update_where(
        "users",
        UpdateQuery::new()
            .with_predicate(Predicate::Compare {
                op: CompareOp::Eq,
                left: Operand::Field(FieldPath::from_fields(["username"])),
                right: Operand::Literal(Value::String("alice".to_string())),
            })
            .set(
                FieldPath::from_fields(["role"]),
                Expr::Operand(Operand::Literal(Value::String("owner".to_string()))),
            ),
    )
    .await?;

    let owners = db
        .query(
            "users",
            SelectQuery::new().with_predicate(Predicate::Compare {
                op: CompareOp::Eq,
                left: Operand::Field(FieldPath::from_fields(["role"])),
                right: Operand::Literal(Value::String("owner".to_string())),
            }),
        )
        .await?;
    assert_eq!(owners.len(), 1);
    assert_eq!(
        owners[0].get("username"),
        Some(&Value::String("alice".to_string()))
    );

    db.delete("users", "u2").await?;

    let bob_rows = db
        .query(
            "users",
            SelectQuery::new().with_predicate(Predicate::Compare {
                op: CompareOp::Eq,
                left: Operand::Field(FieldPath::from_fields(["username"])),
                right: Operand::Literal(Value::String("bob".to_string())),
            }),
        )
        .await?;
    assert!(bob_rows.is_empty());

    let _ = std::fs::remove_file(path);
    Ok(())
}
