use axum::body::{Body, to_bytes};
use http::{Request, StatusCode};
use semantic_app::AppConfig;
use semantic_server::SemanticServer;
use tower::ServiceExt;

#[tokio::test]
async fn duplicate_upload_and_explicit_delete_have_stable_http_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let config = AppConfig::new()
        .with_data_dir(temp.path())
        .with_auto_analyze_media(false);
    let server = SemanticServer::from_uris(
        format!("redb:{}", temp.path().join("db.redb").display()),
        config.default_blob_uri().unwrap(),
        None,
        config,
    )
    .await
    .unwrap();
    for (body, expected) in [
        ("original", StatusCode::CREATED),
        ("changed", StatusCode::CONFLICT),
    ] {
        let response = server
            .router()
            .oneshot(
                Request::post("/api/v1/file")
                    .header("x-semantic-file-id", "identity")
                    .header("content-type", "text/plain")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let data = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&data).unwrap();
        if expected == StatusCode::CONFLICT {
            assert_eq!(value["code"], "file_already_exists");
            assert_eq!(value["data"]["object"]["id"]["string"], "identity");
        }
    }
    let response = server
        .router()
        .oneshot(
            Request::get("/api/v1/file/identity")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .as_ref(),
        b"original"
    );
    for _ in 0..2 {
        let response = server
            .router()
            .oneshot(
                Request::delete("/api/v1/file/identity")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
    }
    let response = server
        .router()
        .oneshot(
            Request::get("/api/v1/file/identity")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(value["code"], "file_not_found");
}
