use super::*;
use crate::{AppRequestContext, DbScopeId, FileContent, FileCreateRequest, Principal, SemanticApp};
use semantic_db_core::{DEFAULT_COLLECTION, Db};

fn db(path: &Path) -> Arc<Db> {
    Arc::new(Db::new(
        semantic_db_redb::open_backend(path, semantic_data::schema::DbOpenMode::AutoCreate)
            .unwrap(),
    ))
}

fn ctx(db: Arc<Db>, store: DynObjStore) -> AppRequestContext {
    AppRequestContext {
        app: SemanticApp::builder()
            .with_default_scope(DbScopeId::new("main"), db)
            .with_default_file_store(DbScopeId::new("main"), store)
            .with_auto_analyze_media(false)
            .build()
            .unwrap(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    }
}

async fn create(ctx: &AppRequestContext, id: &str) -> String {
    let record = ctx
        .app
        .files()
        .create(
            ctx,
            FileCreateRequest {
                scope_id: None,
                id: Some(id.into()),
                filestore_locator: None,
                filename: None,
                mime_type: Some("text/plain".into()),
                entity: Object::new(),
                content: FileContent::Bytes(bytes::Bytes::from_static(b"preserve me")),
            },
        )
        .await
        .unwrap();
    record
        .object
        .get("filestore_locator")
        .unwrap()
        .as_str()
        .unwrap()
        .into()
}

fn inventory(db: &Arc<Db>) -> BTreeMap<String, Arc<dyn SemanticDb>> {
    BTreeMap::from([("db".into(), db.clone() as Arc<dyn SemanticDb>)])
}

fn options() -> MaintenanceOptions {
    MaintenanceOptions {
        dry_run: false,
        sweep_orphans: false,
        max_entries: 100,
    }
}

async fn cleanups(db: &Db) -> Vec<Object> {
    let QueryResult::Select(rows) = db
        .query("SELECT * FROM entities WHERE type = 'semantic:filestore:cleanup'")
        .await
        .unwrap()
    else {
        panic!("select");
    };
    rows
}

#[test]
fn exclusive_physical_lease_owner_and_inventory_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    assert!(
        ManagedFileStore::initialize(&root, "app".into(), BTreeSet::from(["db".into()]), 0)
            .is_err()
    );
    let managed =
        ManagedFileStore::initialize(&root, "app".into(), BTreeSet::from(["db".into()]), 10)
            .unwrap();
    let store = managed.into_store();
    let other_scope = store.clone();
    let raw: DynObjStore = Arc::new(
        objstore_fs::FsObjStore::new(objstore_fs::FsObjStoreConfig::new(root.join("objects")))
            .unwrap(),
    );
    assert!(validate_store_access(raw.as_ref()).is_err());
    assert!(validate_store_access(store.as_ref()).is_ok());
    assert!(ManagedFileStore::open(&root, "app").is_err());
    drop(store);
    assert!(ManagedFileStore::open(&root, "app").is_err());
    drop(other_scope);
    assert!(ManagedFileStore::open(&root, "different-app").is_err());
    assert!(FileMaintenance::open(&root, "app", BTreeMap::new()).is_err());
    assert!(
        ManagedFileStore::initialize(&root, "app".into(), BTreeSet::from(["db".into()]), 10)
            .is_err()
    );
    assert!(ManagedFileStore::open(&root, "app").is_ok());
    let shared = temp.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::write(shared.join("existing"), b"unmanaged").unwrap();
    assert!(
        ManagedFileStore::initialize(&shared, "app".into(), BTreeSet::from(["db".into()]), 10)
            .is_err()
    );
    assert_eq!(
        std::fs::read(shared.join("existing")).unwrap(),
        b"unmanaged"
    );
}

