use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use semantic_data::query::{
    AggregateOp, BinaryOp, DeleteQuery, Expr, FunctionArg, JoinCondition, JoinQuery, JoinSource,
    JoinType, Operand, OrderBy, PatternMatchKind, QueryField, QueryInput, SelectQuery,
    SortDirection, TextQueryFormat, UpdateQuery,
};
use semantic_data::schema::{
    ClassAttribute, ClassType, Meta, Migration, MigrationCollectionKind, MigrationDdlOperation,
    MigrationIntegrityMode, MigrationOperation, Module, Package, RelationIndexingMode,
    RelationMode, RelationType, StringType, Type, TypeDef, TypeKind, TypeRef, UIntWidth,
    Visibility,
    attribute::{attribute_ref::AttributeRef, attribute_type::AttributeType},
    primitives::{int_width::IntWidth, number_type::NumberType},
};
use semantic_data::value::{FieldPath, Object, Value};
use semantic_db_core::{
    Db, DbError, DdlBatch, DdlCollectionKind, DdlOperation, QueryResult,
    catalog::{CollectionKind, IntegrityMode, RELATION_CLASS_ID},
};

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
    test_package_migrations(db).await;
    test_select_query(db).await;
    test_sql_insert_query(db).await;
    test_sql_end_to_end_regressions(db).await;
    test_update_query(db).await;
    test_delete_query(db).await;
    test_ast_predicate_constructs(db).await;
    test_sql_predicate_constructs(db).await;
    test_subquery_patterns(db).await;
    test_join_semantics(db).await;
    test_nested_ref_field_access(db).await;
    test_ast_ordering_variants(db).await;
    test_sql_ordering_variants(db).await;
    test_ast_limit_offset_variants(db).await;
    test_sql_limit_offset_variants(db).await;
    test_ast_aggregation_distinct_grouping(db).await;
    test_sql_aggregation_distinct_grouping(db).await;
    test_text_query_formats(db).await;
    test_relationships_generic_embedded(db).await;
    test_relationships_generic_external(db).await;
    test_builtin_type_filter_query(db).await;
    test_class_collection_alias_query(db).await;
    test_strict_registered_schema_typeless_insert(db).await;
}

async fn test_schema_registration(db: &Db) {
    db.create_collection("shared_suite_schema", CollectionKind::Polymorphic)
        .await
        .expect("schema registration should succeed");

    let catalog = db
        .catalog()
        .await
        .expect("catalog fetch should succeed after schema registration");
    let collection = catalog
        .collection_by_name("shared_suite_schema")
        .expect("registered collection should be present in catalog");
    assert!(matches!(collection.kind, CollectionKind::Polymorphic));
}

async fn test_package_migrations(db: &Db) {
    let package_v1 = blog_package_v1();
    let outcome_v1 = db
        .upsert_package(package_v1.clone())
        .await
        .expect("initial package registration should succeed");
    assert_eq!(outcome_v1.executed_migrations.len(), 1);

    let catalog = db
        .catalog()
        .await
        .expect("catalog fetch after package registration should succeed");
    let stored_package = catalog
        .package_by_name(&package_v1.name)
        .expect("package should be stored in catalog");
    assert_eq!(
        stored_package.root.types["shared:blog:status"]
            .module
            .as_deref(),
        Some("blog")
    );
    assert_eq!(
        catalog
            .type_def_by_name("shared.blog.status")
            .expect("type definition should be registered")
            .type_def
            .module
            .as_deref(),
        Some("blog")
    );
    assert!(
        catalog
            .applied_migration("shared.blog", "blog", "001_init")
            .is_some(),
        "executed migration should be tracked in the catalog",
    );

    let seeded = db
        .get("shared_suite_blog_posts", "seed")
        .await
        .expect("seeded row lookup should succeed")
        .expect("initial migration should insert a seed row");
    assert_eq!(
        seeded.object.get("shared:blog:title"),
        Some(&Value::String("Hello".to_string()))
    );

    let package_v2 = blog_package_v2();
    let outcome_v2 = db
        .upsert_package(package_v2.clone())
        .await
        .expect("package update with a new migration should succeed");
    assert_eq!(outcome_v2.executed_migrations.len(), 1);
    assert_eq!(outcome_v2.executed_migrations[0].migration.name, "002_body");

    let catalog = db
        .catalog()
        .await
        .expect("catalog fetch after package update should succeed");
    let stored_package = catalog
        .package_by_name(&package_v2.name)
        .expect("updated package should still be stored in catalog");
    assert_eq!(stored_package.name, package_v2.name);
    assert!(stored_package.root.types.contains_key("shared:blog:status"));
    assert!(
        catalog
            .applied_migration("shared.blog", "blog", "002_body")
            .is_some(),
        "newly executed migration should be tracked in the catalog",
    );

    let updated = db
        .get("shared_suite_blog_posts", "seed")
        .await
        .expect("updated seed row lookup should succeed")
        .expect("seed row should still exist after package update");
    assert_eq!(
        updated.object.get("shared:blog:body"),
        Some(&Value::String("World".to_string()))
    );
}

async fn test_strict_registered_schema_typeless_insert(db: &Db) {
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared:strict_typeless_title".to_string(),
                    name: "strict_typeless_title".to_string(),
                    ty: Type {
                        kind: TypeKind::String(StringType {
                            format: None,
                            normalization: None,
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    },
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertCollection {
                name: "shared_suite_strict_typeless".to_string(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::StrictRegisteredSchema,
            }),
    )
    .await
    .expect("strict typeless schema setup should succeed");

    let mut typeless_known = Object::new();
    typeless_known.insert("id", Value::String("known".to_string()));
    typeless_known.insert(
        "shared:strict_typeless_title",
        Value::String("ok".to_string()),
    );
    db.insert("shared_suite_strict_typeless", "known", typeless_known)
        .await
        .expect("strict collection should allow typeless rows with known attributes");

    let mut typeless_unknown = Object::new();
    typeless_unknown.insert("id", Value::String("unknown".to_string()));
    typeless_unknown.insert("rogue", Value::String("x".to_string()));
    let err = db
        .insert("shared_suite_strict_typeless", "unknown", typeless_unknown)
        .await
        .expect_err("strict collection should reject unknown fields for typeless rows");
    assert!(
        err.to_string().contains("not allowed"),
        "expected unknown-field rejection, got: {err}"
    );

    if db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        let ddl_out = db
            .query_text(
                TextQueryFormat::Sql,
                r#"CREATE ATTRIBUTE "shared:strict_typeless_age" TYPE u32"#,
            )
            .await
            .expect("sql create attribute should succeed");
        assert!(matches!(ddl_out, QueryResult::Ddl(())));

        let mut with_new_attr = Object::new();
        with_new_attr.insert("id", Value::String("with-age".to_string()));
        with_new_attr.insert("shared:strict_typeless_age", Value::U32(42));
        db.insert("shared_suite_strict_typeless", "with-age", with_new_attr)
            .await
            .expect("strict collection should accept sql-registered attribute");
    }
}

async fn test_select_query(db: &Db) {
    db.create_collection("shared_suite_select", CollectionKind::Polymorphic)
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
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["kind"]),
                    Value::String("music".to_string()),
                )),
        )
        .await
        .expect("select query should succeed");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("id"), Some(&Value::String("evt-a".to_string())));
    assert_eq!(rows[0].get("score"), Some(&Value::I64(10)));
}

async fn test_builtin_type_filter_query(db: &Db) {
    db.create_collection("shared_suite_type_filter", CollectionKind::Polymorphic)
        .await
        .expect("type filter collection creation should succeed");

    let mut directory = Object::new();
    directory.insert("id", Value::String("type-dir".to_string()));
    directory.insert("type", Value::String("shared.suite.directory".to_string()));
    directory.insert("title", Value::String("Directory".to_string()));
    db.insert("shared_suite_type_filter", "type-dir", directory)
        .await
        .expect("typed row insert should succeed");

    let ast_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_type_filter")
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["type"]),
                    Value::String("shared.suite.directory".to_string()),
                )),
        )
        .await
        .expect("AST type filter query should succeed");
    assert_eq!(row_ids(&ast_rows), vec!["type-dir"]);

    if db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        let result = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT id, type FROM shared_suite_type_filter WHERE type = 'shared.suite.directory'",
            )
            .await
            .expect("SQL type filter query should succeed");
        let QueryResult::Select(sql_rows) = result else {
            panic!("SQL type filter query should return SELECT rows");
        };
        assert_eq!(row_ids(&sql_rows), vec!["type-dir"]);
        assert_eq!(
            sql_rows[0].get("type"),
            Some(&Value::String("shared.suite.directory".to_string())),
        );
    }
}

