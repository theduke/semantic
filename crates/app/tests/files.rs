use std::sync::Arc;

use bytes::Bytes;
use futures_util::TryStreamExt;
use semantic_app::{
    AppError, AppRequestContext, DbScopeId, FileContent, FileCreateRequest, Principal, SemanticApp,
    SemanticDb,
};
use semantic_data::{Object, schema::DbOpenMode};
use semantic_db_core::{Batch, BatchOperation, Db, DbError};

fn request(id: Option<&str>, bytes: &'static [u8]) -> FileCreateRequest {
    FileCreateRequest {
        scope_id: None,
        id: id.map(str::to_owned),
        filestore_locator: None,
        filename: Some("original.txt".into()),
        mime_type: Some("text/plain".into()),
        entity: Object::new(),
        content: FileContent::Bytes(Bytes::from_static(bytes)),
    }
}

fn setup(path: &std::path::Path) -> (AppRequestContext, Arc<Db>) {
    let db = Arc::new(Db::new(
        semantic_db_redb::open_backend(path.join("db.redb"), DbOpenMode::AutoCreate).unwrap(),
    ));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .with_default_file_store_uri(
            DbScopeId::new("main"),
            format!("fs://{}", path.join("blobs").display()),
        )
        .with_auto_analyze_media(false)
        .build()
        .unwrap();
    (
        AppRequestContext {
            app,
            principal: Principal::system(),
            session: None,
            request_scope: None,
        },
        db,
    )
}

async fn contents(ctx: &AppRequestContext, id: &str) -> Vec<u8> {
    ctx.app
        .files()
        .read(ctx, None, id.into())
        .await
        .unwrap()
        .stream
        .try_fold(Vec::new(), |mut bytes, chunk| async move {
            bytes.extend_from_slice(&chunk);
            Ok(bytes)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn duplicate_and_concurrent_uploads_preserve_original_identity_and_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let (ctx, db) = setup(temp.path());
    ctx.app
        .files()
        .create(&ctx, request(Some("same"), b"original"))
        .await
        .unwrap();
    let before = db
        .get(semantic_db_core::DEFAULT_COLLECTION, "same")
        .await
        .unwrap()
        .unwrap();
    let error = ctx
        .app
        .files()
        .create(&ctx, request(Some("same"), b"replacement"))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, AppError::FileAlreadyExists { id } if id == "same"));
    assert_eq!(
        db.get(semantic_db_core::DEFAULT_COLLECTION, "same")
            .await
            .unwrap()
            .unwrap()
            .object,
        before.object
    );
    assert_eq!(contents(&ctx, "same").await, b"original");

    let (left, right) = tokio::join!(
        ctx.app.files().create(&ctx, request(Some("race"), b"left")),
        ctx.app
            .files()
            .create(&ctx, request(Some("race"), b"right")),
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let expected = if left.is_ok() {
        b"left".as_slice()
    } else {
        b"right".as_slice()
    };
    assert_eq!(contents(&ctx, "race").await, expected);
    let first = ctx
        .app
        .files()
        .create(&ctx, request(None, b"hash collision"))
        .await
        .unwrap();
    assert!(matches!(
        ctx.app
            .files()
            .create(&ctx, request(None, b"hash collision"))
            .await,
        Err(AppError::FileAlreadyExists { .. })
    ));
    assert_eq!(contents(&ctx, &first.id).await, b"hash collision");
}

#[tokio::test]
async fn create_checks_sequential_overlay_and_rejects_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let (ctx, db) = setup(temp.path());
    ctx.resolve_db(None).await.unwrap();
    let create = || {
        let mut object = Object::new();
        object.insert("id", "new".to_owned());
        BatchOperation::Create {
            collection: semantic_db_core::DEFAULT_COLLECTION.into(),
            id: "new".into(),
            object,
        }
    };
    let error = SemanticDb::execute_batch(
        db.as_ref(),
        Batch::new().with_op(create()).with_op(create()),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DbError::EntityExists { .. }));
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION, "new")
            .await
            .unwrap()
            .is_none()
    );
    SemanticDb::execute_batch(
        db.as_ref(),
        Batch::new()
            .with_op(create())
            .with_op(BatchOperation::DeleteById {
                collection: semantic_db_core::DEFAULT_COLLECTION.into(),
                id: "new".into(),
            })
            .with_op(create()),
    )
    .await
    .unwrap();
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION, "new")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn explicit_delete_retains_shared_bytes_and_records_durable_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let (ctx, db) = setup(temp.path());
    let first = ctx
        .app
        .files()
        .create(&ctx, request(Some("first"), b"shared"))
        .await
        .unwrap();
    let locator = first
        .object
        .get("filestore_locator")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let mut shared = first.object.clone();
    shared.insert("id", "second".to_owned());
    SemanticDb::insert(
        db.as_ref(),
        semantic_db_core::DEFAULT_COLLECTION.into(),
        "second".into(),
        shared,
    )
    .await
    .unwrap();
    ctx.app
        .files()
        .delete(&ctx, None, "first".into())
        .await
        .unwrap();
    ctx.app
        .files()
        .delete(&ctx, None, "first".into())
        .await
        .unwrap();
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION, "first")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(contents(&ctx, "second").await, b"shared");
    assert!(temp.path().join("blobs").join(&locator).exists());
    let cleanup = db
        .query("SELECT * FROM entities WHERE type = 'semantic:filestore:cleanup'")
        .await
        .unwrap();
    let semantic_db_core::QueryResult::Select(rows) = cleanup else {
        panic!("select")
    };
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0]
            .iter()
            .any(|(_, value)| value.as_str() == Some(locator.as_str()))
    );
    drop(ctx);
    drop(db);
    let (ctx, db) = setup(temp.path());
    ctx.resolve_db(None).await.unwrap();
    let semantic_db_core::QueryResult::Select(rows) = db
        .query("SELECT * FROM entities WHERE type = 'semantic:filestore:cleanup'")
        .await
        .unwrap()
    else {
        panic!("select")
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(contents(&ctx, "second").await, b"shared");
}

