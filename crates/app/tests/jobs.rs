use semantic_app::{DbScopeId, Principal, SemanticApp, SemanticDb, jobs::DbJobStore};
use semantic_data::{DateTime, jobs::*, schema::DbOpenMode};
use semantic_jobs::{JobContext, JobHandler, JobStore, JobsBuilder};
use std::{future::Future, pin::Pin, sync::Arc};

fn record(id: &str, now: DateTime) -> JobRecord {
    JobRecord {
        id: JobId(id.into()),
        kind: JobKindId("test.historical".into()),
        status: JobStatus::Queued,
        progress: JobProgress {
            completed: u64::MAX,
            total: Some(u64::MAX),
            ..Default::default()
        },
        error: None,
        created_at: now,
        updated_at: now,
        started_at: None,
        finished_at: None,
        snapshot_seq: 1,
    }
}
fn database(path: &std::path::Path, mode: DbOpenMode) -> Arc<dyn SemanticDb> {
    Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(path, mode).unwrap(),
    ))
}

#[tokio::test]
async fn incompatible_jobs_package_is_rejected_without_mutating_history_or_catalog() {
    for variant in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let db = database(&dir.path().join("jobs.redb"), DbOpenMode::AutoCreate);
        let store = DbJobStore::new(db.clone());
        store.initialize().await.unwrap();
        let original = record("preserved", DateTime::now_utc());
        store.put(&original).await.unwrap();
        let mut future = package();
        if variant == 0 {
            future.version = Some(semantic_data::schema::SchemaVersion {
                major: 2,
                minor: 0,
                patch: 0,
                pre: None,
                build: None,
            });
        } else if variant == 1 {
            let mut migration = future.migrations[0].clone();
            migration.name = "002_future".into();
            migration.operations.clear();
            future.migrations.push(migration);
        } else {
            future
                .root
                .classes
                .get_mut(CLASS_ID)
                .unwrap()
                .creatable_in_ui = Some(true);
            let mut migration = future.migrations[0].clone();
            migration.name = "002_changed_class".into();
            migration.operations = vec![semantic_data::schema::MigrationOperation::Ddl(
                semantic_data::schema::MigrationDdlOperation::UpsertClass {
                    class: future.root.classes[CLASS_ID].clone(),
                },
            )];
            future.migrations.push(migration);
        }
        db.upsert_package(future).await.unwrap();
        // Also exercise migration history rejection when a later writer has
        // replaced the catalog definition while leaving newer applied history.
        if variant == 1 {
            db.upsert_package(package()).await.unwrap();
        }
        let before = db.catalog().await.unwrap().to_storage_snapshot();
        assert!(store.initialize().await.is_err());
        assert_eq!(db.catalog().await.unwrap().to_storage_snapshot(), before);
        assert_eq!(store.get(&original.id).await.unwrap(), Some(original));
    }
}

