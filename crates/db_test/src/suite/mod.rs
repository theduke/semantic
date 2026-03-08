use std::future::Future;
use std::pin::Pin;

use semantic_data::query::{
    AggregateOp, BinaryOp, CompareOp, DeleteQuery, Expr, FunctionArg, JoinCondition, JoinQuery,
    JoinSource, JoinType, Operand, OrderBy, PatternMatchKind, Predicate, QueryField, QueryInput,
    SelectQuery, SortDirection, TextQueryFormat, UpdateQuery,
};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::{Db, DbError, QueryResult, catalog::CollectionKind};

trait DbTextQueryExt {
    fn query_text<'a>(
        &'a self,
        format: TextQueryFormat,
        query: impl Into<String>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<QueryResult, DbError>> + 'a>>;
}

impl DbTextQueryExt for Db {
    fn query_text<'a>(
        &'a self,
        format: TextQueryFormat,
        query: impl Into<String>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<QueryResult, DbError>> + 'a>> {
        let query = query.into();
        Box::pin(async move { self.query(QueryInput::Text { format, query }).await })
    }
}

pub async fn test_db(db: &Db) {
    test_schema_registration(db).await;
    test_select_query(db).await;
    test_sql_insert_query(db).await;
    test_update_query(db).await;
    test_delete_query(db).await;
    test_ast_predicate_constructs(db).await;
    test_sql_predicate_constructs(db).await;
    test_join_semantics(db).await;
    test_ast_ordering_variants(db).await;
    test_sql_ordering_variants(db).await;
    test_ast_limit_offset_variants(db).await;
    test_sql_limit_offset_variants(db).await;
    test_ast_aggregation_distinct_grouping(db).await;
    test_sql_aggregation_distinct_grouping(db).await;
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
            "SELECT id FROM shared_suite_sql_predicates WHERE id = 'sql-a' AND LOWER(kind) = 'music' AND UPPER(kind) = 'MUSIC'",
        )
        .await
        .expect("sql function query should succeed");
    let QueryResult::Select(function_rows) = function_result else {
        panic!("sql function query should return SELECT rows");
    };
    assert_eq!(function_rows.len(), 1);
}

async fn test_join_semantics(db: &Db) {
    db.create_collection("shared_suite_join_items", CollectionKind::Untyped)
        .await
        .expect("join source collection creation should succeed");
    db.create_collection("shared_suite_join_profiles", CollectionKind::Untyped)
        .await
        .expect("join profile collection creation should succeed");

    let mut artist = Object::new();
    artist.insert("id", Value::String("artist-1".to_string()));
    artist.insert("type", Value::String("Artist".to_string()));
    artist.insert("name", Value::String("Mia".to_string()));
    db.insert("shared_suite_join_items", "artist-1", artist)
        .await
        .expect("artist insert should succeed");

    let mut song = Object::new();
    song.insert("id", Value::String("song-1".to_string()));
    song.insert("type", Value::String("Song".to_string()));
    song.insert("artist_id", Value::String("artist-1".to_string()));
    db.insert("shared_suite_join_items", "song-1", song)
        .await
        .expect("song insert should succeed");

    let mut profile = Object::new();
    profile.insert("id", Value::String("artist-1".to_string()));
    profile.insert("kind", Value::String("featured".to_string()));
    db.insert("shared_suite_join_profiles", "artist-1", profile)
        .await
        .expect("profile insert should succeed");

    if db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        let same_collection_all = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT s.id AS sid, a.name AS artist_name FROM shared_suite_join_items AS s INNER JOIN _ AS a ON s.artist_id = a.id WHERE s.type = 'Song' AND a.type = 'Artist'",
            )
            .await
            .expect("sql same-collection join should succeed");
        let QueryResult::Select(same_collection_rows) = same_collection_all else {
            panic!("sql same-collection join should return SELECT rows");
        };
        assert_eq!(same_collection_rows.len(), 1);
        assert_eq!(
            same_collection_rows[0].get("artist_name"),
            Some(&Value::String("Mia".to_string()))
        );

        let class_join = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT s.id AS sid, a.name AS artist_name FROM shared_suite_join_items AS s INNER JOIN Artist AS a ON s.artist_id = a.id WHERE s.type = 'Song'",
            )
            .await
            .expect("sql class join should succeed");
        let QueryResult::Select(class_rows) = class_join else {
            panic!("sql class join should return SELECT rows");
        };
        assert_eq!(class_rows.len(), 1);
        assert_eq!(
            class_rows[0].get("artist_name"),
            Some(&Value::String("Mia".to_string()))
        );

        let cross_collection_join = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT s.id AS sid, p.kind AS profile_kind FROM shared_suite_join_items AS s INNER JOIN shared_suite_join_profiles._ AS p ON s.artist_id = p.id WHERE s.type = 'Song'",
            )
            .await
            .expect("sql cross-collection join should succeed");
        let QueryResult::Select(cross_rows) = cross_collection_join else {
            panic!("sql cross-collection join should return SELECT rows");
        };
        assert_eq!(cross_rows.len(), 1);
        assert_eq!(
            cross_rows[0].get("profile_kind"),
            Some(&Value::String("featured".to_string()))
        );

        let custom_join_predicate = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT s.id AS sid FROM shared_suite_join_items AS s INNER JOIN shared_suite_join_profiles._ AS p ON s.artist_id = p.id AND p.kind = 'featured' WHERE s.type = 'Song'",
            )
            .await
            .expect("sql join with custom ON predicate should succeed");
        let QueryResult::Select(custom_rows) = custom_join_predicate else {
            panic!("sql join with custom ON predicate should return SELECT rows");
        };
        assert_eq!(row_strings(&custom_rows, "sid"), vec!["song-1".to_string()]);
    }

    let ast_join_where = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_join_items")
                .with_source_alias("s")
                .with_predicate(Predicate::Compare {
                    op: CompareOp::Eq,
                    left: Operand::Field(FieldPath::from_fields(["s", "type"])),
                    right: Operand::Literal(Value::String("Song".to_string())),
                })
                .with_joins(vec![JoinQuery {
                    source: JoinSource {
                        collection: Some("shared_suite_join_profiles".to_string()),
                        class: None,
                    },
                    alias: Some("p".to_string()),
                    join_type: JoinType::Inner,
                    condition: JoinCondition::OnPredicate(Predicate::Compare {
                        op: CompareOp::Eq,
                        left: Operand::Field(FieldPath::from_fields(["s", "artist_id"])),
                        right: Operand::Field(FieldPath::from_fields(["p", "id"])),
                    }),
                    predicate: Some(Predicate::Compare {
                        op: CompareOp::Eq,
                        left: Operand::Field(FieldPath::from_fields(["kind"])),
                        right: Operand::Literal(Value::String("featured".to_string())),
                    }),
                }])
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "s", "id",
                    ])))),
                    alias: Some("sid".to_string()),
                }]),
        )
        .await
        .expect("ast join with join-local predicate should succeed");
    assert_eq!(
        row_strings(&ast_join_where, "sid"),
        vec!["song-1".to_string()]
    );
}