#[cfg(unix)]
#[test]
fn canonical_path_aliases_share_the_same_store_lease() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let managed =
        ManagedFileStore::initialize(&root, "app".into(), BTreeSet::from(["db".into()]), 10)
            .unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    assert!(ManagedFileStore::open(&alias, "app").is_err());
    drop(managed);
    let via_alias = ManagedFileStore::open(&alias, "app").unwrap();
    assert_eq!(via_alias.root, root.canonicalize().unwrap());
    assert!(ManagedFileStore::open(&root, "app").is_err());
}

#[tokio::test]
async fn retention_dry_run_and_republication_across_registered_databases() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let managed = ManagedFileStore::initialize(
        &root,
        "app".into(),
        BTreeSet::from(["a".into(), "b".into()]),
        60,
    )
    .unwrap();
    let store = managed.into_store();
    let a = db(&temp.path().join("a.redb"));
    let b = db(&temp.path().join("b.redb"));
    let ca = ctx(a.clone(), store.clone());
    let cb = ctx(b.clone(), store.clone());
    cb.resolve_db(None).await.unwrap();
    let locator = create(&ca, "first").await;
    let file = a
        .get(DEFAULT_COLLECTION, "first")
        .await
        .unwrap()
        .unwrap()
        .object;
    ca.app
        .files()
        .delete(&ca, None, "first".into())
        .await
        .unwrap();
    // Publication after deletion in another scope/database must protect the bytes.
    SemanticDb::insert(b.as_ref(), DEFAULT_COLLECTION.into(), "first".into(), file)
        .await
        .unwrap();
    assert!(
        FileMaintenance::open(
            &root,
            "app",
            BTreeMap::from([
                ("a".into(), a.clone() as Arc<dyn SemanticDb>),
                ("b".into(), b.clone() as Arc<dyn SemanticDb>)
            ])
        )
        .is_err()
    );
    drop(ca);
    drop(cb);
    drop(store);
    let maintenance = FileMaintenance::open(
        &root,
        "app",
        BTreeMap::from([
            ("a".into(), a.clone() as Arc<dyn SemanticDb>),
            ("b".into(), b.clone() as Arc<dyn SemanticDb>),
        ]),
    )
    .unwrap();
    let now = time::OffsetDateTime::now_utc();
    assert_eq!(
        maintenance
            .run_at(options(), now + time::Duration::hours(1))
            .await
            .unwrap()
            .retained,
        1
    );
    // Simulate a later offline maintenance window with the last metadata removed.
    SemanticDb::delete(b.as_ref(), DEFAULT_COLLECTION.into(), "first".into())
        .await
        .unwrap();
    assert_eq!(
        maintenance.run_at(options(), now).await.unwrap().retained,
        1
    );
    let dry = maintenance
        .run_at(
            MaintenanceOptions {
                dry_run: true,
                ..options()
            },
            now + time::Duration::hours(1),
        )
        .await
        .unwrap();
    assert_eq!(dry.eligible, 1);
    assert_eq!(dry.deleted, 0);
    assert!(root.join("objects").join(&locator).exists());
    assert_eq!(cleanups(&a).await.len(), 1);
    assert_eq!(
        maintenance
            .run_at(options(), now + time::Duration::hours(1))
            .await
            .unwrap()
            .deleted,
        1
    );
    assert!(!root.join("objects").join(&locator).exists());
    assert!(cleanups(&a).await.is_empty());
}

