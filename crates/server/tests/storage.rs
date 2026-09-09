use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use semantic_app::AppConfig;
use semantic_server::{LocalDbConfig, SemanticServer};
use tower::ServiceExt;

#[test]
fn local_database_uris_preserve_paths() {
    for (uri, path) in [
        ("redb:relative/data", "relative/data"),
        ("redb:/absolute/data", "/absolute/data"),
        ("redb:space %20?#.db", "space %20?#.db"),
    ] {
        assert_eq!(
            LocalDbConfig::from_uri("redb", uri).unwrap().path.to_str(),
            Some(path)
        );
    }
    for uri in [
        "redb",
        "redb:",
        "logfs:data",
        "redb:<blob>",
        "redb:bad\0path",
    ] {
        assert!(LocalDbConfig::from_uri("redb", uri).is_err(), "{uri}");
    }
}

#[test]
fn local_database_uris_reject_url_syntax() {
    for scheme in ["redb", "logfs"] {
        for path in ["", "relative/data", "/absolute/data"] {
            let uri = format!("{scheme}://{path}");
            assert!(LocalDbConfig::from_uri(scheme, &uri).is_err(), "{uri}");
        }
    }
}

async fn upload(server: &SemanticServer) -> String {
    let response = server
        .router()
        .oneshot(
            Request::post("/api/v1/file")
                .header("content-type", "text/plain")
                .header("x-semantic-filename", "hello.txt")
                .body(Body::from("persistent contents"))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let uploaded: serde_json::Value = serde_json::from_slice(&body).unwrap();
    uploaded["id"].as_str().unwrap().to_owned()
}

async fn download(server: &SemanticServer, id: &str) {
    let response = server
        .router()
        .oneshot(
            Request::get(format!("/api/v1/file/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(body.as_ref(), b"persistent contents");
}

#[cfg(feature = "logfs")]
#[tokio::test]
async fn shared_logfs_opens_once_and_replays_database_and_blobs() {
    use objstore::{ObjStore, ObjStoreBuilder};

    let dir = tempfile::tempdir().unwrap();
    let blob_uri = format!(
        "logfs://{}?allow_create=true",
        dir.path().join("shared.log").display()
    );
    let config = AppConfig::new().with_auto_analyze_media(false);
    let server = SemanticServer::from_uris("log:<blob>".into(), blob_uri.clone(), config.clone())
        .await
        .unwrap();
    // logfs holds an exclusive file lock: opening the backend independently
    // would fail here, and would also have prevented server startup above.
    let mut stores = ObjStoreBuilder::new();
    stores.register_provider(objstore_logfs::LogFsProvider::new());
    assert!(stores.build(&blob_uri).is_err());
    let id = upload(&server).await;
    download(&server, &id).await;
    drop(server);

    {
        let store = stores.build(&blob_uri).unwrap();
        let keys = store.list_all_keys("").await.unwrap();
        assert!(keys.iter().any(|key| key.starts_with("db/default/wal/v1/")));
        assert!(keys.iter().any(|key| key == &format!("blob/default/{id}")));
        assert!(
            keys.iter()
                .all(|key| key.starts_with("db/default/") || key.starts_with("blob/default/"))
        );
    }

    let server = SemanticServer::from_uris("log:<blob>".into(), blob_uri, config)
        .await
        .unwrap();
    // Download needs both the replayed database record and the persisted blob.
    download(&server, &id).await;
}

#[cfg(feature = "redb")]
#[tokio::test]
async fn redb_uri_creates_parent_and_preserves_blob_root_layout() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::new()
        .with_data_dir(dir.path())
        .with_auto_analyze_media(false);
    let db_uri = format!("redb:{}", dir.path().join("nested/database.redb").display());
    let blob_uri = config.default_blob_uri().unwrap();
    let server = SemanticServer::from_uris(db_uri.clone(), blob_uri.clone(), config.clone())
        .await
        .unwrap();
    let id = upload(&server).await;
    assert!(config.default_blob_path().join(&id).exists());
    drop(server);
    let server = SemanticServer::from_uris(db_uri, blob_uri, config)
        .await
        .unwrap();
    download(&server, &id).await;
}

#[cfg(feature = "logfs")]
#[tokio::test]
async fn direct_logfs_uri_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig::new()
        .with_data_dir(dir.path())
        .with_auto_analyze_media(false);
    let db_uri = format!("logfs:{}", dir.path().join("nested/database.log").display());
    let blob_uri = config.default_blob_uri().unwrap();
    let server = SemanticServer::from_uris(db_uri.clone(), blob_uri.clone(), config.clone())
        .await
        .unwrap();
    let id = upload(&server).await;
    drop(server);
    let server = SemanticServer::from_uris(db_uri, blob_uri, config)
        .await
        .unwrap();
    download(&server, &id).await;
}

#[cfg(feature = "logfs")]
#[tokio::test]
async fn shared_database_validates_config_mode_and_reuses_one_writer() {
    use objstore::ObjStoreBuilder;
    use semantic_app::{DbOpenRequest, DbProvider, Principal};
    use semantic_data::schema::DbOpenMode;
    use semantic_server::{LogDbConfig, LogDbProvider};
    use std::sync::Arc;

    assert_eq!(
        LogDbConfig::from_uri("log:<blob>").unwrap(),
        LogDbConfig::BlobStore
    );
    for uri in [
        "log:",
        "log:other",
        "logfs:<blob>",
        "log://<blob>",
        "log:<blob>?prefix=x",
    ] {
        assert!(LogDbConfig::from_uri(uri).is_err(), "{uri}");
    }
    let dir = tempfile::tempdir().unwrap();
    let mut stores = ObjStoreBuilder::new();
    stores.register_provider(objstore_logfs::LogFsProvider::new());
    let store = stores
        .build(&format!(
            "logfs://{}?allow_create=true",
            dir.path().join("shared.log").display()
        ))
        .unwrap();
    let provider = LogDbProvider::new(store);
    let principal = Principal::system();
    assert!(
        provider
            .open(
                DbOpenRequest {
                    uri: "log:<blob>".into(),
                    mode: DbOpenMode::OpenExisting
                },
                &principal
            )
            .await
            .is_err()
    );
    let request = DbOpenRequest {
        uri: "log:<blob>".into(),
        mode: DbOpenMode::AutoCreate,
    };
    let (first, second) = tokio::join!(
        provider.open(request.clone(), &principal),
        provider.open(request, &principal)
    );
    assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
}