async fn test_ast_aggregation_distinct_grouping(db: &Db) {
    db.create_collection("shared_suite_ast_agg", CollectionKind::Untyped)
        .await
        .expect("ast aggregation test collection creation should succeed");
    for (id, kind, score) in [
        ("agg-a", "music", 10),
        ("agg-b", "music", 20),
        ("agg-c", "video", 5),
        ("agg-d", "video", 5),
    ] {
        db.insert("shared_suite_ast_agg", id, row(id, kind, score))
            .await
            .expect("ast aggregation seed insert should succeed");
    }

    let grouped = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_agg")
                .with_group_by(vec![Expr::Operand(Operand::Field(FieldPath::from_fields(
                    ["kind"],
                )))])
                .with_projection(vec![
                    QueryField {
                        expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "kind",
                        ])))),
                        alias: Some("kind".to_string()),
                    },
                    QueryField {
                        expr: Box::new(Expr::Aggregate {
                            op: AggregateOp::Count,
                            distinct: false,
                            arg: Box::new(FunctionArg::Wildcard),
                        }),
                        alias: Some("n".to_string()),
                    },
                    QueryField {
                        expr: Box::new(Expr::Aggregate {
                            op: AggregateOp::Sum,
                            distinct: false,
                            arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                                FieldPath::from_fields(["score"]),
                            )))),
                        }),
                        alias: Some("total".to_string()),
                    },
                ])
                .with_having(Predicate::Expr(Expr::Binary {
                    op: BinaryOp::Gt,
                    left: Box::new(Expr::Aggregate {
                        op: AggregateOp::Sum,
                        distinct: false,
                        arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                            FieldPath::from_fields(["score"]),
                        )))),
                    }),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::F64(10.0.into())))),
                })),
        )
        .await
        .expect("ast grouped aggregate query should succeed");
    assert_eq!(grouped.len(), 1);
    assert_eq!(
        grouped[0].get("kind"),
        Some(&Value::String("music".to_string()))
    );
    assert_eq!(grouped[0].get("n"), Some(&Value::I64(2)));
    assert_eq!(grouped[0].get("total"), Some(&Value::F64(30.0.into())));

    let distinct_kinds = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_agg")
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    alias: Some("kind".to_string()),
                }])
                .with_distinct(true)
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["kind"]))),
                    direction: SortDirection::Asc,
                }]),
        )
        .await
        .expect("ast distinct query should succeed");
    assert_eq!(
        row_strings(&distinct_kinds, "kind"),
        vec!["music".to_string(), "video".to_string()]
    );

    let global = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_agg")
                .with_projection(vec![
                    QueryField {
                        expr: Box::new(Expr::Aggregate {
                            op: AggregateOp::Count,
                            distinct: true,
                            arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                                FieldPath::from_fields(["kind"]),
                            )))),
                        }),
                        alias: Some("kinds".to_string()),
                    },
                    QueryField {
                        expr: Box::new(Expr::Aggregate {
                            op: AggregateOp::Sum,
                            distinct: true,
                            arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                                FieldPath::from_fields(["score"]),
                            )))),
                        }),
                        alias: Some("uniq_total".to_string()),
                    },
                ]),
        )
        .await
        .expect("ast global aggregate query should succeed");
    assert_eq!(global.len(), 1);
    assert_eq!(global[0].get("kinds"), Some(&Value::I64(2)));
    assert_eq!(global[0].get("uniq_total"), Some(&Value::F64(35.0.into())));
}

