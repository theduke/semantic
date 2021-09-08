mod assets;

use std::{net::SocketAddr, ops::Add};

use anyhow::{Context, Result};
use axum::{
    extract::{self, Extension},
    AddExtensionLayer,
};
use factordb::AnyError;
use hyper::{Body, Method, Request, Response, StatusCode};

use semantic_core::api::{self, ApiError, ApiResponse, BackendConfig, Query};

use crate::app::App;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ServerConfig {
    /// The interface to bind to.
    ///
    /// Examples:
    /// - 127.0.0.1:3000
    /// - 0.0.0.0:8080
    /// - ::1:3000
    pub interface: String,
}

type AppState = extract::Extension<App>;

pub async fn run_server(app: App, config: ServerConfig) -> Result<(), AnyError> {
    use axum::handler::{get, post};

    // Run the server like above...
    let addr: SocketAddr = config.interface.parse().context(format!(
        "Invalid server interface specification '{}'",
        config.interface
    ))?;

    #[cfg(debug_assertions)]
    let asset_source = {
        let manifest_dir_raw =
            std::env::var("CARGO_MANIFEST_DIR").expect("Could not find CARGO_MANIFEST_DIR env var");
        let path = std::path::PathBuf::from(manifest_dir_raw)
            .parent()
            .expect("CARGO_MANIFEST_DIR has no parent")
            .parent()
            .expect("CARGO_MANIFEST_DIR has no parent")
            .join("target/ui");
        assets::FsAssetSource::new(path)
    };

    #[cfg(not(debug_assertions))]
    let asset_source = {
        static ASSETS: include_dir::Dir = include_dir::include_dir!("../../target/ui");
        assets::StaticAssetSource::new(ASSETS.clone())
    };

    let assets = assets::Assets::new(asset_source);

    let app = axum::Router::new()
        .route("/api/query", post(handler_api_query))
        .route(
            "/api/upload-file",
            post(handler_blob_upload).options(cors_handler),
        )
        .nest("/blob/files", get(handler_blob_read))
        .nest("/assets", get(handler_assets))
        .or(get(handler_index))
        .layer(AddExtensionLayer::new(app))
        .layer(AddExtensionLayer::new(assets))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    tracing::info!(interface=%addr, "starting web server");

    let server = axum::Server::bind(&addr).serve(app.into_make_service());

    server.await.map_err(|error| {
        tracing::error!(?error, "Server failed");
        error.into()
    })
}

async fn handler_assets(
    Extension(assets): extract::Extension<assets::Assets>,
    req: Request<Body>,
) -> Response<Body> {
    assets.request(req.uri().path())
}

async fn handler_index(Extension(assets): extract::Extension<assets::Assets>) -> Response<Body> {
    let mut res = assets.request("index.html");

    // Add Cross-Origin headers.
    // Both for security, and to enable better performance.now() precision.
    // See https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cross-Origin-Embedder-Policy
    // and https://developer.mozilla.org/en-US/docs/Web/HTTP/Cross-Origin_Resource_Policy_(CORP).
    res.headers_mut().append(
        header::HeaderName::from_str("Cross-Origin-Resource-Policy").unwrap(),
        "same-origin".parse().unwrap(),
    );
    res.headers_mut().append(
        header::HeaderName::from_str("Cross-Origin-Embedder-Policy").unwrap(),
        "require-corp".parse().unwrap(),
    );

    res
}

async fn cors_handler() -> Response<Body> {
    cors_response()
}

fn cors_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .header(
            hyper::header::ACCESS_CONTROL_ALLOW_HEADERS,
            format!(
                "{},content-type",
                semantic_core::api::FileUploadMetadata::HEADER_NAME
            ),
        )
        .body(Body::empty())
        .unwrap()
}

fn not_found() -> Response<Body> {
    Response::builder()
        .status(hyper::http::StatusCode::NOT_FOUND)
        .body(Body::from(b"Not found".to_vec()))
        .unwrap()
}

