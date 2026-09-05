use deadpool_postgres::{Manager, Pool};
use semantic_data::query::SelectQuery;
use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, Meta, StringType, Type, TypeKind,
};
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::{CollectionKind, IntegrityMode, OBJECT_TYPE_FIELD};
use semantic_db_core::{AccessPath, Db, DdlBatch, DdlCollectionKind, DdlOperation};
use semantic_db_postgres::{PostgresBackend, PostgresBackendOptions, PostgresMode, quote_ident};

static TEST_SCHEMA_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Get a connection pool from the `POSTGRES_URI` env var.
fn get_pool() -> Option<Pool> {
    let uri = std::env::var("POSTGRES_URI").ok()?;
    let config: tokio_postgres::Config = uri.parse().expect("invalid POSTGRES_URI");
    let mgr = Manager::new(config, tokio_postgres::NoTls);
    Some(Pool::builder(mgr).max_size(4).build().expect("pool build"))
}

fn unique_schema(prefix: &str) -> String {
    let counter = TEST_SCHEMA_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_nanos();
    format!("{prefix}_{}_{}_{}", std::process::id(), nonce, counter)
}

// ---------------------------------------------------------------------------
// Test 1: Discovery — single PK table
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_discovery_single_pk() {
    let Some(pool) = get_pool() else { return };
    let client = pool.get().await.unwrap();
    let schema = unique_schema("discovery_single");
    let quoted_schema = quote_ident(&schema);
    client
        .batch_execute(&format!(
            "CREATE SCHEMA {quoted_schema};
             CREATE TABLE {quoted_schema}.test_items
                 (id SERIAL PRIMARY KEY, name TEXT NOT NULL, score INTEGER)"
        ))
        .await
        .unwrap();
    client
        .execute(
            &format!(
                "INSERT INTO {quoted_schema}.test_items (name, score)
                 VALUES ($1, $2), ($3, $4)"
            ),
            &[&"hello", &42i32, &"world", &99i32],
        )
        .await
        .unwrap();

    let collection = format!("postgres:{schema}:test_items");

    let backend = PostgresBackend::new(
        pool.clone(),
        PostgresMode::Discovery {
            schemas: vec![schema.clone()],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    // SELECT all
    let results = db
        .select(SelectQuery::new().with_collection(&collection))
        .await
        .unwrap();

    assert_eq!(results.len(), 2, "should return 2 rows");

    // Computed id present
    for obj in &results {
        let id = obj.get("id").and_then(|v| v.as_str()).unwrap();
        assert!(
            id.starts_with("pg1."),
            "id should use the versioned PostgreSQL tuple format, got: {id}"
        );
        // type field should be injected
        let type_val = obj.get(OBJECT_TYPE_FIELD).and_then(|v| v.as_str()).unwrap();
        assert_eq!(type_val, collection);
    }

    // GET by id
    let first_id = results[0]
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let entity = db.get(collection.as_str(), &first_id).await.unwrap();
    assert!(entity.is_some(), "get() should find entity by synthetic id");
    let entity = entity.unwrap();
    assert_eq!(entity.id, first_id);
    assert_eq!(entity.collection, collection);

    drop(db);
    client
        .batch_execute(&format!("DROP SCHEMA {quoted_schema} CASCADE"))
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Test 2: Discovery — composite PK
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_discovery_composite_pk() {
    let Some(pool) = get_pool() else { return };
    let client = pool.get().await.unwrap();
    let schema = unique_schema("discovery_composite");
    let quoted_schema = quote_ident(&schema);
    client
        .batch_execute(&format!(
            "CREATE SCHEMA {quoted_schema};
             CREATE TABLE {quoted_schema}.composite_test
                 (a INT, b INT, val TEXT, PRIMARY KEY (a, b))"
        ))
        .await
        .unwrap();
    client
        .execute(
            &format!(
                "INSERT INTO {quoted_schema}.composite_test (a, b, val)
                 VALUES (1, 2, 'data')"
            ),
            &[],
        )
        .await
        .unwrap();

    let collection = format!("postgres:{schema}:composite_test");

    let backend = PostgresBackend::new(
        pool.clone(),
        PostgresMode::Discovery {
            schemas: vec![schema.clone()],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    let results = db
        .select(SelectQuery::new().with_collection(&collection))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    let id = results[0].get("id").and_then(|v| v.as_str()).unwrap();
    assert!(id.starts_with("pg1."));

    // get() by synthetic ID
    let entity = db.get(collection.as_str(), id).await.unwrap();
    assert!(entity.is_some());

    drop(db);
    client
        .batch_execute(&format!("DROP SCHEMA {quoted_schema} CASCADE"))
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Test 3: DDL forbidden in discovery
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_ddl_forbidden_in_discovery() {
    let Some(pool) = get_pool() else { return };
    let client = pool.get().await.unwrap();
    let schema = unique_schema("discovery_ddl");
    let quoted_schema = quote_ident(&schema);
    client
        .batch_execute(&format!("CREATE SCHEMA {quoted_schema}"))
        .await
        .unwrap();
    let backend = PostgresBackend::new(
        pool.clone(),
        PostgresMode::Discovery {
            schemas: vec![schema],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    let err = db
        .create_collection("test", CollectionKind::Schema)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("read-only") || msg.contains("forbidden"),
        "expected write rejection, got: {err}"
    );
    drop(db);
    client
        .batch_execute(&format!("DROP SCHEMA {quoted_schema} CASCADE"))
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Test 4: INSERT rejected in discovery
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_insert_rejected_in_discovery() {
    let Some(pool) = get_pool() else { return };
    let client = pool.get().await.unwrap();
    let schema = unique_schema("discovery_insert");
    let quoted_schema = quote_ident(&schema);
    client
        .batch_execute(&format!(
            "CREATE SCHEMA {quoted_schema};
             CREATE TABLE {quoted_schema}.test_items (id SERIAL PRIMARY KEY, name TEXT)"
        ))
        .await
        .unwrap();
    client
        .execute(
            &format!("INSERT INTO {quoted_schema}.test_items (name) VALUES ('seed')"),
            &[],
        )
        .await
        .unwrap();

    let collection = format!("postgres:{schema}:test_items");

    let backend = PostgresBackend::new(
        pool.clone(),
        PostgresMode::Discovery {
            schemas: vec![schema],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    let mut obj = Object::new();
    obj.insert("name".to_string(), Value::String("should-fail".into()));
    let err = db
        .insert(collection.as_str(), "ignored", obj)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("read-only"),
        "expected read-only rejection, got: {err}"
    );

    drop(db);
    client
        .batch_execute(&format!("DROP SCHEMA {quoted_schema} CASCADE"))
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Test 5: explain() and plan() use the supplied query
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_explain_and_plan_stub() {
    let Some(pool) = get_pool() else { return };
    let client = pool.get().await.unwrap();
    let schema = unique_schema("discovery_plan");
    let quoted_schema = quote_ident(&schema);
    client
        .batch_execute(&format!(
            "CREATE SCHEMA {quoted_schema};
             CREATE TABLE {quoted_schema}.test_items (id SERIAL PRIMARY KEY, name TEXT)"
        ))
        .await
        .unwrap();

    let collection = format!("postgres:{schema}:test_items");

    let backend = PostgresBackend::new(
        pool.clone(),
        PostgresMode::Discovery {
            schemas: vec![schema],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    // explain should return FullScan
    let explain = db
        .explain(SelectQuery::new().with_collection(&collection))
        .await;
    assert!(explain.is_ok(), "explain() should succeed: {:?}", explain);
    if let Ok(qe) = explain {
        assert_eq!(qe.access_path, AccessPath::FullScan);
    }

    // plan() should use the real semantic planner.
    let plan = db
        .plan(SelectQuery::new().with_collection(&collection))
        .await;
    assert!(plan.is_ok(), "plan() should succeed: {plan:?}");

    drop(db);
    client
        .batch_execute(&format!("DROP SCHEMA {quoted_schema} CASCADE"))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_discovery_native_metadata_fk_enum_domain_and_typed_get() {
    let Some(pool) = get_pool() else { return };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("discovery_meta_{}_{}", std::process::id(), nonce);
    let client = pool.get().await.unwrap();
    client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema};
             CREATE TYPE {schema}.mood AS ENUM ('happy', 'calm');
             CREATE DOMAIN {schema}.positive_int AS integer CHECK (VALUE > 0);
             CREATE TABLE {schema}.parents (
                 id uuid PRIMARY KEY,
                 mood {schema}.mood NOT NULL,
                 score {schema}.positive_int NOT NULL,
                 doubled integer GENERATED ALWAYS AS (score * 2) STORED
             );
             CREATE TABLE {schema}.children (
                 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                 parent_id uuid NOT NULL REFERENCES {schema}.parents(id),
                 note text NOT NULL
             );
             CREATE INDEX children_note_idx ON {schema}.children(note);
             INSERT INTO {schema}.parents(id, mood, score)
                 VALUES ('00000000-0000-0000-0000-000000000042', 'happy', 7);
             INSERT INTO {schema}.children(parent_id, note)
                 VALUES ('00000000-0000-0000-0000-000000000042', 'child');",
            schema = quote_ident(&schema)
        ))
        .await
        .unwrap();

    let db = Db::new(
        PostgresBackend::new_with_options(
            pool.clone(),
            PostgresBackendOptions::relational_discovery(vec![schema.clone()]),
        )
        .await
        .unwrap(),
    );
    let catalog = db.catalog().await.unwrap();
    let parents = format!("postgres:{schema}:parents");
    let children = format!("postgres:{schema}:children");
    assert!(
        catalog
            .relationship_by_id(&format!(
                "postgres:fk:{schema}:children:children_parent_id_fkey"
            ))
            .is_some()
    );
    let generated = catalog
        .attribute_by_id(&format!("{parents}:doubled"))
        .unwrap();
    assert_eq!(
        generated
            .attribute
            .meta
            .annotations
            .get("postgres.generated")
            .map(String::as_str),
        Some("true")
    );
    let identity = catalog.attribute_by_id(&format!("{children}:id")).unwrap();
    assert_eq!(
        identity
            .attribute
            .meta
            .annotations
            .get("postgres.identity")
            .map(String::as_str),
        Some("true")
    );
    let child_collection = catalog.collection_by_name(&children).unwrap();
    assert!(
        catalog
            .indexes_for_collection(child_collection.lid)
            .any(|index| { index.schema.id.contains("children_note_idx") && !index.schema.unique })
    );
    drop(catalog);

    let parent_rows = db
        .select(SelectQuery::new().with_collection(parents.as_str()))
        .await
        .unwrap();
    assert_eq!(
        parent_rows[0].get("mood"),
        Some(&Value::String("happy".into()))
    );
    assert_eq!(parent_rows[0].get("score"), Some(&Value::I32(7)));
    let parent_id = parent_rows[0].get("id").unwrap().as_str().unwrap();
    assert!(db.get(parents.as_str(), parent_id).await.unwrap().is_some());

    let child_rows = db
        .select(SelectQuery::new().with_collection(children.as_str()))
        .await
        .unwrap();
    let foreign_id = child_rows[0]
        .get("parent_id")
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(foreign_id, parent_id);
    let child_id = child_rows[0].get("id").unwrap().as_str().unwrap();
    assert!(db.get(children.as_str(), child_id).await.unwrap().is_some());

    drop(db);
    client
        .batch_execute(&format!("DROP SCHEMA {} CASCADE", quote_ident(&schema)))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_semantic_managed_backend_suite() {
    let Some(pool) = get_pool() else { return };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("semantic_test_{}_{}", std::process::id(), nonce);
    let options = PostgresBackendOptions::semantic_managed().with_metadata_schema(&schema);
    let backend = PostgresBackend::new_with_options(pool.clone(), options.clone())
        .await
        .unwrap();
    let db = Db::new(backend);

    semantic_db_test::suite::test_db(&db).await;
    drop(db);

    let client = pool.get().await.unwrap();
    let entity_count: i64 = client
        .query_one(
            &format!(
                "SELECT count(*) FROM {}.entities WHERE document->>'format' = '1'",
                quote_ident(&schema)
            ),
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        entity_count > 0,
        "tagged entity documents should be persisted"
    );

    let reopened = Db::new(
        PostgresBackend::new_with_options(pool.clone(), options)
            .await
            .expect("managed backend should reopen"),
    );
    assert!(
        reopened
            .catalog()
            .await
            .unwrap()
            .collection_by_name("shared_suite_schema")
            .is_some()
    );
    drop(reopened);

    client
        .batch_execute(&format!("DROP SCHEMA {} CASCADE", quote_ident(&schema)))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_relational_managed_strict_projection_and_reopen() {
    let Some(pool) = get_pool() else { return };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("relational_test_{}_{}", std::process::id(), nonce);
    let options = PostgresBackendOptions::relational_managed(&schema);
    let db = Db::new(
        PostgresBackend::new_with_options(pool.clone(), options.clone())
            .await
            .expect("relational managed backend should construct"),
    );

    let error = db
        .create_collection("invalid_polymorphic", CollectionKind::Polymorphic)
        .await
        .expect_err("managed relational mode must reject polymorphic collections");
    assert!(error.to_string().contains("polymorphic"), "{error}");

    let attribute_id = "test:relational:title";
    let class_id = "test:relational:article";
    db.execute_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertCollection {
                name: class_id.to_string(),
                kind: DdlCollectionKind::Schema,
                integrity_mode: IntegrityMode::StrictRegisteredSchema,
            })
            .with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: attribute_id.to_string(),
                    name: "title".to_string(),
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
                    id: class_id.to_string(),
                    name: "Article".to_string(),
                    inherits: None,
                    extends: vec![],
                    strict_schema: false,
                    attributes: std::collections::BTreeMap::from([(
                        "title".to_string(),
                        ClassAttribute {
                            attribute: AttributeRef {
                                id: attribute_id.to_string(),
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
            }),
    )
    .await
    .expect("valid strict class and collection should be created atomically");

    let mut object = Object::new();
    object.insert("id".to_string(), Value::String("article-1".to_string()));
    object.insert("type".to_string(), Value::String(class_id.to_string()));
    object.insert(
        attribute_id.to_string(),
        Value::String("Relational".to_string()),
    );
    db.insert(class_id, "article-1", object)
        .await
        .expect("typed entity insert should succeed");
    assert!(db.get(class_id, "article-1").await.unwrap().is_some());

    let client = pool.get().await.unwrap();
    let row = client
        .query_one(
            &format!(
                "SELECT cm.physical_table, fm.physical_column, fm.physical_type
                 FROM {}.collection_map cm JOIN {}.field_map fm USING (collection_lid)
                 WHERE cm.collection_name = $1 AND fm.canonical_field = $2",
                quote_ident(&schema),
                quote_ident(&schema)
            ),
            &[&class_id, &attribute_id],
        )
        .await
        .unwrap();
    let table: String = row.get(0);
    let column: String = row.get(1);
    let physical_type: String = row.get(2);
    assert_eq!(physical_type, "text");
    let projected: String = client
        .query_one(
            &format!(
                "SELECT {} FROM {}.{} WHERE _semantic_id = $1",
                quote_ident(&column),
                quote_ident(&schema),
                quote_ident(&table)
            ),
            &[&"article-1"],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(projected, "Relational");

    client
        .execute(
            &format!(
                "UPDATE {}.{} SET {} = 'tampered' WHERE _semantic_id = $1",
                quote_ident(&schema),
                quote_ident(&table),
                quote_ident(&column)
            ),
            &[&"article-1"],
        )
        .await
        .unwrap();
    let drift = db.get(class_id, "article-1").await.unwrap_err();
    assert!(drift.to_string().contains("drift detected"), "{drift}");
    client
        .execute(
            &format!(
                "UPDATE {}.{} SET {} = 'Relational' WHERE _semantic_id = $1",
                quote_ident(&schema),
                quote_ident(&table),
                quote_ident(&column)
            ),
            &[&"article-1"],
        )
        .await
        .unwrap();

    drop(db);
    let reopened = Db::new(
        PostgresBackend::new_with_options(pool.clone(), options)
            .await
            .expect("relational managed backend should reopen"),
    );
    assert!(reopened.get(class_id, "article-1").await.unwrap().is_some());
    drop(reopened);
    client
        .batch_execute(&format!("DROP SCHEMA {} CASCADE", quote_ident(&schema)))
        .await
        .unwrap();
}