#[tokio::test]
async fn postgres_store_cursor_wide_values_and_reconciliation() {
    let Ok(uri) = std::env::var("POSTGRES_URI") else {
        eprintln!("skipping PostgreSQL jobs parity: POSTGRES_URI is unset");
        return;
    };
    use semantic_db_postgres::{
        PostgresBackend, PostgresBackendOptions, PostgresConfig, PostgresMode,
    };
    let pool = semantic_db_postgres::create_pool(&PostgresConfig {
        uri,
        mode: PostgresMode::Managed,
        max_pool_size: Some(2),
    })
    .unwrap();
    let schema = format!("jobs_test_{}", uuid::Uuid::new_v4().simple());
    let backend = PostgresBackend::new_with_options(
        pool.clone(),
        PostgresBackendOptions::semantic_managed().with_metadata_schema(schema.clone()),
    )
    .await
    .unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(backend));
    let store = Arc::new(DbJobStore::new(db.clone()));
    store.initialize().await.unwrap();
    store.initialize().await.unwrap();
    let now = DateTime::now_utc();
    for id in ["c", "a", "b"] {
        store.put(&record(id, now)).await.unwrap();
    }
    assert_eq!(store.count().await.unwrap(), 3);
    let first = store
        .list(JobListQuery {
            oldest_first: true,
            limit: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        first
            .records
            .iter()
            .map(|r| r.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(first.records[0].progress.completed, u64::MAX);
    let next = store
        .list(JobListQuery {
            oldest_first: true,
            limit: 2,
            cursor: first.next_cursor,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(next.records[0].id.0, "c");
    assert!(next.next_cursor.is_none());
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION.into(), "a".into())
            .await
            .unwrap()
            .is_none()
    );
    let jobs = semantic_jobs::ScopeJobs::open(store, Default::default(), Default::default())
        .await
        .unwrap();
    assert_eq!(
        jobs.get(JobId("a".into())).await.unwrap().unwrap().status,
        JobStatus::Interrupted
    );
    assert_eq!(jobs.clear_completed().await.unwrap().deleted, 3);
    jobs.shutdown().await.unwrap();
    drop(jobs);
    drop(db);
    // This schema was created solely by this test using a random, quoted name.
    pool.get()
        .await
        .unwrap()
        .batch_execute(&format!(
            "DROP SCHEMA {} CASCADE",
            semantic_db_postgres::quote_ident(&schema)
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn redb_store_schema_cursor_replay_and_wide_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jobs.redb");
    let db = database(&path, DbOpenMode::AutoCreate);
    let store = DbJobStore::new(db.clone());
    store.initialize().await.unwrap();
    store.initialize().await.unwrap();
    let now = DateTime::now_utc();
    for id in ["c", "a", "b"] {
        store.put(&record(id, now)).await.unwrap();
    }
    assert_eq!(store.count().await.unwrap(), 3);
    let first = store
        .list(JobListQuery {
            oldest_first: true,
            limit: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        first
            .records
            .iter()
            .map(|r| r.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(first.records[0].progress.completed, u64::MAX);
    let next = store
        .list(JobListQuery {
            oldest_first: true,
            limit: 2,
            cursor: first.next_cursor,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(next.records[0].id.0, "c");
    assert!(next.next_cursor.is_none());
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION.into(), "a".into())
            .await
            .unwrap()
            .is_none()
    );
    drop(store);
    drop(db);
    let store = Arc::new(DbJobStore::new(database(&path, DbOpenMode::OpenExisting)));
    let jobs = semantic_jobs::ScopeJobs::open(store, Default::default(), Default::default())
        .await
        .unwrap();
    let interrupted = jobs.get(JobId("a".into())).await.unwrap().unwrap();
    assert_eq!(interrupted.status, JobStatus::Interrupted);
    assert_eq!(interrupted.progress.completed, u64::MAX);
    assert_eq!(jobs.clear_completed().await.unwrap().deleted, 3);
    jobs.shutdown().await.unwrap();
}

struct Compute(JobKindDescriptor);
impl JobHandler for Compute {
    type Input = tokio::sync::oneshot::Receiver<u64>;
    type Output = u64;
    fn kind(&self) -> &JobKindDescriptor {
        &self.0
    }
    fn run<'a>(
        &'a self,
        input: Self::Input,
        context: JobContext,
    ) -> Pin<Box<dyn Future<Output = Result<u64, JobError>> + Send + 'a>> {
        Box::pin(async move {
            tokio::select! { value = input => value.map_err(|_| JobError::new("input_closed", "Sender closed")), _ = context.cancellation().cancelled() => Err(JobError::new("cancelled", "Cancelled")) }
        })
    }
}

#[tokio::test]
async fn app_single_coordinator_close_and_commands() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(&dir.path().join("app.redb"), DbOpenMode::AutoCreate);
    let mut builder = JobsBuilder::new();
    let handler = builder
        .register(Compute(JobKindDescriptor {
            id: JobKindId("example.compute".into()),
            title: "Compute".into(),
            description: None,
        }))
        .unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("test"), db)
        .with_jobs(builder.build(), Default::default())
        .register_builtin_commands()
        .unwrap()
        .build()
        .unwrap();
    let principal = Principal::system();
    let (one, two) = tokio::join!(
        app.jobs(&principal, "test".into()),
        app.jobs(&principal, "test".into())
    );
    let one = one.unwrap();
    let two = two.unwrap();
    let group = one.create_group().await.unwrap();
    let (send, receive) = tokio::sync::oneshot::channel();
    let ticket = two
        .submit(
            &handler,
            receive,
            semantic_jobs::SubmitOptions {
                groups: vec![group],
            },
        )
        .await
        .unwrap();
    assert!(
        app.scopes()
            .close_scope(&principal, &"test".into())
            .is_err()
    );
    send.send(42).unwrap();
    assert_eq!(ticket.wait().await.unwrap(), 42);
    let context = semantic_app::AppRequestContext {
        app: app.clone(),
        principal: principal.clone(),
        session: None,
        request_scope: Some("test".into()),
    };
    let response = app
        .invoke(
            context,
            semantic_rpc_core::RpcRequest {
                id: 1,
                command: "semantic.jobs.list".into(),
                payload: semantic_data::Value::Object(JobListQuery::default().to_object()),
            },
        )
        .await;
    assert!(
        matches!(response.result, semantic_rpc_core::RpcResult::Ok(_)),
        "{:?}",
        response.result
    );
    app.scopes()
        .close_scope_with_jobs(&principal, &"test".into())
        .await
        .unwrap();
    assert_eq!(one.health(), semantic_jobs::JobsHealth::Closed);
}