async fn test_class_collection_alias_query(db: &Db) {
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute {
                attribute: blog_string_attribute("shared.alias.relation.from", "from"),
            })
            .with_op(DdlOperation::UpsertAttribute {
                attribute: blog_string_attribute("shared.alias.node.from", "from"),
            })
            .with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: "shared.alias.relation".to_string(),
                    name: "AliasRelation".to_string(),
                    inherits: None,
                    extends: vec![],
                    attributes: BTreeMap::from([(
                        "from".to_string(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "shared.alias.relation.from".to_string(),
                            },
                            required: true,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    )]),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: "shared.alias.node".to_string(),
                    name: "AliasNode".to_string(),
                    inherits: Some(semantic_data::schema::ClassRef {
                        id: "shared.alias.relation".to_string(),
                    }),
                    extends: vec![],
                    attributes: BTreeMap::from([(
                        "from".to_string(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "shared.alias.node.from".to_string(),
                            },
                            required: true,
                            ui_order: None,
                            computed: None,
                            constraints: vec![],
                            meta: Meta::default(),
                        },
                    )]),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertCollection {
                name: "shared.alias.node".to_string(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            }),
    )
    .await
    .expect("alias query schema setup should succeed");

    let mut row = Object::new();
    row.insert("id", Value::String("alias-node-1".to_string()));
    row.insert("type", Value::String("shared.alias.node".to_string()));
    row.insert("from", Value::String("parent-1".to_string()));
    db.insert("shared.alias.node", "alias-node-1", row)
        .await
        .expect("alias node insert should succeed");

    let ast_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared.alias.node")
                .with_source_alias("n")
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["n", "from"]),
                    Value::String("parent-1".to_string()),
                )),
        )
        .await
        .expect("AST class collection alias query should succeed");
    assert_eq!(row_ids(&ast_rows), vec!["alias-node-1"]);

    if db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        let result = db
            .query_text(
                TextQueryFormat::Sql,
                r#"SELECT n.from FROM "shared.alias.node" AS n WHERE n.from = 'parent-1'"#,
            )
            .await
            .expect("SQL class collection alias query should succeed");
        let QueryResult::Select(sql_rows) = result else {
            panic!("SQL class collection alias query should return SELECT rows");
        };
        assert_eq!(sql_rows.len(), 1);
    }
}

async fn test_update_query(db: &Db) {
    db.create_collection("shared_suite_update", CollectionKind::Polymorphic)
        .await
        .expect("update test collection creation should succeed");
    db.insert("shared_suite_update", "item-1", row("item-1", "music", 1))
        .await
        .expect("update test row insert should succeed");

    let stats = db
        .update_where(
            UpdateQuery::new()
                .with_collection("shared_suite_update")
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["id"]),
                    Value::String("item-1".to_string()),
                ))
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

    db.create_collection("shared_suite_sql_insert", CollectionKind::Polymorphic)
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

