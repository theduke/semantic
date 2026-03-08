use semantic_data::query::{
    BinaryOp, CompareOp, DeleteQuery, Expr, FunctionArg, Operand, PatternMatchKind, Predicate,
    SelectQuery, TextQueryFormat, UpdateQuery,
};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::{Db, QueryResult, catalog::CollectionKind};

pub async fn test_db(db: &Db) {
    test_schema_registration(db).await;
    test_select_query(db).await;
    test_sql_insert_query(db).await;
    test_update_query(db).await;
    test_delete_query(db).await;
    test_ast_predicate_constructs(db).await;
    test_sql_predicate_constructs(db).await;
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

async fn test_ast_predicate_constructs(db: &Db) {
    db.create_collection("shared_suite_ast_predicates", CollectionKind::Untyped)
        .await
        .expect("ast predicate test collection creation should succeed");

    let mut a = row("ast-a", "Music", 10);
    a.insert("note", Value::String("hello world".to_string()));
    a.insert("opt", Value::Null);
    db.insert("shared_suite_ast_predicates", "ast-a", a)
        .await
        .expect("first ast row insert should succeed");

    let mut b = row("ast-b", "video", 3);
    b.insert("note", Value::String("zzz".to_string()));
    b.insert("opt", Value::String("value".to_string()));
    db.insert("shared_suite_ast_predicates", "ast-b", b)
        .await
        .expect("second ast row insert should succeed");

    let in_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::InList {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    list: vec![
                        Expr::Operand(Operand::Literal(Value::String("Music".to_string()))),
                        Expr::Operand(Operand::Literal(Value::String("podcast".to_string()))),
                    ],
                    negated: false,
                })),
        )
        .await
        .expect("ast IN query should succeed");
    assert_eq!(in_rows.len(), 1);
    assert_eq!(
        in_rows[0].get("id"),
        Some(&Value::String("ast-a".to_string()))
    );

    let not_in_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::InList {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    list: vec![Expr::Operand(Operand::Literal(Value::String(
                        "Music".to_string(),
                    )))],
                    negated: true,
                })),
        )
        .await
        .expect("ast NOT IN query should succeed");
    assert_eq!(not_in_rows.len(), 1);
    assert_eq!(
        not_in_rows[0].get("id"),
        Some(&Value::String("ast-b".to_string()))
    );

    let like_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::PatternMatch {
                    kind: PatternMatchKind::Like,
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "note",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "hello%".to_string(),
                    )))),
                    case_insensitive: false,
                    negated: false,
                })),
        )
        .await
        .expect("ast LIKE query should succeed");
    assert_eq!(like_rows.len(), 1);

    let ilike_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::PatternMatch {
                    kind: PatternMatchKind::Like,
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "music".to_string(),
                    )))),
                    case_insensitive: true,
                    negated: false,
                })),
        )
        .await
        .expect("ast ILIKE query should succeed");
    assert_eq!(ilike_rows.len(), 1);

    let similar_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::PatternMatch {
                    kind: PatternMatchKind::SimilarTo,
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "note",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "hello%".to_string(),
                    )))),
                    case_insensitive: false,
                    negated: false,
                })),
        )
        .await
        .expect("ast SIMILAR TO query should succeed");
    assert_eq!(similar_rows.len(), 1);

    let regex_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::RegexMatch {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "note",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "^hello".to_string(),
                    )))),
                    case_insensitive: false,
                    negated: false,
                })),
        )
        .await
        .expect("ast regex query should succeed");
    assert_eq!(regex_rows.len(), 1);

    let between_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::Between {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "score",
                    ])))),
                    low: Box::new(Expr::Operand(Operand::Literal(Value::I64(5)))),
                    high: Box::new(Expr::Operand(Operand::Literal(Value::I64(12)))),
                    negated: false,
                })),
        )
        .await
        .expect("ast BETWEEN query should succeed");
    assert_eq!(between_rows.len(), 1);

    let null_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::IsNull {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "opt",
                    ])))),
                    negated: false,
                })),
        )
        .await
        .expect("ast IS NULL query should succeed");
    assert_eq!(null_rows.len(), 1);

    let not_null_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::IsNull {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "opt",
                    ])))),
                    negated: true,
                })),
        )
        .await
        .expect("ast IS NOT NULL query should succeed");
    assert_eq!(not_null_rows.len(), 1);

    let exists_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::Exists {
                    query: Box::new(
                        SelectQuery::new()
                            .with_collection("shared_suite_ast_predicates")
                            .with_predicate(Predicate::Compare {
                                op: CompareOp::Eq,
                                left: Operand::Field(FieldPath::from_fields(["id"])),
                                right: Operand::Literal(Value::String("ast-a".to_string())),
                            }),
                    ),
                    negated: false,
                })),
        )
        .await
        .expect("ast EXISTS query should succeed");
    assert_eq!(exists_rows.len(), 2);

    let func_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::Expr(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Function {
                        name: "LOWER".to_string(),
                        args: vec![FunctionArg::Expr(Expr::Operand(Operand::Field(
                            FieldPath::from_fields(["kind"]),
                        )))],
                    }),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "music".to_string(),
                    )))),
                })),
        )
        .await
        .expect("ast LOWER function query should succeed");
    assert_eq!(func_rows.len(), 1);

    let aggregate_like_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Predicate::And(vec![
                    Predicate::Compare {
                        op: CompareOp::Eq,
                        left: Operand::Field(FieldPath::from_fields(["id"])),
                        right: Operand::Literal(Value::String("ast-a".to_string())),
                    },
                    Predicate::Expr(Expr::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(Expr::Function {
                            name: "SUM".to_string(),
                            args: vec![FunctionArg::Expr(Expr::Operand(Operand::Field(
                                FieldPath::from_fields(["score"]),
                            )))],
                        }),
                        right: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "score",
                        ])))),
                    }),
                    Predicate::Expr(Expr::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(Expr::Function {
                            name: "AVG".to_string(),
                            args: vec![FunctionArg::Expr(Expr::Operand(Operand::Field(
                                FieldPath::from_fields(["score"]),
                            )))],
                        }),
                        right: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "score",
                        ])))),
                    }),
                    Predicate::Expr(Expr::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(Expr::Function {
                            name: "MIN".to_string(),
                            args: vec![FunctionArg::Expr(Expr::Operand(Operand::Field(
                                FieldPath::from_fields(["score"]),
                            )))],
                        }),
                        right: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "score",
                        ])))),
                    }),
                    Predicate::Expr(Expr::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(Expr::Function {
                            name: "MAX".to_string(),
                            args: vec![FunctionArg::Expr(Expr::Operand(Operand::Field(
                                FieldPath::from_fields(["score"]),
                            )))],
                        }),
                        right: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "score",
                        ])))),
                    }),
                ])),
        )
        .await
        .expect("ast SUM/AVG/MIN/MAX function query should succeed");
    assert_eq!(aggregate_like_rows.len(), 1);
}

