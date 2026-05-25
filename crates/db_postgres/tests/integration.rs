use deadpool_postgres::{Manager, Pool};
use semantic_data::query::SelectQuery;
use semantic_data::value::{Object, Value};
use semantic_db_core::catalog::{CollectionKind, OBJECT_TYPE_FIELD};
use semantic_db_core::{AccessPath, Db};
use semantic_db_postgres::{PostgresBackend, PostgresMode};

/// Get a connection pool from the `POSTGRES_URI` env var.
fn get_pool() -> Pool {
    let uri =
        std::env::var("POSTGRES_URI").expect("POSTGRES_URI env var required for postgres tests");
    let config: tokio_postgres::Config = uri.parse().expect("invalid POSTGRES_URI");
    let mgr = Manager::new(config, tokio_postgres::NoTls);
    Pool::builder(mgr).max_size(4).build().expect("pool build")
}

// ---------------------------------------------------------------------------
// Test 1: Discovery — single PK table
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_discovery_single_pk() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();

    let _ = client
        .execute("DROP TABLE IF EXISTS test_items CASCADE", &[])
        .await;
    client
        .execute(
            "CREATE TABLE test_items (id SERIAL PRIMARY KEY, name TEXT NOT NULL, score INTEGER)",
            &[],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO test_items (name, score) VALUES ($1, $2), ($3, $4)",
            &[&"hello", &42i32, &"world", &99i32],
        )
        .await
        .unwrap();

    let backend = PostgresBackend::new(
        pool,
        PostgresMode::Discovery {
            schemas: vec!["public".into()],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    // SELECT all
    let results = db
        .select(SelectQuery::new().with_collection("postgres:public:test_items"))
        .await
        .unwrap();

    assert_eq!(results.len(), 2, "should return 2 rows");

    // Computed id present
    for obj in &results {
        let id = obj.get("id").and_then(|v| v.as_str()).unwrap();
        assert!(
            id.starts_with("test_items-"),
            "id should start with table name, got: {id}"
        );
        // type field should be injected
        let type_val = obj.get(OBJECT_TYPE_FIELD).and_then(|v| v.as_str()).unwrap();
        assert_eq!(type_val, "postgres:public:test_items");
    }

    // GET by id
    let first_id = results[0]
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    let entity = db
        .get("postgres:public:test_items", &first_id)
        .await
        .unwrap();
    assert!(entity.is_some(), "get() should find entity by synthetic id");
    let entity = entity.unwrap();
    assert_eq!(entity.id, first_id);
    assert_eq!(entity.collection, "postgres:public:test_items");

    // Cleanup
    let _ = client
        .execute("DROP TABLE IF EXISTS test_items CASCADE", &[])
        .await;
}

// ---------------------------------------------------------------------------
// Test 2: Discovery — composite PK
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_discovery_composite_pk() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();

    let _ = client
        .execute("DROP TABLE IF EXISTS composite_test CASCADE", &[])
        .await;
    client
        .execute(
            "CREATE TABLE composite_test (a INT, b INT, val TEXT, PRIMARY KEY (a, b))",
            &[],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO composite_test (a, b, val) VALUES (1, 2, 'data')",
            &[],
        )
        .await
        .unwrap();

    let backend = PostgresBackend::new(
        pool,
        PostgresMode::Discovery {
            schemas: vec!["public".into()],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    let results = db
        .select(SelectQuery::new().with_collection("postgres:public:composite_test"))
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    let id = results[0].get("id").and_then(|v| v.as_str()).unwrap();
    assert_eq!(id, "composite_test-1::2");

    // get() by synthetic ID
    let entity = db.get("postgres:public:composite_test", id).await.unwrap();
    assert!(entity.is_some());

    // Cleanup
    let _ = client
        .execute("DROP TABLE IF EXISTS composite_test CASCADE", &[])
        .await;
}

// ---------------------------------------------------------------------------
// Test 3: DDL forbidden in discovery
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_ddl_forbidden_in_discovery() {
    let pool = get_pool();
    let backend = PostgresBackend::new(
        pool,
        PostgresMode::Discovery {
            schemas: vec!["public".into()],
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
}

// ---------------------------------------------------------------------------
// Test 4: INSERT rejected in discovery
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_insert_rejected_in_discovery() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();

    let _ = client
        .execute("DROP TABLE IF EXISTS test_items CASCADE", &[])
        .await;
    client
        .execute(
            "CREATE TABLE test_items (id SERIAL PRIMARY KEY, name TEXT)",
            &[],
        )
        .await
        .unwrap();
    client
        .execute("INSERT INTO test_items (name) VALUES ('seed')", &[])
        .await
        .unwrap();

    let backend = PostgresBackend::new(
        pool,
        PostgresMode::Discovery {
            schemas: vec!["public".into()],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    let mut obj = Object::new();
    obj.insert("name".to_string(), Value::String("should-fail".into()));
    let err = db
        .insert("postgres:public:test_items", "ignored", obj)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("read-only"),
        "expected read-only rejection, got: {err}"
    );

    // Cleanup
    let _ = client
        .execute("DROP TABLE IF EXISTS test_items CASCADE", &[])
        .await;
}

// ---------------------------------------------------------------------------
// Test 5: explain() and plan() stub
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_explain_and_plan_stub() {
    let pool = get_pool();
    let client = pool.get().await.unwrap();

    let _ = client
        .execute("DROP TABLE IF EXISTS test_items CASCADE", &[])
        .await;
    client
        .execute(
            "CREATE TABLE test_items (id SERIAL PRIMARY KEY, name TEXT)",
            &[],
        )
        .await
        .unwrap();

    let backend = PostgresBackend::new(
        pool,
        PostgresMode::Discovery {
            schemas: vec!["public".into()],
        },
    )
    .await
    .unwrap();
    let db = Db::new(backend);

    // explain should return FullScan
    let explain = db
        .explain(SelectQuery::new().with_collection("postgres:public:test_items"))
        .await;
    assert!(explain.is_ok(), "explain() should succeed: {:?}", explain);
    if let Ok(qe) = explain {
        assert_eq!(qe.access_path, AccessPath::FullScan);
    }

    // plan() should be stubbed with an error
    let plan = db
        .plan(SelectQuery::new().with_collection("postgres:public:test_items"))
        .await;
    assert!(
        plan.is_err(),
        "plan() is stubbed and should return an error"
    );

    // Cleanup
    let _ = client
        .execute("DROP TABLE IF EXISTS test_items CASCADE", &[])
        .await;
}