async fn test_sql_end_to_end_regressions(db: &Db) {
    if !db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        return;
    }

    const ITEMS: &str = "shared_suite_sql_e2e_items";
    const TAGS: &str = "shared_suite_sql_e2e_tags";

    db.create_collection(ITEMS, CollectionKind::Polymorphic)
        .await
        .expect("SQL end-to-end item collection creation should succeed");
    db.create_collection(TAGS, CollectionKind::Polymorphic)
        .await
        .expect("SQL end-to-end tag collection creation should succeed");

    db.insert(ITEMS, "sql-e2e-a", row("sql-e2e-a", "music", 4))
        .await
        .expect("first SQL end-to-end item insert should succeed");
    db.insert(ITEMS, "sql-e2e-b", row("sql-e2e-b", "video", 12))
        .await
        .expect("second SQL end-to-end item insert should succeed");
    db.insert(TAGS, "sql-tag-a", row("sql-tag-a", "tag", 0))
        .await
        .expect("first SQL end-to-end tag insert should succeed");
    db.insert(TAGS, "sql-tag-b", row("sql-tag-b", "tag", 0))
        .await
        .expect("second SQL end-to-end tag insert should succeed");

    let case_result = db
        .query_text(
            TextQueryFormat::Sql,
            format!(
                "SELECT id, CASE WHEN score > 10 THEN 'high' WHEN score > 0 THEN 'positive' ELSE 'none' END AS band, CASE kind WHEN 'music' THEN 'audio' WHEN 'video' THEN 'visual' ELSE 'other' END AS category FROM {ITEMS} ORDER BY id"
            ),
        )
        .await
        .expect("searched and simple multi-branch CASE SQL should execute");
    let QueryResult::Select(case_rows) = case_result else {
        panic!("CASE SQL query should return SELECT rows");
    };
    assert_eq!(row_ids(&case_rows), vec!["sql-e2e-a", "sql-e2e-b"]);
    assert_eq!(
        row_strings(&case_rows, "band"),
        vec!["positive".to_string(), "high".to_string()]
    );
    assert_eq!(
        row_strings(&case_rows, "category"),
        vec!["audio".to_string(), "visual".to_string()]
    );

    let cross_result = db
        .query_text(
            TextQueryFormat::Sql,
            format!(
                "SELECT i.id AS item_id, t.id AS tag_id FROM {ITEMS} AS i CROSS JOIN {TAGS}._ AS t ORDER BY i.id, t.id"
            ),
        )
        .await
        .expect("cross-collection CROSS JOIN SQL should execute");
    let QueryResult::Select(cross_rows) = cross_result else {
        panic!("CROSS JOIN SQL query should return SELECT rows");
    };
    let pairs = cross_rows
        .iter()
        .map(|row| {
            (
                row.get("item_id")
                    .and_then(Value::as_str)
                    .expect("CROSS JOIN row should contain item_id"),
                row.get("tag_id")
                    .and_then(Value::as_str)
                    .expect("CROSS JOIN row should contain tag_id"),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        vec![
            ("sql-e2e-a", "sql-tag-a"),
            ("sql-e2e-a", "sql-tag-b"),
            ("sql-e2e-b", "sql-tag-a"),
            ("sql-e2e-b", "sql-tag-b"),
        ]
    );

    let update_result = db
        .query_text(
            TextQueryFormat::Sql,
            format!("UPDATE {ITEMS} SET score = score + 6 WHERE id = 'sql-e2e-a' RETURNING *"),
        )
        .await
        .expect("SQL UPDATE RETURNING wildcard should execute");
    let QueryResult::Update(update) = update_result else {
        panic!("SQL UPDATE RETURNING wildcard should return an UPDATE result");
    };
    assert_eq!(update.stats.matched, 1);
    assert_eq!(update.stats.affected, 1);
    assert_eq!(update.returning.len(), 1);
    let mut expected_updated_row = row("sql-e2e-a", "music", 4);
    expected_updated_row.insert("score", Value::F64(10.0.into()));
    assert_eq!(update.returning[0], expected_updated_row);

    for invalid_sql in [
        format!("UPDATE {ITEMS} SET score = 999 LIMIT -1"),
        format!("DELETE FROM {ITEMS} LIMIT -1"),
    ] {
        db.query_text(TextQueryFormat::Sql, invalid_sql)
            .await
            .expect_err("invalid SQL DML LIMIT should be rejected");
    }
    let unchanged = db
        .get(ITEMS, "sql-e2e-a")
        .await
        .expect("row lookup after invalid SQL DML LIMIT should succeed")
        .expect("invalid SQL DML LIMIT must not remove the row");
    assert_eq!(
        unchanged.object.get("score"),
        Some(&Value::F64(10.0.into()))
    );
    assert!(
        db.get(ITEMS, "sql-e2e-b")
            .await
            .expect("control row lookup after invalid DELETE LIMIT should succeed")
            .is_some()
    );

    let delete_result = db
        .query_text(
            TextQueryFormat::Sql,
            format!("DELETE FROM {ITEMS} WHERE id = 'sql-e2e-b' RETURNING *"),
        )
        .await
        .expect("SQL DELETE RETURNING wildcard should execute");
    let QueryResult::Delete(delete) = delete_result else {
        panic!("SQL DELETE RETURNING wildcard should return a DELETE result");
    };
    assert_eq!(delete.deleted, 1);
    assert_eq!(delete.returning.len(), 1);
    assert_eq!(delete.returning[0], row("sql-e2e-b", "video", 12));
    assert!(
        db.get(ITEMS, "sql-e2e-b")
            .await
            .expect("deleted SQL end-to-end row lookup should succeed")
            .is_none()
    );

    let mixed_projection = db
        .query_text(
            TextQueryFormat::Sql,
            format!("SELECT *, score + 1 AS boosted FROM {ITEMS} WHERE id = 'sql-e2e-a'"),
        )
        .await
        .expect("mixed wildcard SQL projection should execute");
    let QueryResult::Select(mixed_rows) = mixed_projection else {
        panic!("mixed wildcard SQL projection should return SELECT rows");
    };
    assert_eq!(mixed_rows.len(), 1);
    assert_eq!(mixed_rows[0].len(), 4);
    assert_eq!(
        mixed_rows[0].get("id"),
        Some(&Value::String("sql-e2e-a".to_string()))
    );
    assert_eq!(
        mixed_rows[0].get("kind"),
        Some(&Value::String("music".to_string()))
    );
    assert_eq!(mixed_rows[0].get("score"), Some(&Value::F64(10.0.into())));
    assert_eq!(mixed_rows[0].get("boosted"), Some(&Value::F64(11.0.into())));
}

async fn test_delete_query(db: &Db) {
    db.create_collection("shared_suite_delete", CollectionKind::Polymorphic)
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
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["kind"]),
                    Value::String("temp".to_string()),
                )),
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
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared:test:kind".to_string(),
                    name: "kind".to_string(),
                    ty: Type {
                        kind: TypeKind::String(StringType {
                            format: None,
                            normalization: None,
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    },
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared:test:score".to_string(),
                    name: "score".to_string(),
                    ty: Type {
                        kind: TypeKind::Number(NumberType::Int(IntWidth::I64)),
                        constraints: vec![],
                        annotations: vec![],
                    },
                    constraints: vec![],
                    meta: Meta::default(),
                },
            }),
    )
    .await
    .expect("text query strict attributes setup should succeed");
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
    db.create_collection("shared_suite_ast_predicates", CollectionKind::Polymorphic)
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
                .with_predicate(Expr::InList {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    list: vec![
                        Expr::Operand(Operand::Literal(Value::String("Music".to_string()))),
                        Expr::Operand(Operand::Literal(Value::String("podcast".to_string()))),
                    ],
                    negated: false,
                }),
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
                .with_predicate(Expr::InList {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    list: vec![Expr::Operand(Operand::Literal(Value::String(
                        "Music".to_string(),
                    )))],
                    negated: true,
                }),
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
                .with_predicate(Expr::PatternMatch {
                    kind: PatternMatchKind::Like,
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "note",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "hello%".to_string(),
                    )))),
                    case_insensitive: false,
                    negated: false,
                }),
        )
        .await
        .expect("ast LIKE query should succeed");
    assert_eq!(like_rows.len(), 1);

    let ilike_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::PatternMatch {
                    kind: PatternMatchKind::Like,
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "kind",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "music".to_string(),
                    )))),
                    case_insensitive: true,
                    negated: false,
                }),
        )
        .await
        .expect("ast ILIKE query should succeed");
    assert_eq!(ilike_rows.len(), 1);

    let similar_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::PatternMatch {
                    kind: PatternMatchKind::SimilarTo,
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "note",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "hello%".to_string(),
                    )))),
                    case_insensitive: false,
                    negated: false,
                }),
        )
        .await
        .expect("ast SIMILAR TO query should succeed");
    assert_eq!(similar_rows.len(), 1);

    let regex_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::RegexMatch {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "note",
                    ])))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "^hello".to_string(),
                    )))),
                    case_insensitive: false,
                    negated: false,
                }),
        )
        .await
        .expect("ast regex query should succeed");
    assert_eq!(regex_rows.len(), 1);

    let between_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::Between {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "score",
                    ])))),
                    low: Box::new(Expr::Operand(Operand::Literal(Value::I64(5)))),
                    high: Box::new(Expr::Operand(Operand::Literal(Value::I64(12)))),
                    negated: false,
                }),
        )
        .await
        .expect("ast BETWEEN query should succeed");
    assert_eq!(between_rows.len(), 1);

    let null_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::IsNull {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "opt",
                    ])))),
                    negated: false,
                }),
        )
        .await
        .expect("ast IS NULL query should succeed");
    assert_eq!(null_rows.len(), 1);

    let not_null_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::IsNull {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "opt",
                    ])))),
                    negated: true,
                }),
        )
        .await
        .expect("ast IS NOT NULL query should succeed");
    assert_eq!(not_null_rows.len(), 1);

    let exists_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::Exists {
                    query: Box::new(
                        SelectQuery::new()
                            .with_collection("shared_suite_ast_predicates")
                            .with_predicate(eq_predicate(
                                FieldPath::from_fields(["id"]),
                                Value::String("ast-a".to_string()),
                            )),
                    ),
                    negated: false,
                }),
        )
        .await
        .expect("ast EXISTS query should succeed");
    assert_eq!(exists_rows.len(), 2);

    let func_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ast_predicates")
                .with_predicate(Expr::Binary {
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
                }),
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

    db.create_collection("shared_suite_sql_predicates", CollectionKind::Polymorphic)
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

    let similar_error = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id FROM shared_suite_sql_predicates WHERE note SIMILAR TO 'hello%'",
        )
        .await
        .expect_err("SQL SIMILAR TO should be rejected until its semantics are implemented");
    assert!(similar_error.to_string().contains("SIMILAR TO"));

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
            "SELECT id FROM shared_suite_sql_predicates WHERE EXISTS (SELECT nested.id FROM shared_suite_sql_predicates AS nested WHERE nested.id = 'sql-a')",
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