fn internal_server_error(msg: impl Into<String>) -> Response<Body> {
    Response::builder()
        .status(hyper::http::StatusCode::INTERNAL_SERVER_ERROR)
        .body(Body::from(msg.into()))
        .unwrap()
}

async fn handler_blob_upload(Extension(app): AppState, req: Request<Body>) -> Response<Body> {
    if req.method() == Method::OPTIONS {
        return cors_response();
    }

    tracing::trace!("handler_blob_upload");
    let reply = match file_upload(&app, req).await {
        Ok(item) => ApiResponse::Ok(item),
        Err(err) => {
            tracing::warn!(?err, "file upload failed");
            ApiResponse::Err(api_error(&err))
        }
    };

    tracing::info!(?reply, "blob upload reply");

    api_response(reply, Vec::new())
}

async fn file_upload(
    app: &App,
    req: Request<Body>,
) -> Result<semantic_core::base::TypedFile, AnyError> {
    // FIXME: check authentication

    tracing::trace!("file upload started");

    use semantic_core::api::FileUploadMetadata;

    let meta: FileUploadMetadata =
        if let Some(header) = req.headers().get(FileUploadMetadata::HEADER_NAME) {
            let raw = header
                .to_str()
                .map_err(|_| anyhow::anyhow!("Invalid file metadata header"))?;
            serde_json::from_str(raw).context("Invalid file metadata header")?
        } else {
            FileUploadMetadata {
                filename: None,
                title: None,
            }
        };

    tracing::trace!("fetching file upload body");
    let body = hyper::body::to_bytes(req.into_body()).await?;
    tracing::trace!(len=%body.len(), "file upload body retrieved");

    let item = app.create_file(meta, body.to_vec()).await?;
    tracing::trace!(?item, "file created");
    Ok(item)
}

async fn handler_blob_read(Extension(app): AppState, req: Request<Body>) -> Response<Body> {
    if req.method() == Method::OPTIONS {
        return cors_response();
    }

    let raw_path = req.uri().path();

    let real_path = if raw_path.starts_with("/blob/files/") {
        raw_path.trim_start_matches("/blob/").to_string()
    } else if raw_path.starts_with("/files") {
        raw_path.trim_start_matches('/').to_string()
    } else {
        format!("files{}", raw_path)
    };

    let blob = if let Some(b) = app.blob() {
        b
    } else {
        return internal_server_error("Blobstore not initialized");
    };

    match blob.get(&real_path).await {
        Ok(Some(data)) => {
            tracing::trace!(%real_path, "serving file");
            Response::builder()
                .status(StatusCode::OK)
                .body(data.into())
                .unwrap()
        }
        Ok(None) => not_found(),
        Err(err) => {
            tracing::error!(error=?err, "Could not serve file");
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(format!("Error: {}", err).into_bytes().into())
                .unwrap()
        }
    }
}

fn api_error(err: &AnyError) -> ApiError {
    ApiError {
        message: err.to_string(),
    }
}

fn api_response_err(err: &AnyError) -> ApiResponse {
    ApiResponse::Err(api_error(err))
}

fn api_response<T>(
    res: ApiResponse<T>,
    extra_headers: Vec<(hyper::header::HeaderName, hyper::http::HeaderValue)>,
) -> Response<Body>
where
    T: serde::Serialize,
{
    // TODO: no unwrap?
    let res_json = serde_json::to_vec(&res).unwrap();

    let mut res = Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, "*");

    for (key, value) in extra_headers {
        res = res.header(key, value);
    }

    res.body(Body::from(res_json)).unwrap()
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct TokenClaims {
    sub: String,
    exp: u64,
    // TODO: use hash of config instead.
    config: BackendConfig,
}

impl TokenClaims {
    fn encode(&self, key: &str) -> Result<String, jsonwebtoken::errors::Error> {
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            self,
            &jsonwebtoken::EncodingKey::from_secret(key.as_bytes()),
        )
    }

    fn decode(key: &str, token: &str) -> Result<Self, jsonwebtoken::errors::Error> {
        let data = jsonwebtoken::decode::<Self>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(key.as_bytes()),
            &jsonwebtoken::Validation::default(),
        )?;

        Ok(data.claims)
    }
}

