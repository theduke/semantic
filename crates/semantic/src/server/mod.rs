mod assets;

use std::{net::SocketAddr, ops::Add, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    extract::{self, Extension},
    http, AddExtensionLayer,
};
use factordb::AnyError;
use hyper::{header, Body, Method, Request, Response, StatusCode};

use semantic_core::api::{self, ApiError, ApiResponse, DbConfig, Query};

use crate::app::{App, AppConfig};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ServerConfig {
    /// General Semantic config.
    pub app: AppConfig,

    /// The interface to bind to.
    ///
    /// Examples:
    /// - 127.0.0.1:3000
    /// - 0.0.0.0:8080
    /// - ::1:3000
    pub interface: String,

    /// If true, all interaction via the server requires a login or an access
    /// token.
    pub require_auth: bool,
}

struct ServerState {
    config: ServerConfig,
    app: App,
}

impl ServerState {
    fn needs_auth(&self) -> bool {
        self.config.require_auth
    }
}

type ServerContext = Extension<Arc<ServerState>>;

pub async fn run_server(
    config: ServerConfig,
    runtime: tokio::runtime::Handle,
) -> Result<(), AnyError> {
    use axum::handler::{get, post};

    // Run the server like above...
    let addr: SocketAddr = config.interface.parse().context(format!(
        "Invalid server interface specification '{}'",
        config.interface
    ))?;

    let app = App::build(config.app.clone(), runtime).await?;

    let state = Arc::new(ServerState { config, app });

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

    let router = axum::Router::new()
        .route("/api/query", post(handler_api_query))
        .route(
            "/api/upload-file",
            post(handler_blob_upload).options(cors_handler),
        )
        .nest("/blob/files", get(handler_blob_read))
        .nest("/assets", get(handler_assets))
        .or(get(handler_index))
        .layer(AddExtensionLayer::new(state))
        .layer(AddExtensionLayer::new(assets))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    tracing::info!(interface=%addr, "starting web server");

    let server = axum::Server::bind(&addr).serve(router.into_make_service());

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
    let res = assets.request("index.html");

    // COMMENTED OUT due to image loading issues in chrome.
    // Add Cross-Origin headers.
    // Both for security, and to enable better performance.now() precision.
    // See https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Cross-Origin-Embedder-Policy
    // and https://developer.mozilla.org/en-US/docs/Web/HTTP/Cross-Origin_Resource_Policy_(CORP).
    // res.headers_mut().append(
    //     header::HeaderName::from_str("Cross-Origin-Resource-Policy").unwrap(),
    //     "same-origin".parse().unwrap(),
    // );
    // res.headers_mut().append(
    //     header::HeaderName::from_str("Cross-Origin-Embedder-Policy").unwrap(),
    //     "require-corp".parse().unwrap(),
    // );

    res
}

async fn cors_handler() -> Response<Body> {
    cors_response()
}

fn cors_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .header(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
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

async fn handler_blob_upload(
    Extension(state): ServerContext,
    req: Request<Body>,
) -> Response<Body> {
    if let Err(err) = request_auth_check(&state, &req) {
        return api_response(api_response_err(&err), vec![]);
    }

    if req.method() == Method::OPTIONS {
        return cors_response();
    }

    tracing::trace!("handler_blob_upload");
    let reply = match file_upload(&state, req).await {
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
    state: &ServerState,
    req: Request<Body>,
) -> Result<semantic_core::base::TypedFile, AnyError> {
    request_validate_auth_cookie(state, &req)?;

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
                collection_id: None,
            }
        };

    tracing::trace!("fetching file upload body");
    let body = hyper::body::to_bytes(req.into_body()).await?;
    tracing::trace!(len=%body.len(), "file upload body retrieved");

    let item = state.app.create_file(meta, body.to_vec()).await?;
    tracing::trace!(?item, "file created");
    Ok(item)
}