async fn test_sql_aggregation_distinct_grouping(db: &Db) {
    if !db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        return;
    }
    db.create_collection("shared_suite_sql_agg", CollectionKind::Untyped)
        .await
        .expect("sql aggregation test collection creation should succeed");
    for (id, kind, score) in [
        ("sql-agg-a", "music", 10),
        ("sql-agg-b", "music", 20),
        ("sql-agg-c", "video", 5),
        ("sql-agg-d", "video", 5),
    ] {
        db.insert("shared_suite_sql_agg", id, row(id, kind, score))
            .await
            .expect("sql aggregation seed insert should succeed");
    }

    let grouped = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT kind, COUNT(*) AS n, SUM(score) AS total, AVG(score) AS avg, MIN(score) AS mn, MAX(score) AS mx FROM shared_suite_sql_agg GROUP BY kind HAVING SUM(score) > 10.0 ORDER BY total DESC",
        )
        .await
        .expect("sql grouped aggregate query should succeed");
    let QueryResult::Select(grouped_rows) = grouped else {
        panic!("sql grouped aggregate query should return SELECT rows");
    };
    assert_eq!(grouped_rows.len(), 1);
    assert_eq!(
        grouped_rows[0].get("kind"),
        Some(&Value::String("music".to_string()))
    );
    assert_eq!(grouped_rows[0].get("n"), Some(&Value::I64(2)));
    assert_eq!(grouped_rows[0].get("total"), Some(&Value::F64(30.0.into())));
    assert_eq!(grouped_rows[0].get("avg"), Some(&Value::F64(15.0.into())));
    assert_eq!(grouped_rows[0].get("mn"), Some(&Value::I64(10)));
    assert_eq!(grouped_rows[0].get("mx"), Some(&Value::I64(20)));

    let distinct = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT DISTINCT kind FROM shared_suite_sql_agg ORDER BY kind ASC",
        )
        .await
        .expect("sql distinct query should succeed");
    let QueryResult::Select(distinct_rows) = distinct else {
        panic!("sql distinct query should return SELECT rows");
    };
    assert_eq!(
        row_strings(&distinct_rows, "kind"),
        vec!["music".to_string(), "video".to_string()]
    );

    let global = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT COUNT(DISTINCT kind) AS kinds, SUM(DISTINCT score) AS uniq_total FROM shared_suite_sql_agg",
        )
        .await
        .expect("sql global aggregate query should succeed");
    let QueryResult::Select(global_rows) = global else {
        panic!("sql global aggregate query should return SELECT rows");
    };
    assert_eq!(global_rows.len(), 1);
    assert_eq!(global_rows[0].get("kinds"), Some(&Value::I64(2)));
    assert_eq!(
        global_rows[0].get("uniq_total"),
        Some(&Value::F64(35.0.into()))
    );
}