async fn test_subquery_patterns(db: &Db) {
    db.create_collection("shared_suite_subquery_outer", CollectionKind::Polymorphic)
        .await
        .expect("subquery outer collection creation should succeed");
    db.create_collection("shared_suite_subquery_inner", CollectionKind::Polymorphic)
        .await
        .expect("subquery inner collection creation should succeed");

    for (id, score) in [("sq-a", 1), ("sq-b", 2), ("sq-c", 3)] {
        let mut row = Object::new();
        row.insert("id", Value::String(id.to_string()));
        row.insert("score", Value::I64(score));
        db.insert("shared_suite_subquery_outer", id, row)
            .await
            .expect("subquery outer seed insert should succeed");
    }
    for (id, value, tag) in [
        ("in-1", 2, "in"),
        ("in-2", 3, "in"),
        ("out-1", 1, "out"),
        ("only-1", 3, "only"),
    ] {
        let mut row = Object::new();
        row.insert("id", Value::String(id.to_string()));
        row.insert("value", Value::I64(value));
        row.insert("tag", Value::String(tag.to_string()));
        db.insert("shared_suite_subquery_inner", id, row)
            .await
            .expect("subquery inner seed insert should succeed");
    }

    let in_subquery_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_subquery_outer")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::In,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "score",
                    ])))),
                    right: Box::new(Expr::Subquery(Box::new(
                        SelectQuery::new()
                            .with_collection("shared_suite_subquery_inner")
                            .with_predicate(eq_predicate(
                                FieldPath::from_fields(["tag"]),
                                Value::String("in".to_string()),
                            ))
                            .with_projection(vec![QueryField {
                                expr: Box::new(Expr::Operand(Operand::Field(
                                    FieldPath::from_fields(["value"]),
                                ))),
                                alias: None,
                            }]),
                    ))),
                })
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    direction: SortDirection::Asc,
                }]),
        )
        .await
        .expect("ast IN subquery query should succeed");
    assert_eq!(row_ids(&in_subquery_rows), vec!["sq-b", "sq-c"]);

    let not_in_subquery_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_subquery_outer")
                .with_predicate(Expr::Unary {
                    op: semantic_data::query::UnaryOp::Not,
                    expr: Box::new(Expr::Binary {
                        op: BinaryOp::In,
                        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "score",
                        ])))),
                        right: Box::new(Expr::Subquery(Box::new(
                            SelectQuery::new()
                                .with_collection("shared_suite_subquery_inner")
                                .with_predicate(eq_predicate(
                                    FieldPath::from_fields(["tag"]),
                                    Value::String("in".to_string()),
                                ))
                                .with_projection(vec![QueryField {
                                    expr: Box::new(Expr::Operand(Operand::Field(
                                        FieldPath::from_fields(["value"]),
                                    ))),
                                    alias: None,
                                }]),
                        ))),
                    }),
                }),
        )
        .await
        .expect("ast NOT IN subquery query should succeed");
    assert_eq!(row_ids(&not_in_subquery_rows), vec!["sq-a"]);

    let scalar_subquery_predicate_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_subquery_outer")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "score",
                    ])))),
                    right: Box::new(Expr::Subquery(Box::new(
                        SelectQuery::new()
                            .with_collection("shared_suite_subquery_inner")
                            .with_predicate(eq_predicate(
                                FieldPath::from_fields(["tag"]),
                                Value::String("only".to_string()),
                            ))
                            .with_projection(vec![QueryField {
                                expr: Box::new(Expr::Operand(Operand::Field(
                                    FieldPath::from_fields(["value"]),
                                ))),
                                alias: None,
                            }]),
                    ))),
                }),
        )
        .await
        .expect("ast scalar subquery predicate should succeed");
    assert_eq!(row_ids(&scalar_subquery_predicate_rows), vec!["sq-c"]);

    let scalar_subquery_projection = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_subquery_outer")
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    direction: SortDirection::Asc,
                }])
                .with_projection(vec![
                    QueryField {
                        expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "id",
                        ])))),
                        alias: Some("id".to_string()),
                    },
                    QueryField {
                        expr: Box::new(Expr::Subquery(Box::new(
                            SelectQuery::new()
                                .with_collection("shared_suite_subquery_inner")
                                .with_predicate(eq_predicate(
                                    FieldPath::from_fields(["tag"]),
                                    Value::String("only".to_string()),
                                ))
                                .with_projection(vec![QueryField {
                                    expr: Box::new(Expr::Operand(Operand::Field(
                                        FieldPath::from_fields(["value"]),
                                    ))),
                                    alias: None,
                                }]),
                        ))),
                        alias: Some("cutoff".to_string()),
                    },
                ]),
        )
        .await
        .expect("ast scalar subquery projection should succeed");
    assert_eq!(scalar_subquery_projection.len(), 3);
    assert!(
        scalar_subquery_projection
            .iter()
            .all(|row| row.get("cutoff") == Some(&Value::I64(3)))
    );

    if db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        let in_sql = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT id FROM shared_suite_subquery_outer WHERE score IN (SELECT inner_rows.value FROM shared_suite_subquery_inner AS inner_rows WHERE inner_rows.tag = 'in') ORDER BY id",
            )
            .await
            .expect("sql IN subquery query should succeed");
        let QueryResult::Select(in_sql_rows) = in_sql else {
            panic!("sql IN subquery query should return SELECT rows");
        };
        assert_eq!(row_ids(&in_sql_rows), vec!["sq-b", "sq-c"]);

        let not_in_sql = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT id FROM shared_suite_subquery_outer WHERE score NOT IN (SELECT inner_rows.value FROM shared_suite_subquery_inner AS inner_rows WHERE inner_rows.tag = 'in') ORDER BY id",
            )
            .await
            .expect("sql NOT IN subquery query should succeed");
        let QueryResult::Select(not_in_sql_rows) = not_in_sql else {
            panic!("sql NOT IN subquery query should return SELECT rows");
        };
        assert_eq!(row_ids(&not_in_sql_rows), vec!["sq-a"]);

        let scalar_predicate_sql = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT id FROM shared_suite_subquery_outer WHERE score = (SELECT inner_rows.value FROM shared_suite_subquery_inner AS inner_rows WHERE inner_rows.tag = 'only')",
            )
            .await
            .expect("sql scalar subquery predicate should succeed");
        let QueryResult::Select(scalar_predicate_rows) = scalar_predicate_sql else {
            panic!("sql scalar subquery predicate should return SELECT rows");
        };
        assert_eq!(row_ids(&scalar_predicate_rows), vec!["sq-c"]);

        let scalar_projection_sql = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT id, (SELECT inner_rows.value FROM shared_suite_subquery_inner AS inner_rows WHERE inner_rows.tag = 'only') AS cutoff FROM shared_suite_subquery_outer ORDER BY id",
            )
            .await
            .expect("sql scalar subquery projection should succeed");
        let QueryResult::Select(scalar_projection_rows) = scalar_projection_sql else {
            panic!("sql scalar subquery projection should return SELECT rows");
        };
        assert_eq!(scalar_projection_rows.len(), 3);
        assert!(
            scalar_projection_rows
                .iter()
                .all(|row| row.get("cutoff") == Some(&Value::I64(3)))
        );
    }
}

async fn test_join_semantics(db: &Db) {
    db.create_collection("shared_suite_join_items", CollectionKind::Polymorphic)
        .await
        .expect("join source collection creation should succeed");
    db.create_collection("shared_suite_join_profiles", CollectionKind::Polymorphic)
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
                "SELECT s.id AS sid, a.name AS artist_name FROM shared_suite_join_items AS s INNER JOIN _ AS a ON s.artist_id = a.id WHERE (s.type = 'Song' OR s.type = 'local:Song') AND (a.type = 'Artist' OR a.type = 'local:Artist')",
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
                "SELECT s.id AS sid, a.name AS artist_name FROM shared_suite_join_items AS s INNER JOIN Artist AS a ON s.artist_id = a.id WHERE (s.type = 'Song' OR s.type = 'local:Song')",
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
                "SELECT s.id AS sid, p.kind AS profile_kind FROM shared_suite_join_items AS s INNER JOIN shared_suite_join_profiles._ AS p ON s.artist_id = p.id WHERE (s.type = 'Song' OR s.type = 'local:Song')",
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
                "SELECT s.id AS sid FROM shared_suite_join_items AS s INNER JOIN shared_suite_join_profiles._ AS p ON s.artist_id = p.id AND p.kind = 'featured' WHERE (s.type = 'Song' OR s.type = 'local:Song')",
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
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["s", "type"]),
                    Value::String("Song".to_string()),
                ))
                .with_joins(vec![JoinQuery {
                    source: JoinSource {
                        collection: Some("shared_suite_join_profiles".to_string()),
                        class: None,
                    },
                    alias: Some("p".to_string()),
                    join_type: JoinType::Inner,
                    condition: JoinCondition::OnExpr(eq_predicate_exprs(
                        Expr::Operand(Operand::Field(FieldPath::from_fields(["s", "artist_id"]))),
                        Expr::Operand(Operand::Field(FieldPath::from_fields(["p", "id"]))),
                    )),
                    predicate: Some(eq_predicate(
                        FieldPath::from_fields(["kind"]),
                        Value::String("featured".to_string()),
                    )),
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

async fn test_nested_ref_field_access(db: &Db) {
    db.create_collection("shared_suite_ref_paths", CollectionKind::Polymorphic)
        .await
        .expect("ref path test collection creation should succeed");

    let mut grand = Object::new();
    grand.insert("id", Value::String("ref-grand".to_string()));
    grand.insert("kind", Value::String("top".to_string()));
    grand.insert("title", Value::String("root".to_string()));
    db.insert("shared_suite_ref_paths", "ref-grand", grand)
        .await
        .expect("grand row insert should succeed");

    let mut parent = Object::new();
    parent.insert("id", Value::String("ref-parent".to_string()));
    parent.insert("kind", Value::String("blah".to_string()));
    parent.insert("title", Value::String("abc".to_string()));
    parent.insert("parent", Value::String("ref-grand".to_string()));
    db.insert("shared_suite_ref_paths", "ref-parent", parent)
        .await
        .expect("parent row insert should succeed");

    let mut child = Object::new();
    child.insert("id", Value::String("ref-child".to_string()));
    child.insert("kind", Value::String("leaf".to_string()));
    child.insert("parent", Value::String("ref-parent".to_string()));
    // "manager" is intentionally unregistered so this must use ad-hoc fallback.
    child.insert("manager", Value::String("ref-parent".to_string()));
    db.insert("shared_suite_ref_paths", "ref-child", child)
        .await
        .expect("child row insert should succeed");

    let via_parent_query = SelectQuery::new()
        .with_collection("shared_suite_ref_paths")
        .with_predicate(eq_predicate(
            FieldPath::from_fields(["parent", "title"]),
            Value::String("abc".to_string()),
        ))
        .with_projection(vec![QueryField {
            expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "id",
            ])))),
            alias: Some("id".to_string()),
        }]);
    let via_parent = db
        .select(via_parent_query)
        .await
        .expect("single-hop ref path query should succeed");
    assert_eq!(row_ids(&via_parent), vec!["ref-child"]);

    let nested = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ref_paths")
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["parent", "kind"]),
                    Value::String("blah".to_string()),
                ))
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "parent", "parent", "id",
                    ])))),
                    alias: Some("ancestor".to_string()),
                }]),
        )
        .await
        .expect("nested ref path query should succeed");
    assert_eq!(
        row_strings(&nested, "ancestor"),
        vec!["ref-grand".to_string()]
    );

    let fallback = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_ref_paths")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Function {
                        name: "LOWER".to_string(),
                        args: vec![FunctionArg::Expr(Expr::Operand(Operand::Field(
                            FieldPath::from_fields(["manager", "title"]),
                        )))],
                    }),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "abc".to_string(),
                    )))),
                })
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "id",
                    ])))),
                    alias: Some("id".to_string()),
                }]),
        )
        .await
        .expect("ad-hoc nested ref fallback query should succeed");
    assert_eq!(row_ids(&fallback), vec!["ref-child"]);
}

