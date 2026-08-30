use axum::body::Body;
use axum::extract::OriginalUri;
use axum::response::Response;
use http::StatusCode;
use include_dir::{Dir, include_dir};

static UI: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/../../target/dx/semantic_ui/release/web/public");

pub(crate) async fn handler(OriginalUri(uri): OriginalUri) -> Response {
    let path = uri.path();
    if path == "/api" || path.starts_with("/api/") {
        return not_found();
    }

    let relative_path = path.trim_start_matches('/');
    let file = UI
        .get_file(relative_path)
        .or_else(|| UI.get_file("index.html"));

    match file {
        Some(file) => Response::builder()
            .header(
                http::header::CONTENT_TYPE,
                mime_guess::from_path(file.path())
                    .first_or_octet_stream()
                    .as_ref(),
            )
            .body(Body::from(file.contents()))
            .expect("embedded UI response is valid"),
        None => not_found(),
    }
}

fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::empty())
        .expect("not-found response is valid")
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use http::{StatusCode, Uri, header};

    use super::*;

    #[tokio::test]
    async fn serves_files_and_falls_back_to_index_for_client_routes() {
        let index = UI.get_file("index.html").expect("embedded index.html");

        let root = handler(OriginalUri(Uri::from_static("/"))).await;
        assert_eq!(root.status(), StatusCode::OK);
        assert_eq!(root.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(
            to_bytes(root.into_body(), usize::MAX).await.unwrap(),
            index.contents()
        );

        let client_route = handler(OriginalUri(Uri::from_static("/collections/example"))).await;
        assert_eq!(client_route.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(client_route.into_body(), usize::MAX)
                .await
                .unwrap(),
            index.contents()
        );

        let asset = UI
            .get_dir("assets")
            .expect("embedded assets directory")
            .files()
            .next()
            .expect("embedded UI asset");
        let asset_uri = format!("/{}", asset.path().display()).parse().unwrap();
        let asset_response = handler(OriginalUri(asset_uri)).await;
        assert_eq!(asset_response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(asset_response.into_body(), usize::MAX)
                .await
                .unwrap(),
            asset.contents()
        );
    }

    #[tokio::test]
    async fn does_not_serve_the_ui_for_unknown_api_routes() {
        let response = handler(OriginalUri(Uri::from_static("/api/unknown"))).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