async fn test_ast_ordering_variants(db: &Db) {
    db.create_collection("shared_suite_ordering_ast", CollectionKind::Untyped)
        .await
        .expect("ast ordering test collection creation should succeed");

    for (id, kind, score) in [
        ("ord-a", "music", 7),
        ("ord-b", "video", 2),
        ("ord-c", "music", 7),
        ("ord-d", "podcast", 10),
    ] {
        db.insert("shared_suite_ordering_ast", id, row(id, kind, score))
            .await
            .expect("ast ordering seed insert should succeed");
    }

    let score_asc = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ordering_ast")
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["score"]))),
                    direction: SortDirection::Asc,
                }]),
        )
        .await
        .expect("ast score ASC ordering query should succeed");
    assert_eq!(
        row_ids(&score_asc),
        vec!["ord-b", "ord-a", "ord-c", "ord-d"]
    );

    let score_desc_expr = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ordering_ast")
                .with_order_by(vec![OrderBy {
                    expr: Expr::Binary {
                        op: BinaryOp::Mul,
                        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "score",
                        ])))),
                        right: Box::new(Expr::Operand(Operand::Literal(Value::I64(-1)))),
                    },
                    direction: SortDirection::Asc,
                }]),
        )
        .await
        .expect("ast expression ordering query should succeed");
    assert_eq!(
        row_ids(&score_desc_expr),
        vec!["ord-d", "ord-a", "ord-c", "ord-b"]
    );

    let multi_key = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ordering_ast")
                .with_order_by(vec![
                    OrderBy {
                        expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["score"]))),
                        direction: SortDirection::Desc,
                    },
                    OrderBy {
                        expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                        direction: SortDirection::Asc,
                    },
                ]),
        )
        .await
        .expect("ast multi-key ordering query should succeed");
    assert_eq!(
        row_ids(&multi_key),
        vec!["ord-d", "ord-a", "ord-c", "ord-b"]
    );
}

async fn test_sql_ordering_variants(db: &Db) {
    if !db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        return;
    }

    db.create_collection("shared_suite_ordering_sql", CollectionKind::Untyped)
        .await
        .expect("sql ordering test collection creation should succeed");

    for (id, kind, score) in [
        ("sql-ord-a", "music", 5),
        ("sql-ord-b", "video", 1),
        ("sql-ord-c", "music", 5),
        ("sql-ord-d", "podcast", 9),
    ] {
        db.insert("shared_suite_ordering_sql", id, row(id, kind, score))
            .await
            .expect("sql ordering seed insert should succeed");
    }

    let by_expr = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_ordering_sql ORDER BY score + 1 DESC, id ASC",
        )
        .await
        .expect("sql expression ordering query should succeed");
    let QueryResult::Select(by_expr_rows) = by_expr else {
        panic!("sql expression ordering query should return SELECT rows");
    };
    assert_eq!(
        row_ids(&by_expr_rows),
        vec!["sql-ord-d", "sql-ord-a", "sql-ord-c", "sql-ord-b"]
    );
}

async fn test_ast_limit_offset_variants(db: &Db) {
    db.create_collection("shared_suite_limit_ast", CollectionKind::Untyped)
        .await
        .expect("ast limit/offset test collection creation should succeed");

    for (id, kind, score) in [
        ("lim-a", "music", 1),
        ("lim-b", "video", 2),
        ("lim-c", "music", 3),
        ("lim-d", "podcast", 4),
    ] {
        db.insert("shared_suite_limit_ast", id, row(id, kind, score))
            .await
            .expect("ast limit/offset seed insert should succeed");
    }

    let rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_limit_ast")
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    direction: SortDirection::Asc,
                }])
                .with_offset(Expr::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(Expr::Operand(Operand::Literal(Value::I64(1)))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(1)))),
                })
                .with_limit(Expr::Binary {
                    op: BinaryOp::Mul,
                    left: Box::new(Expr::Operand(Operand::Literal(Value::I64(1)))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(2)))),
                }),
        )
        .await
        .expect("ast limit/offset expression query should succeed");
    assert_eq!(row_ids(&rows), vec!["lim-c", "lim-d"]);
}

async fn test_sql_limit_offset_variants(db: &Db) {
    if !db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        return;
    }

    db.create_collection("shared_suite_limit_sql", CollectionKind::Untyped)
        .await
        .expect("sql limit/offset test collection creation should succeed");

    for (id, kind, score) in [
        ("sql-lim-a", "music", 1),
        ("sql-lim-b", "video", 2),
        ("sql-lim-c", "music", 3),
        ("sql-lim-d", "podcast", 4),
    ] {
        db.insert("shared_suite_limit_sql", id, row(id, kind, score))
            .await
            .expect("sql limit/offset seed insert should succeed");
    }

    let result = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_limit_sql ORDER BY id ASC LIMIT 1 + 1 OFFSET 1 + 1",
        )
        .await
        .expect("sql limit/offset expression query should succeed");
    let QueryResult::Select(rows) = result else {
        panic!("sql limit/offset expression query should return SELECT rows");
    };
    assert_eq!(row_ids(&rows), vec!["sql-lim-c", "sql-lim-d"]);
}

fn row(id: &str, kind: &str, score: i64) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(id.to_string()));
    row.insert("kind", Value::String(kind.to_string()));
    row.insert("score", Value::I64(score));
    row
}

fn row_ids(rows: &[Object]) -> Vec<String> {
    row_strings(rows, "id")
}

fn row_strings(rows: &[Object], key: &str) -> Vec<String> {
    rows.iter()
        .map(|row| match row.get(key) {
            Some(Value::String(value)) => value.clone(),
            other => panic!("expected string {key} field, got {other:?}"),
        })
        .collect()
}