#[tokio::test]
async fn persisted_retry_backoff_and_crash_after_byte_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let managed =
        ManagedFileStore::initialize(&root, "app".into(), BTreeSet::from(["db".into()]), 1)
            .unwrap();
    let db = db(&temp.path().join("db.redb"));
    let context = ctx(db.clone(), managed.into_store());
    let locator = create(&context, "retry").await;
    context
        .app
        .files()
        .delete(&context, None, "retry".into())
        .await
        .unwrap();
    drop(context);
    let path = root.join("objects").join(&locator);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap(); // Deterministic storage deletion failure.
    let now = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
    let maintenance = FileMaintenance::open(&root, "app", inventory(&db)).unwrap();
    assert_eq!(maintenance.run_at(options(), now).await.unwrap().failed, 1);
    let rows = cleanups(&db).await;
    assert_eq!(
        cleanup_value(&rows[0], "attempts").and_then(unsigned),
        Some(1)
    );
    assert!(cleanup_string(&rows[0], "last_error").is_ok());
    drop(maintenance);
    // Restart, preserving the retry timestamp and failure count.
    let maintenance = FileMaintenance::open(&root, "app", inventory(&db)).unwrap();
    assert_eq!(
        maintenance
            .run_at(options(), now + time::Duration::seconds(1))
            .await
            .unwrap()
            .eligible,
        0
    );
    std::fs::remove_dir(&path).unwrap();
    // Missing bytes model a crash after storage deletion and before intent removal.
    assert_eq!(
        maintenance
            .run_at(options(), now + time::Duration::seconds(2))
            .await
            .unwrap()
            .deleted,
        1
    );
    assert!(cleanups(&db).await.is_empty());
}

#[tokio::test]
async fn orphan_sweep_is_bounded_dry_run_and_uses_durable_queue() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let managed =
        ManagedFileStore::initialize(&root, "app".into(), BTreeSet::from(["db".into()]), 10)
            .unwrap();
    let db = db(&temp.path().join("db.redb"));
    let context = ctx(db.clone(), managed.into_store());
    context.resolve_db(None).await.unwrap();
    drop(context);
    let locator = format!("upload-tmp-{}", uuid::Uuid::new_v4());
    let path = root.join("objects").join(&locator);
    std::fs::write(&path, b"crashed upload").unwrap();
    std::fs::write(root.join("objects/legacy-custom"), b"retain").unwrap();
    let now = time::OffsetDateTime::now_utc();
    let maintenance = FileMaintenance::open(&root, "app", inventory(&db)).unwrap();
    let sweep = MaintenanceOptions {
        sweep_orphans: true,
        max_entries: 1,
        ..options()
    };
    assert_eq!(
        maintenance.run_at(sweep, now).await.unwrap().orphan_intents,
        0
    );
    assert_eq!(
        maintenance
            .run_at(
                MaintenanceOptions {
                    dry_run: true,
                    ..sweep
                },
                now + time::Duration::hours(1)
            )
            .await
            .unwrap()
            .orphan_intents,
        1
    );
    assert!(cleanups(&db).await.is_empty());
    assert_eq!(
        maintenance
            .run_at(sweep, now + time::Duration::hours(1))
            .await
            .unwrap()
            .orphan_intents,
        1
    );
    assert!(path.exists());
    assert_eq!(
        maintenance
            .run_at(sweep, now + time::Duration::hours(1))
            .await
            .unwrap()
            .deleted,
        0
    );
    assert_eq!(
        maintenance
            .run_at(sweep, now + time::Duration::hours(2))
            .await
            .unwrap()
            .deleted,
        1
    );
    assert!(!path.exists());
    assert!(root.join("objects/legacy-custom").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn orphan_sweep_never_follows_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("store");
    let managed =
        ManagedFileStore::initialize(&root, "app".into(), BTreeSet::from(["db".into()]), 1)
            .unwrap();
    let db = db(&temp.path().join("db.redb"));
    let context = ctx(db.clone(), managed.into_store());
    context.resolve_db(None).await.unwrap();
    drop(context);
    let external = temp.path().join("external");
    std::fs::write(&external, b"never delete").unwrap();
    let locator = format!("upload-tmp-{}", uuid::Uuid::new_v4());
    std::os::unix::fs::symlink(&external, root.join("objects").join(locator)).unwrap();
    let maintenance = FileMaintenance::open(&root, "app", inventory(&db)).unwrap();
    let report = maintenance
        .run_at(
            MaintenanceOptions {
                sweep_orphans: true,
                ..options()
            },
            time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        )
        .await
        .unwrap();
    assert_eq!(report.orphan_intents, 0);
    assert_eq!(std::fs::read(external).unwrap(), b"never delete");
}