async fn handler_api_query(Extension(app): AppState, req: Request<Body>) -> Response<Body> {
    match api_query(&app, req).await {
        Ok(res) => res,
        Err(err) => api_response(api_response_err(&err), Vec::new()),
    }
}

const TOKEN_COOKIE_NAME: &'static str = "token";

fn build_token_cookie(
    value: &str,
    delete: bool,
) -> Result<hyper::http::HeaderValue, hyper::http::header::InvalidHeaderValue> {
    let s = if delete {
        format!(
            "{}=; HttpOnly; expires=Thu, 01 Jan 1970 00:00:00 GMT;",
            TOKEN_COOKIE_NAME
        )
    } else {
        format!("{}={}; HttpOnly;", TOKEN_COOKIE_NAME, value)
    };

    hyper::header::HeaderValue::from_str(&s)
}

fn get_auth_cookie_token(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get_all("cookie")
        .iter()
        .find_map(|raw_value| {
            let values = raw_value.to_str().ok()?;
            let pair = values.split(';').find(|x| {
                x.trim_start()
                    .starts_with(&format!("{}=", TOKEN_COOKIE_NAME))
            })?;
            let value = pair.split('=').nth(1)?;
            Some(value.trim().to_string())
        })
}

fn validate_auth_token(app: &App, raw_token: &str) -> Result<TokenClaims, AnyError> {
    let claims = TokenClaims::decode(&app.config().token_key, raw_token)?;

    let config = app
        .backend_config()
        .ok_or_else(|| anyhow::anyhow!("Invalid token"))?;

    if config != claims.config {
        return Err(anyhow::anyhow!("Invalid token"));
    }
    Ok(claims)
}

async fn api_query(app: &App, req: Request<Body>) -> Result<Response<Body>, AnyError> {
    let token = get_auth_cookie_token(&req)
        .map(|token| validate_auth_token(app, &token))
        .transpose()?;

    let body = hyper::body::to_bytes(req.into_body()).await?;
    let query: Query = serde_json::from_slice(&body)?;

    tracing::trace!(?query, "running api query");

    // Handle queries that don't need authentication.

    if app.needs_authentication() && !token.is_some() {
        match &query {
            Query::ServerStatus | Query::Initialize { config: _ } => {}
            _ => {
                return Err(anyhow::anyhow!("Permission denied"));
            }
        }
    }

    let mut extra_headers = Vec::new();

    let res = match query {
        api::Query::ServerStatus => Ok(api::Reply::ServerStatus(api::ServerStatus {
            backend_initialized: app.db().is_some(),
        })),
        api::Query::Initialize { config } => {
            app.configure_backend(config.clone()).await?;
            let exp = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)?
                .add(std::time::Duration::from_secs(60 * 60 * 2))
                .as_secs();

            let key = &app.config().token_key;

            let new_token = TokenClaims {
                sub: "semantic".into(),
                exp,
                config: config.clone(),
            }
            .encode(key)?;

            let cookie = build_token_cookie(&new_token, false)?;

            extra_headers.push((hyper::header::SET_COOKIE, cookie));
            extra_headers.push((
                hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN,
                "*".parse().unwrap(),
            ));
            extra_headers.push((
                hyper::header::ACCESS_CONTROL_ALLOW_METHODS,
                "POST".parse().unwrap(),
            ));
            Ok(api::Reply::Initialize)
        }
        api::Query::CloseBackend => {
            app.close_backend().await?;
            extra_headers.push((hyper::header::SET_COOKIE, build_token_cookie("", true)?));
            Ok(api::Reply::CloseBackend)
        }
        other => {
            // No special server-related logic required, so we use the default
            // run_query.
            app.run_query(other).await
        }
    };

    let reply = res.map_err(|err| {
        tracing::error!(?err, "api query failed");
        err
    })?;
    let res = api_response(ApiResponse::Ok(reply), extra_headers);
    Ok(res)
}