async fn test_sql_predicate_constructs(db: &Db) {
    if !db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        return;
    }

    db.create_collection("shared_suite_sql_predicates", CollectionKind::Untyped)
        .await
        .expect("sql predicate test collection creation should succeed");

    let mut a = row("sql-a", "Music", 10);
    a.insert("note", Value::String("hello world".to_string()));
    a.insert("opt", Value::Null);
    db.insert("shared_suite_sql_predicates", "sql-a", a)
        .await
        .expect("first sql predicate row insert should succeed");

    let mut b = row("sql-b", "video", 3);
    b.insert("note", Value::String("zzz".to_string()));
    b.insert("opt", Value::String("value".to_string()));
    db.insert("shared_suite_sql_predicates", "sql-b", b)
        .await
        .expect("second sql predicate row insert should succeed");

    let in_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE kind IN ('Music', 'podcast')",
        )
        .await
        .expect("sql IN query should succeed");
    let QueryResult::Select(in_rows) = in_result else {
        panic!("sql IN query should return SELECT rows");
    };
    assert_eq!(in_rows.len(), 1);

    let not_in_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE kind NOT IN ('Music')",
        )
        .await
        .expect("sql NOT IN query should succeed");
    let QueryResult::Select(not_in_rows) = not_in_result else {
        panic!("sql NOT IN query should return SELECT rows");
    };
    assert_eq!(not_in_rows.len(), 1);

    let like_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE note LIKE 'hello%'",
        )
        .await
        .expect("sql LIKE query should succeed");
    let QueryResult::Select(like_rows) = like_result else {
        panic!("sql LIKE query should return SELECT rows");
    };
    assert_eq!(like_rows.len(), 1);

    let ilike_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE kind ILIKE 'music'",
        )
        .await
        .expect("sql ILIKE query should succeed");
    let QueryResult::Select(ilike_rows) = ilike_result else {
        panic!("sql ILIKE query should return SELECT rows");
    };
    assert_eq!(ilike_rows.len(), 1);

    let similar_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE note SIMILAR TO 'hello%'",
        )
        .await
        .expect("sql SIMILAR TO query should succeed");
    let QueryResult::Select(similar_rows) = similar_result else {
        panic!("sql SIMILAR TO query should return SELECT rows");
    };
    assert_eq!(similar_rows.len(), 1);

    let regex_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE note ~ '^hello'",
        )
        .await
        .expect("sql regex query should succeed");
    let QueryResult::Select(regex_rows) = regex_result else {
        panic!("sql regex query should return SELECT rows");
    };
    assert_eq!(regex_rows.len(), 1);

    let regex_i_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE kind ~* '^music$'",
        )
        .await
        .expect("sql case-insensitive regex query should succeed");
    let QueryResult::Select(regex_i_rows) = regex_i_result else {
        panic!("sql case-insensitive regex query should return SELECT rows");
    };
    assert_eq!(regex_i_rows.len(), 1);

    let between_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE score BETWEEN 5 AND 12",
        )
        .await
        .expect("sql BETWEEN query should succeed");
    let QueryResult::Select(between_rows) = between_result else {
        panic!("sql BETWEEN query should return SELECT rows");
    };
    assert_eq!(between_rows.len(), 1);

    let null_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE opt IS NULL",
        )
        .await
        .expect("sql IS NULL query should succeed");
    let QueryResult::Select(null_rows) = null_result else {
        panic!("sql IS NULL query should return SELECT rows");
    };
    assert_eq!(null_rows.len(), 1);

    let not_null_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE opt IS NOT NULL",
        )
        .await
        .expect("sql IS NOT NULL query should succeed");
    let QueryResult::Select(not_null_rows) = not_null_result else {
        panic!("sql IS NOT NULL query should return SELECT rows");
    };
    assert_eq!(not_null_rows.len(), 1);

    let exists_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE EXISTS (SELECT id FROM shared_suite_sql_predicates WHERE id = 'sql-a')",
        )
        .await
        .expect("sql EXISTS query should succeed");
    let QueryResult::Select(exists_rows) = exists_result else {
        panic!("sql EXISTS query should return SELECT rows");
    };
    assert_eq!(exists_rows.len(), 2);

    let function_result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE id = 'sql-a' AND LOWER(kind) = 'music' AND UPPER(kind) = 'MUSIC' AND COUNT(*) = 1 AND SUM(score) = score AND AVG(score) = score AND MIN(score) = score AND MAX(score) = score",
        )
        .await
        .expect("sql function query should succeed");
    let QueryResult::Select(function_rows) = function_result else {
        panic!("sql function query should return SELECT rows");
    };
    assert_eq!(function_rows.len(), 1);
}

fn row(id: &str, kind: &str, score: i64) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(id.to_string()));
    row.insert("kind", Value::String(kind.to_string()));
    row.insert("score", Value::I64(score));
    row
}