async fn test_ast_aggregation_distinct_grouping(db: &Db) {
    db.create_collection("shared_suite_ast_agg", CollectionKind::Polymorphic)
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
                .with_having(Expr::Binary {
                    op: BinaryOp::Gt,
                    left: Box::new(Expr::Aggregate {
                        op: AggregateOp::Sum,
                        distinct: false,
                        arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                            FieldPath::from_fields(["score"]),
                        )))),
                    }),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::F64(10.0.into())))),
                }),
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
    db.create_collection("shared_suite_sql_agg", CollectionKind::Polymorphic)
        .await
        .expect("sql aggregation test collection creation should succeed");
    db.create_collection("shared_suite_sql_agg_empty", CollectionKind::Polymorphic)
        .await
        .expect("empty sql aggregation test collection creation should succeed");
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

    let empty_global = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT COUNT(*) + 1 AS adjusted, 'empty' AS label, COALESCE(SUM(score), 0) AS total, SUM(score) + 1 AS nullable, SUM(score) IS NULL AS sum_is_null FROM shared_suite_sql_agg_empty HAVING COUNT(*) = 0",
        )
        .await
        .expect("empty sql global aggregate query should succeed");
    let QueryResult::Select(empty_global_rows) = empty_global else {
        panic!("empty sql global aggregate query should return SELECT rows");
    };
    assert_eq!(empty_global_rows.len(), 1);
    assert_eq!(
        empty_global_rows[0].get("adjusted"),
        Some(&Value::F64(1.0.into()))
    );
    assert_eq!(
        empty_global_rows[0].get("label"),
        Some(&Value::String("empty".to_string()))
    );
    assert_eq!(empty_global_rows[0].get("total"), Some(&Value::I64(0)));
    assert_eq!(empty_global_rows[0].get("nullable"), Some(&Value::Null));
    assert_eq!(
        empty_global_rows[0].get("sum_is_null"),
        Some(&Value::Bool(true))
    );

    let aggregate_predicates = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT SUM(score) BETWEEN 30.0 AND 50.0 AS in_range, SUM(score) NOT BETWEEN 30.0 AND 50.0 AS out_of_range, COUNT(*) IN (4, NULL) AS count_match, COUNT(*) NOT IN (4, NULL) AS count_not_match, MAX(kind) LIKE 'v%' AS max_like, MAX(kind) ~ '^v' AS max_regex FROM shared_suite_sql_agg",
        )
        .await
        .expect("aggregate predicate expressions should succeed");
    let QueryResult::Select(aggregate_predicate_rows) = aggregate_predicates else {
        panic!("aggregate predicate expressions should return SELECT rows");
    };
    assert_eq!(aggregate_predicate_rows.len(), 1);
    for key in ["in_range", "count_match", "max_like", "max_regex"] {
        assert_eq!(
            aggregate_predicate_rows[0].get(key),
            Some(&Value::Bool(true)),
            "{key}"
        );
    }
    for key in ["out_of_range", "count_not_match"] {
        assert_eq!(
            aggregate_predicate_rows[0].get(key),
            Some(&Value::Bool(false)),
            "{key}"
        );
    }

    let empty_aggregate_predicates = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT SUM(score) BETWEEN 1.0 AND 2.0 AS in_range, SUM(score) NOT BETWEEN 1.0 AND 2.0 AS out_of_range, COUNT(*) IN (1, NULL) AS count_match, COUNT(*) NOT IN (1, NULL) AS count_not_match, MAX(kind) LIKE 'v%' AS max_like, MAX(kind) !~ '^v' AS max_not_regex FROM shared_suite_sql_agg_empty",
        )
        .await
        .expect("empty aggregate predicate expressions should succeed");
    let QueryResult::Select(empty_aggregate_predicate_rows) = empty_aggregate_predicates else {
        panic!("empty aggregate predicate expressions should return SELECT rows");
    };
    assert_eq!(empty_aggregate_predicate_rows.len(), 1);
    for key in [
        "in_range",
        "out_of_range",
        "count_match",
        "count_not_match",
        "max_like",
        "max_not_regex",
    ] {
        assert_eq!(
            empty_aggregate_predicate_rows[0].get(key),
            Some(&Value::Null),
            "{key}"
        );
    }

    let by_alias = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT kind, SUM(score) AS total FROM shared_suite_sql_agg GROUP BY kind ORDER BY total DESC",
        )
        .await
        .expect("sql aggregate alias ordering query should succeed");
    let QueryResult::Select(by_alias_rows) = by_alias else {
        panic!("sql aggregate alias ordering query should return SELECT rows");
    };
    assert_eq!(
        row_strings(&by_alias_rows, "kind"),
        vec!["music".to_string(), "video".to_string()]
    );

    let by_ordinal = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT kind, SUM(score) AS total FROM shared_suite_sql_agg GROUP BY kind ORDER BY 2 ASC",
        )
        .await
        .expect("sql aggregate ordinal ordering query should succeed");
    let QueryResult::Select(by_ordinal_rows) = by_ordinal else {
        panic!("sql aggregate ordinal ordering query should return SELECT rows");
    };
    assert_eq!(
        row_strings(&by_ordinal_rows, "kind"),
        vec!["video".to_string(), "music".to_string()]
    );
    assert_eq!(
        by_ordinal_rows
            .iter()
            .map(|row| row.get("total").cloned())
            .collect::<Vec<_>>(),
        vec![Some(Value::F64(10.0.into())), Some(Value::F64(30.0.into()))]
    );

    let grouped_derived = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT score + 1 AS bucket, COUNT(*) AS n FROM shared_suite_sql_agg GROUP BY score + 1 ORDER BY bucket ASC",
        )
        .await
        .expect("sql grouped derived expression query should succeed");
    let QueryResult::Select(grouped_derived_rows) = grouped_derived else {
        panic!("sql grouped derived expression query should return SELECT rows");
    };
    assert_eq!(grouped_derived_rows.len(), 3);
    assert_eq!(
        grouped_derived_rows
            .iter()
            .map(|row| (row.get("bucket").cloned(), row.get("n").cloned()))
            .collect::<Vec<_>>(),
        vec![
            (Some(Value::F64(6.0.into())), Some(Value::I64(2))),
            (Some(Value::F64(11.0.into())), Some(Value::I64(1))),
            (Some(Value::F64(21.0.into())), Some(Value::I64(1))),
        ]
    );

    let grouped_ordinal = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT score + 1 AS bucket, COUNT(*) AS n FROM shared_suite_sql_agg GROUP BY 1 ORDER BY bucket ASC",
        )
        .await
        .expect("sql derived projection GROUP BY ordinal query should succeed");
    let QueryResult::Select(grouped_ordinal_rows) = grouped_ordinal else {
        panic!("sql derived projection GROUP BY ordinal query should return SELECT rows");
    };
    assert_eq!(grouped_ordinal_rows, grouped_derived_rows);

    let ambiguous_group_alias = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT score + 1 AS bucket, COUNT(*) AS n FROM shared_suite_sql_agg GROUP BY bucket",
        )
        .await
        .expect_err("ambiguous SQL GROUP BY output alias should be rejected");
    let ambiguous_group_alias = ambiguous_group_alias.to_string();
    assert!(ambiguous_group_alias.contains("ambiguous with a projection alias"));
    assert!(ambiguous_group_alias.contains("ordinal or repeat"));

    for invalid_sql in [
        "SELECT kind, score, COUNT(*) AS n FROM shared_suite_sql_agg GROUP BY kind",
        "SELECT kind, COUNT(*) AS n FROM shared_suite_sql_agg GROUP BY kind HAVING score > 0",
        "SELECT kind, COUNT(*) AS n FROM shared_suite_sql_agg GROUP BY kind ORDER BY score",
        "SELECT id FROM shared_suite_sql_agg WHERE SUM(score) > 0",
        "SELECT kind FROM shared_suite_sql_agg GROUP BY SUM(score)",
        "SELECT COUNT(SUM(score)) AS n FROM shared_suite_sql_agg",
        "SELECT id, score AS id FROM shared_suite_sql_agg",
        "SELECT *, COUNT(*) AS n FROM shared_suite_sql_agg",
        "SELECT id FROM shared_suite_sql_agg ORDER BY 0",
        "SELECT id FROM shared_suite_sql_agg ORDER BY 2",
    ] {
        db.query_text(TextQueryFormat::Sql, invalid_sql)
            .await
            .expect_err("invalid SQL select semantics should be rejected");
    }

    assert!(
        db.get("shared_suite_sql_agg", "sql-agg-a")
            .await
            .expect("seed lookup after invalid SQL queries should succeed")
            .is_some(),
        "invalid SQL queries must not affect stored rows"
    );
}