async fn handler_blob_read(Extension(state): ServerContext, req: Request<Body>) -> Response<Body> {
    if let Err(res) = request_validate_auth_cookie_http(&state, &req) {
        return res;
    }

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

    let blob = if let Some(b) = state.app.blob() {
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
    extra_headers: Vec<(header::HeaderName, hyper::http::HeaderValue)>,
) -> Response<Body>
where
    T: serde::Serialize,
{
    // TODO: no unwrap?
    let res_json = serde_json::to_vec(&res).unwrap();

    let mut res = Response::builder()
        .status(StatusCode::OK)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*");

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
    config: DbConfig,
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

async fn handler_api_query(Extension(state): ServerContext, req: Request<Body>) -> Response<Body> {
    match api_query(&state, req).await {
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

    header::HeaderValue::from_str(&s)
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

    if config.db.clone().purge_secrets() != claims.config {
        return Err(anyhow::anyhow!("Invalid token"));
    }
    Ok(claims)
}

fn request_validate_auth_cookie(
    state: &ServerState,
    req: &Request<Body>,
) -> Result<Option<TokenClaims>, AnyError> {
    get_auth_cookie_token(req)
        .map(|raw| validate_auth_token(&state.app, &raw))
        .transpose()
}

/// Validate a request for authentication cookies.
/// On error, returns a response that resets the auth cookie if required.
fn request_validate_auth_cookie_http(
    state: &ServerState,
    req: &Request<Body>,
) -> Result<Option<TokenClaims>, Response<Body>> {
    request_validate_auth_cookie(state, req).map_err(|_err| {
        let reset_header = build_token_cookie("", true).unwrap();

        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(http::header::SET_COOKIE, reset_header)
            .body(Body::from("Unauthorized".to_string()))
            .unwrap()
    })
}

fn request_auth_check(
    state: &ServerState,
    req: &Request<Body>,
) -> Result<Option<TokenClaims>, AnyError> {
    if state.needs_auth() {
        match request_validate_auth_cookie(state, req)? {
            Some(claims) => Ok(Some(claims)),
            None => Err(anyhow::anyhow!("Unauthorized")),
        }
    } else {
        Ok(None)
    }
}

async fn api_query(state: &ServerState, req: Request<Body>) -> Result<Response<Body>, AnyError> {
    let token_claims = match request_validate_auth_cookie(state, &req) {
        Ok(claims) => claims,
        Err(err) => {
            // Reset auth cookie on invalid request.
            return Ok(api_response(
                api_response_err(&err),
                vec![(header::SET_COOKIE, build_token_cookie("", true)?)],
            ));
        }
    };

    let body = hyper::body::to_bytes(req.into_body()).await?;
    let query: Query = serde_json::from_slice(&body)?;

    match &query {
        Query::ServerStatus | Query::Initialize(_) => {}
        _ => {
            if state.needs_auth() && token_claims.is_none() {
                return Err(anyhow::anyhow!("Unauthorized"));
            }
        }
    }

    tracing::trace!(?query, "running api query");

    // Handle queries that don't need authentication.

    let mut extra_headers = Vec::new();

    let app = &state.app;

    let res = match query {
        api::Query::ServerStatus => Ok(api::Reply::ServerStatus(api::ServerStatus {
            backend_initialized: app.db().is_some(),
        })),
        api::Query::Initialize(options) => {
            app.configure_backend(options.clone()).await?;
            let exp = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)?
                .add(std::time::Duration::from_secs(60 * 60 * 24))
                .as_secs();

            let key = &app.config().token_key;

            let new_token = TokenClaims {
                sub: "semantic".into(),
                exp,
                // NOTE: removing secrets from the config with purge_secrets.
                // This must not be removed since it would leak passwords etc in
                // the token.
                config: options.db.clone().purge_secrets(),
            }
            .encode(key)?;

            let cookie = build_token_cookie(&new_token, false)?;

            extra_headers.push((header::SET_COOKIE, cookie));
            extra_headers.push((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap()));
            extra_headers.push((
                header::ACCESS_CONTROL_ALLOW_METHODS,
                "POST".parse().unwrap(),
            ));

            let schema = app.load_schema()?;

            Ok(api::Reply::Initialize(schema))
        }
        api::Query::CloseBackend => {
            app.close_backend().await?;
            extra_headers.push((header::SET_COOKIE, build_token_cookie("", true)?));
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