#[tokio::test]
async fn enforced_reference_blocks_native_delete_but_unlinking_retains_file() {
    use semantic_data::schema::{AttributeType, Meta, Type, TypeKind, TypeRef};
    use semantic_db_core::{DdlBatch, DdlOperation};
    let temp = tempfile::tempdir().unwrap();
    let (ctx, db) = setup(temp.path());
    ctx.app
        .files()
        .create(&ctx, request(Some("target"), b"linked"))
        .await
        .unwrap();
    db.execute_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute {
        attribute: AttributeType {
            id: "test:file_ref".into(),
            name: "file_ref".into(),
            ty: Type::new(TypeKind::Ref(TypeRef::new(
                semantic_data::filestore::FILE_CLASS_ID,
            ))),
            constraints: Vec::new(),
            meta: Meta::default(),
        },
    }))
    .await
    .unwrap();
    let mut owner = Object::new();
    owner.insert("id", "owner".to_owned());
    owner.insert("test:file_ref", "target".to_owned());
    SemanticDb::insert(
        db.as_ref(),
        semantic_db_core::DEFAULT_COLLECTION.into(),
        "owner".into(),
        owner,
    )
    .await
    .unwrap();
    assert!(matches!(
        ctx.app.files().delete(&ctx, None, "target".into()).await,
        Err(AppError::FileReferenced { .. })
    ));
    db.activate_validation().await.unwrap();
    assert!(matches!(
        ctx.app.files().delete(&ctx, None, "target".into()).await,
        Err(AppError::FileReferenced { .. })
    ));
    assert_eq!(contents(&ctx, "target").await, b"linked");
    let semantic_db_core::QueryResult::Select(rows) = db
        .query("SELECT * FROM entities WHERE type = 'semantic:filestore:cleanup'")
        .await
        .unwrap()
    else {
        panic!("select")
    };
    assert!(
        rows.is_empty(),
        "failed native deletion must not leave cleanup intent"
    );
    SemanticDb::delete(
        db.as_ref(),
        semantic_db_core::DEFAULT_COLLECTION.into(),
        "owner".into(),
    )
    .await
    .unwrap();
    assert_eq!(contents(&ctx, "target").await, b"linked");
    ctx.app
        .files()
        .delete(&ctx, None, "target".into())
        .await
        .unwrap();
}

#[tokio::test]
async fn custom_publication_fails_closed_and_legacy_locators_remain_readable() {
    let temp = tempfile::tempdir().unwrap();
    let (ctx, db) = setup(temp.path());
    let file = ctx
        .app
        .files()
        .create(&ctx, request(Some("legacy"), b"legacy bytes"))
        .await
        .unwrap();
    let current = file
        .object
        .get("filestore_locator")
        .unwrap()
        .as_str()
        .unwrap();
    std::fs::rename(
        temp.path().join("blobs").join(current),
        temp.path().join("blobs/legacy-custom"),
    )
    .unwrap();
    let mut object = file.object;
    object.insert("filestore_locator", "legacy-custom".to_owned());
    SemanticDb::insert(
        db.as_ref(),
        semantic_db_core::DEFAULT_COLLECTION.into(),
        "legacy".into(),
        object,
    )
    .await
    .unwrap();
    assert_eq!(contents(&ctx, "legacy").await, b"legacy bytes");
    let mut replacement = request(Some("attacker"), b"replacement");
    replacement.filestore_locator = Some("legacy-custom".into());
    assert!(matches!(
        ctx.app.files().create(&ctx, replacement).await,
        Err(AppError::InvalidFileMetadata(_))
    ));
    assert_eq!(contents(&ctx, "legacy").await, b"legacy bytes");
}

#[tokio::test]
async fn failed_stream_never_publishes_metadata() {
    use futures_util::StreamExt;
    let temp = tempfile::tempdir().unwrap();
    let (ctx, db) = setup(temp.path());
    let mut upload = request(Some("broken"), b"");
    upload.content = FileContent::Stream(semantic_app::FileSizedStream {
        size: Some(10),
        stream: futures_util::stream::iter([Ok(Bytes::from_static(b"short"))]).boxed(),
    });
    assert!(ctx.app.files().create(&ctx, upload).await.is_err());
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION, "broken")
            .await
            .unwrap()
            .is_none()
    );
}