async fn test_ast_ordering_variants(db: &Db) {
    db.create_collection("shared_suite_ordering_ast", CollectionKind::Polymorphic)
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

    db.create_collection("shared_suite_ordering_sql", CollectionKind::Polymorphic)
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

    let by_alias = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id, score + 1 AS rank FROM shared_suite_ordering_sql ORDER BY rank DESC, id ASC",
        )
        .await
        .expect("sql projection alias ordering query should succeed");
    let QueryResult::Select(by_alias_rows) = by_alias else {
        panic!("sql projection alias ordering query should return SELECT rows");
    };
    assert_eq!(
        row_ids(&by_alias_rows),
        vec!["sql-ord-d", "sql-ord-a", "sql-ord-c", "sql-ord-b"]
    );
    assert_eq!(
        by_alias_rows
            .iter()
            .map(|row| row.get("rank").cloned())
            .collect::<Vec<_>>(),
        vec![
            Some(Value::F64(10.0.into())),
            Some(Value::F64(6.0.into())),
            Some(Value::F64(6.0.into())),
            Some(Value::F64(2.0.into())),
        ]
    );

    let by_ordinal = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id, score FROM shared_suite_ordering_sql ORDER BY 2 DESC, 1 DESC",
        )
        .await
        .expect("sql projection ordinal ordering query should succeed");
    let QueryResult::Select(by_ordinal_rows) = by_ordinal else {
        panic!("sql projection ordinal ordering query should return SELECT rows");
    };
    assert_eq!(
        row_ids(&by_ordinal_rows),
        vec!["sql-ord-d", "sql-ord-c", "sql-ord-a", "sql-ord-b"]
    );
}

async fn test_ast_limit_offset_variants(db: &Db) {
    db.create_collection("shared_suite_limit_ast", CollectionKind::Polymorphic)
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

    db.create_collection("shared_suite_limit_sql", CollectionKind::Polymorphic)
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

    let alias_page = db
        .query_text(
            TextQueryFormat::Sql,
            "SELECT id, score + 1 AS rank FROM shared_suite_limit_sql ORDER BY rank ASC, id ASC LIMIT 2 OFFSET 1",
        )
        .await
        .expect("sql alias ordered limit/offset query should succeed");
    let QueryResult::Select(alias_page_rows) = alias_page else {
        panic!("sql alias ordered limit/offset query should return SELECT rows");
    };
    assert_eq!(row_ids(&alias_page_rows), vec!["sql-lim-b", "sql-lim-c"]);
    assert_eq!(
        alias_page_rows
            .iter()
            .map(|row| row.get("rank").cloned())
            .collect::<Vec<_>>(),
        vec![Some(Value::F64(3.0.into())), Some(Value::F64(4.0.into()))]
    );
}

async fn test_relationships_generic_embedded(db: &Db) {
    db.create_collection("shared_suite_rel_nodes", CollectionKind::Polymorphic)
        .await
        .expect("relationship nodes collection creation should succeed");
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared.rel.parent_ref".to_string(),
                    name: "parent_ref".to_string(),
                    ty: Type {
                        kind: TypeKind::Ref(semantic_data::schema::core::type_ref::TypeRef {
                            name: "id".to_string(),
                            args: vec![],
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    },
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertRelationship {
                relationship: RelationType {
                    id: "shared.rel.parent".to_string(),
                    name: "parent".to_string(),
                    source_collection: "shared_suite_rel_nodes".to_string(),
                    mode: RelationMode::Embedded {
                        attribute: "shared.rel.parent_ref".to_string(),
                    },
                    indexing_mode: RelationIndexingMode::Enabled,
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared.rel.secondary_ref".to_string(),
                    name: "secondary_ref".to_string(),
                    ty: Type {
                        kind: TypeKind::Ref(semantic_data::schema::core::type_ref::TypeRef {
                            name: "id".to_string(),
                            args: vec![],
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    },
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertRelationship {
                relationship: RelationType {
                    id: "shared.rel.secondary".to_string(),
                    name: "secondary".to_string(),
                    source_collection: "shared_suite_rel_nodes".to_string(),
                    mode: RelationMode::Embedded {
                        attribute: "shared.rel.secondary_ref".to_string(),
                    },
                    indexing_mode: RelationIndexingMode::Enabled,
                    meta: Meta::default(),
                },
            }),
    )
    .await
    .expect("embedded relationship DDL should succeed");

    let mut a = Object::new();
    a.insert("id", Value::String("a".to_string()));
    a.insert("depth", Value::U64(1));
    db.insert("shared_suite_rel_nodes", "a", a)
        .await
        .expect("insert a should succeed");
    let mut b = Object::new();
    b.insert("id", Value::String("b".to_string()));
    b.insert("depth", Value::U64(1));
    b.insert("shared.rel.parent_ref", Value::String("a".to_string()));
    db.insert("shared_suite_rel_nodes", "b", b)
        .await
        .expect("insert b should succeed");
    let mut d = Object::new();
    d.insert("id", Value::String("d".to_string()));
    d.insert("depth", Value::U64(1));
    db.insert("shared_suite_rel_nodes", "d", d)
        .await
        .expect("insert d should succeed");
    let mut c = Object::new();
    c.insert("id", Value::String("c".to_string()));
    c.insert("depth", Value::U64(2));
    c.insert("shared.rel.parent_ref", Value::String("b".to_string()));
    c.insert("shared.rel.secondary_ref", Value::String("d".to_string()));
    db.insert("shared_suite_rel_nodes", "c", c)
        .await
        .expect("insert c should succeed");
    let ast_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(Expr::RelationExists {
                    relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "shared.rel.parent".to_string(),
                    )))),
                    source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "id",
                    ])))),
                    target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "a".to_string(),
                    )))),
                    transitive: true,
                    max_depth: None,
                })
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    direction: SortDirection::Asc,
                }]),
        )
        .await
        .expect("ast relation query should succeed");
    assert_eq!(row_ids(&ast_rows), vec!["b", "c"]);

    let relation_and_scalar = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::And,
                    left: Box::new(eq_predicate(
                        FieldPath::from_fields(["id"]),
                        Value::String("c".to_string()),
                    )),
                    right: Box::new(Expr::RelationExists {
                        relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                            "shared.rel.parent".to_string(),
                        )))),
                        source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "id",
                        ])))),
                        target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                            "a".to_string(),
                        )))),
                        transitive: true,
                        max_depth: None,
                    }),
                }),
        )
        .await
        .expect("relationship and indexed scalar query should succeed");
    assert_eq!(row_ids(&relation_and_scalar), vec!["c"]);

    let relation = |id: &str, target: &str, transitive| Expr::RelationExists {
        relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
            id.to_string(),
        )))),
        source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
            "id",
        ])))),
        target: Box::new(Expr::Operand(Operand::Literal(Value::String(
            target.to_string(),
        )))),
        transitive,
        max_depth: None,
    };
    let two_relationships = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(Expr::Binary {
                    op: BinaryOp::And,
                    left: Box::new(relation("shared.rel.parent", "a", true)),
                    right: Box::new(relation("shared.rel.secondary", "d", false)),
                }),
        )
        .await
        .expect("two relationship predicates should compose");
    assert_eq!(row_ids(&two_relationships), vec!["c"]);

    let max_depth_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(Expr::RelationExists {
                    relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "shared.rel.parent".to_string(),
                    )))),
                    source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "id",
                    ])))),
                    target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "a".to_string(),
                    )))),
                    transitive: true,
                    max_depth: Some(Box::new(Expr::Operand(Operand::Literal(Value::U64(1))))),
                }),
        )
        .await
        .expect("bounded transitive relationship query should succeed");
    assert_eq!(row_ids(&max_depth_rows), vec!["b"]);

    let row_dependent_max_depth = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(Expr::RelationExists {
                    relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "shared.rel.parent".to_string(),
                    )))),
                    source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "id",
                    ])))),
                    target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "a".to_string(),
                    )))),
                    transitive: true,
                    max_depth: Some(Box::new(Expr::Operand(Operand::Field(
                        FieldPath::from_fields(["depth"]),
                    )))),
                })
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    direction: SortDirection::Asc,
                }]),
        )
        .await
        .expect("row-dependent relationship max depth should use normal evaluation");
    assert_eq!(row_ids(&row_dependent_max_depth), vec!["b", "c"]);

    for invalid_max_depth in [Value::Null, Value::String("1".to_string()), Value::I64(-1)] {
        let result = db
            .select(
                SelectQuery::new()
                    .with_collection("shared_suite_rel_nodes")
                    .with_predicate(Expr::RelationExists {
                        relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                            "shared.rel.parent".to_string(),
                        )))),
                        source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "id",
                        ])))),
                        target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                            "a".to_string(),
                        )))),
                        transitive: true,
                        max_depth: Some(Box::new(Expr::Operand(Operand::Literal(
                            invalid_max_depth,
                        )))),
                    }),
            )
            .await;
        assert!(
            result.is_err(),
            "invalid max_depth should preserve its error"
        );
    }

    let direct_rows = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(Expr::RelationExists {
                    relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "shared.rel.parent".to_string(),
                    )))),
                    source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "id",
                    ])))),
                    target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "a".to_string(),
                    )))),
                    transitive: false,
                    max_depth: None,
                }),
        )
        .await
        .expect("direct relation query should succeed");
    assert_eq!(row_ids(&direct_rows), vec!["b"]);

    let update_stats = db
        .update_where(
            UpdateQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(eq_predicate(
                    FieldPath::from_fields(["id"]),
                    Value::String("c".to_string()),
                ))
                .set(
                    FieldPath::from_fields(["shared.rel.parent_ref"]),
                    Expr::Operand(Operand::Literal(Value::Null)),
                ),
        )
        .await
        .expect("relationship update should succeed");
    assert_eq!(update_stats.affected, 1);

    let after_update = db
        .select(
            SelectQuery::new()
                .with_collection("shared_suite_rel_nodes")
                .with_predicate(Expr::RelationExists {
                    relation: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "shared.rel.parent".to_string(),
                    )))),
                    source: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "id",
                    ])))),
                    target: Box::new(Expr::Operand(Operand::Literal(Value::String(
                        "a".to_string(),
                    )))),
                    transitive: true,
                    max_depth: None,
                })
                .with_order_by(vec![OrderBy {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    direction: SortDirection::Asc,
                }]),
        )
        .await
        .expect("relationship query after update should succeed");
    assert_eq!(row_ids(&after_update), vec!["b"]);

    if db
        .supported_text_query_formats()
        .contains(&TextQueryFormat::Sql)
    {
        let sql = db
            .query_text(
                TextQueryFormat::Sql,
                "SELECT id FROM shared_suite_rel_nodes WHERE has_relation_path('shared.rel.parent', id, 'a') ORDER BY id",
            )
            .await
            .expect("sql relationship function query should succeed");
        let QueryResult::Select(sql_rows) = sql else {
            panic!("sql relationship function query should return SELECT rows");
        };
        assert_eq!(row_ids(&sql_rows), vec!["b"]);
    }
}

async fn test_relationships_generic_external(db: &Db) {
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared.rel.weight".to_string(),
                    name: "weight".to_string(),
                    ty: Type {
                        kind: TypeKind::String(StringType {
                            format: None,
                            normalization: None,
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    },
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: "shared.rel.weighted".to_string(),
                    name: "WeightedRelation".to_string(),
                    inherits: Some(semantic_data::schema::ClassRef {
                        id: RELATION_CLASS_ID.to_string(),
                    }),
                    extends: vec![],
                    attributes: std::collections::BTreeMap::from([(
                        "weight".to_string(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: "shared.rel.weight".to_string(),
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
            })
            .with_op(DdlOperation::UpsertCollection {
                name: "shared_suite_rel_docs".to_string(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            })
            .with_op(DdlOperation::UpsertRelationship {
                relationship: RelationType {
                    id: "shared.rel.weighted".to_string(),
                    name: "weighted".to_string(),
                    source_collection: "shared_suite_rel_docs".to_string(),
                    mode: RelationMode::External,
                    indexing_mode: RelationIndexingMode::Enabled,
                    meta: Meta::default(),
                },
            }),
    )
    .await
    .expect("external relationship ddl should succeed");

    db.create_collection("shared_suite_rel_docs_nodes", CollectionKind::Polymorphic)
        .await
        .expect("doc relationship node collection creation should succeed");
    for id in ["x", "y"] {
        let mut node = Object::new();
        node.insert("id", Value::String(id.to_string()));
        db.insert("shared_suite_rel_docs_nodes", id, node)
            .await
            .expect("node insert for document relationship should succeed");
    }

    let mut rel = Object::new();
    rel.insert("id", Value::String("r1".to_string()));
    rel.insert("type", Value::String("shared.rel.weighted".to_string()));
    rel.insert("from", Value::String("x".to_string()));
    rel.insert("to", Value::String("y".to_string()));
    rel.insert("weight", Value::String("5".to_string()));
    db.insert("shared_suite_rel_docs", "r1", rel)
        .await
        .expect("document relation row insert should succeed");

    let edge_rows = db
        .select(
            SelectQuery::new()
                .with_collection("__semantic.relationship_edges")
                .with_predicate(and_all(vec![
                    eq_predicate(
                        FieldPath::from_fields(["relation"]),
                        Value::String("shared.rel.weighted".to_string()),
                    ),
                    eq_predicate(
                        FieldPath::from_fields(["source"]),
                        Value::String("x".to_string()),
                    ),
                    eq_predicate(
                        FieldPath::from_fields(["target"]),
                        Value::String("y".to_string()),
                    ),
                ])),
        )
        .await
        .expect("document relationship edge materialization query should succeed");
    assert_eq!(edge_rows.len(), 1);

    let doc_id_edge_rows = db
        .select(
            SelectQuery::new()
                .with_collection("__semantic.relationship_edges")
                .with_predicate(and_all(vec![
                    eq_predicate(
                        FieldPath::from_fields(["relation"]),
                        Value::String("shared.rel.weighted".to_string()),
                    ),
                    eq_predicate(
                        FieldPath::from_fields(["source"]),
                        Value::String("r1".to_string()),
                    ),
                    eq_predicate(
                        FieldPath::from_fields(["target"]),
                        Value::String("y".to_string()),
                    ),
                ])),
        )
        .await
        .expect("document id relationship edge query should succeed");
    assert!(doc_id_edge_rows.is_empty());

    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared.directory_node.from".to_string(),
                    name: "from".to_string(),
                    ty: Type::new(TypeKind::Ref(TypeRef::new("shared.directory"))),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: "shared.directory_node.order".to_string(),
                    name: "order".to_string(),
                    ty: Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64))),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: "shared.directory".to_string(),
                    name: "Directory".to_string(),
                    inherits: None,
                    extends: vec![],
                    attributes: BTreeMap::new(),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertClass {
                class: ClassType {
                    id: "shared.directory_node".to_string(),
                    name: "DirectoryNode".to_string(),
                    inherits: Some(semantic_data::schema::ClassRef {
                        id: RELATION_CLASS_ID.to_string(),
                    }),
                    extends: vec![],
                    attributes: BTreeMap::from([
                        (
                            "from".to_string(),
                            ClassAttribute {
                                attribute: AttributeRef {
                                    id: "shared.directory_node.from".to_string(),
                                },
                                required: true,
                                ui_order: None,
                                computed: None,
                                constraints: vec![],
                                meta: Meta::default(),
                            },
                        ),
                        (
                            "order".to_string(),
                            ClassAttribute {
                                attribute: AttributeRef {
                                    id: "shared.directory_node.order".to_string(),
                                },
                                required: false,
                                ui_order: None,
                                computed: None,
                                constraints: vec![],
                                meta: Meta::default(),
                            },
                        ),
                    ]),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            })
            .with_op(DdlOperation::UpsertCollection {
                name: "shared_suite_directory_nodes".to_string(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            })
            .with_op(DdlOperation::UpsertRelationship {
                relationship: RelationType {
                    id: "shared.directory_node".to_string(),
                    name: "directory_node".to_string(),
                    source_collection: "shared_suite_directory_nodes".to_string(),
                    mode: RelationMode::External,
                    indexing_mode: RelationIndexingMode::Enabled,
                    meta: Meta::default(),
                },
            }),
    )
    .await
    .expect("shadowed directory node relationship ddl should succeed");

    let mut parent_directory = Object::new();
    parent_directory.insert("id", Value::String("parent-dir".to_string()));
    parent_directory.insert("type", Value::String("shared.directory".to_string()));
    db.insert(
        "shared_suite_directory_nodes",
        "parent-dir",
        parent_directory,
    )
    .await
    .expect("parent directory row insert should succeed");

    let mut child_item = Object::new();
    child_item.insert("id", Value::String("child-item".to_string()));
    db.insert("shared_suite_directory_nodes", "child-item", child_item)
        .await
        .expect("child item row insert should succeed");

    let mut directory_node = Object::new();
    directory_node.insert("id", Value::String("dir-rel-1".to_string()));
    directory_node.insert("type", Value::String("shared.directory_node".to_string()));
    directory_node.insert("from", Value::String("parent-dir".to_string()));
    directory_node.insert("to", Value::String("child-item".to_string()));
    directory_node.insert("order", Value::U64(10));
    db.insert("shared_suite_directory_nodes", "dir-rel-1", directory_node)
        .await
        .expect("directory node relation row insert should succeed");

    let directory_edge_rows = db
        .select(
            SelectQuery::new()
                .with_collection("__semantic.relationship_edges")
                .with_predicate(and_all(vec![
                    eq_predicate(
                        FieldPath::from_fields(["relation"]),
                        Value::String("shared.directory_node".to_string()),
                    ),
                    eq_predicate(
                        FieldPath::from_fields(["source"]),
                        Value::String("parent-dir".to_string()),
                    ),
                    eq_predicate(
                        FieldPath::from_fields(["target"]),
                        Value::String("child-item".to_string()),
                    ),
                ])),
        )
        .await
        .expect("directory node relationship edge query should succeed");
    assert_eq!(directory_edge_rows.len(), 1);
}

fn blog_package_v1() -> Package {
    let title_attr = blog_string_attribute("shared.blog.title", "title");
    let post_class = blog_post_class(false);
    let mut seed = Object::new();
    seed.insert("id", Value::String("seed".to_string()));
    seed.insert("type", Value::String("shared.blog.post".to_string()));
    seed.insert("title", Value::String("Hello".to_string()));

    Package {
        name: "shared.blog".to_string(),
        root: Module {
            name: "blog".to_string(),
            constants: BTreeMap::new(),
            types: BTreeMap::from([(
                "shared.blog.status".to_string(),
                TypeDef {
                    name: "shared.blog.status".to_string(),
                    module: Some("blog".to_string()),
                    params: Vec::new(),
                    ty: Type {
                        kind: TypeKind::String(StringType {
                            format: None,
                            normalization: None,
                        }),
                        constraints: vec![],
                        annotations: vec![],
                    },
                    visibility: Visibility::Public,
                    meta: Meta::default(),
                },
            )]),
            attributes: BTreeMap::from([(title_attr.id.clone(), title_attr.clone())]),
            classes: BTreeMap::from([(post_class.id.clone(), post_class.clone())]),
            interfaces: BTreeMap::new(),
            contracts: BTreeMap::new(),
            meta: Meta::default(),
        },
        modules: BTreeMap::new(),
        migrations: vec![Migration {
            module: "blog".to_string(),
            name: "001_init".to_string(),
            description: Some("Create the initial blog schema.".to_string()),
            operations: vec![
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                    attribute: title_attr,
                }),
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertTypeDef {
                    type_def: TypeDef {
                        name: "shared.blog.status".to_string(),
                        module: Some("blog".to_string()),
                        params: Vec::new(),
                        ty: Type {
                            kind: TypeKind::String(StringType {
                                format: None,
                                normalization: None,
                            }),
                            constraints: vec![],
                            annotations: vec![],
                        },
                        visibility: Visibility::Public,
                        meta: Meta::default(),
                    },
                }),
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class: post_class }),
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
                    name: "shared_suite_blog_posts".to_string(),
                    kind: MigrationCollectionKind::Polymorphic,
                    integrity_mode: MigrationIntegrityMode::StrictRegisteredSchema,
                }),
                MigrationOperation::Insert {
                    collection: "shared_suite_blog_posts".to_string(),
                    id: "seed".to_string(),
                    object: seed,
                },
            ],
            meta: Meta::default(),
        }],
        version: None,
        meta: Meta::default(),
    }
}

fn blog_package_v2() -> Package {
    let mut package = blog_package_v1();
    let body_attr = blog_string_attribute("shared.blog.body", "body");
    let post_class = blog_post_class(true);
    package
        .root
        .attributes
        .insert(body_attr.id.clone(), body_attr.clone());
    package
        .root
        .classes
        .insert(post_class.id.clone(), post_class.clone());
    package.migrations.push(Migration {
        module: "blog".to_string(),
        name: "002_body".to_string(),
        description: Some("Extend blog posts with a body field.".to_string()),
        operations: vec![
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                attribute: body_attr,
            }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class: post_class }),
            MigrationOperation::Update {
                query: UpdateQuery::new()
                    .with_collection("shared_suite_blog_posts")
                    .with_predicate(eq_predicate(
                        FieldPath::from_fields(["id"]),
                        Value::String("seed".to_string()),
                    ))
                    .set(
                        FieldPath::from_fields(["body"]),
                        Expr::Operand(Operand::Literal(Value::String("World".to_string()))),
                    ),
            },
        ],
        meta: Meta::default(),
    });
    package
}

fn blog_string_attribute(id: &str, name: &str) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty: Type {
            kind: TypeKind::String(StringType {
                format: None,
                normalization: None,
            }),
            constraints: vec![],
            annotations: vec![],
        },
        constraints: vec![],
        meta: Meta::default(),
    }
}

fn blog_post_class(include_body: bool) -> ClassType {
    let mut attributes = BTreeMap::from([(
        "title".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: "shared.blog.title".to_string(),
            },
            required: true,
            ui_order: None,
            computed: None,
            constraints: vec![],
            meta: Meta::default(),
        },
    )]);
    if include_body {
        attributes.insert(
            "body".to_string(),
            ClassAttribute {
                attribute: AttributeRef {
                    id: "shared.blog.body".to_string(),
                },
                required: false,
                ui_order: None,
                computed: None,
                constraints: vec![],
                meta: Meta::default(),
            },
        );
    }
    ClassType {
        id: "shared.blog.post".to_string(),
        name: "BlogPost".to_string(),
        inherits: None,
        extends: vec![],
        attributes,
        constraints: vec![],
        meta: Meta::default(),
    }
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

fn eq_predicate(path: FieldPath, value: Value) -> Expr {
    eq_predicate_exprs(
        Expr::Operand(Operand::Field(path)),
        Expr::Operand(Operand::Literal(value)),
    )
}

fn eq_predicate_exprs(left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op: BinaryOp::Eq,
        left: Box::new(left),
        right: Box::new(right),
    }
}

fn and_all(mut items: Vec<Expr>) -> Expr {
    let first = items.remove(0);
    items.into_iter().fold(first, |left, right| Expr::Binary {
        op: BinaryOp::And,
        left: Box::new(left),
        right: Box::new(right),
    })
}
