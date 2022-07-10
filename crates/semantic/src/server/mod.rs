use std::{net::SocketAddr, ops::Add, sync::Arc};

use anyhow::{anyhow, bail, Context, Result};
use axum::{extract::Extension, http};
use factordb::{
    prelude::{EntityContainer, Id},
    AnyError,
};
use headers::{Header, HeaderMapExt};
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
    pub address: String,

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

#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../target/ui"]
struct Asset;

pub async fn run_server(
    config: ServerConfig,
    runtime: tokio::runtime::Handle,
) -> Result<(), AnyError> {
    use axum::routing::{get, post};

    // Run the server like above...
    let addr: SocketAddr = config.address.parse().context(format!(
        "Invalid server interface specification '{}'",
        config.address
    ))?;

    let app = App::build(config.app.clone(), runtime).await?;

    let state = Arc::new(ServerState { config, app });

    let router = axum::Router::new()
        .route(
            "/api/query",
            post(handler_api_query)
                .route_layer(tower_http::compression::CompressionLayer::new().gzip(true)),
        )
        .route(
            "/api/upload-file",
            post(handler_blob_upload).options(cors_handler),
        )
        .route("/blob/*rest", get(handler_file_read))
        .nest("/assets", get(handler_assets))
        .fallback(get(handler_index))
        .layer(Extension(state))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    tracing::info!(interface=%addr, "starting web server");

    let server = axum::Server::bind(&addr).serve(router.into_make_service());

    server.await.map_err(|error| {
        tracing::error!(?error, "Server failed");
        error.into()
    })
}

async fn handler_assets(req: Request<Body>) -> Response<Body> {
    let path = req.uri().path().trim_start_matches('/');

    match Asset::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string();

            Response::builder()
                .header(hyper::header::CONTENT_TYPE, mime)
                .body(Body::from(file.data.as_ref().to_vec()))
                .unwrap()
        }
        None => not_found(),
    }
}

async fn handler_index() -> Response<Body> {
    let file = Asset::get("index.html").unwrap();

    let body = hyper::Body::from(file.data.as_ref().to_vec());

    Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/html")
        .body(body)
        .unwrap()
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

    let meta: FileUploadMetadata = if let Some(header) =
        req.headers().get(FileUploadMetadata::HEADER_NAME)
    {
        let raw = header
            .to_str()
            .context("Invalid (non-utf8) file metadata header")?;
        let decoded = base64::decode(raw)
            .context("Invalid file metadata header: not a valid base64 string")?;
        serde_json::from_slice(&decoded).context("Invalid file metadata header: invalid json")?
    } else {
        FileUploadMetadata::default()
    };

    tracing::trace!("fetching file upload body");
    let body = hyper::body::to_bytes(req.into_body()).await?;
    tracing::trace!(len=%body.len(), "file upload body retrieved");

    let item = state.app.upload_file(meta, body.to_vec()).await?;
    tracing::trace!(?item, "file created");
    Ok(item)
}

fn extract_file_range(req: &Request<Body>) -> Result<(Option<u64>, Option<u64>), AnyError> {
    let mut range_headers = req.headers().get_all(http::header::RANGE).iter().peekable();

    // Bail if no Range header present.
    if range_headers.peek().is_none() {
        return Ok((None, None));
    }

    let items = headers::Range::decode(&mut range_headers)?;
    let mut ranges = items.iter();
    let (start_bound, end_bound) = if let Some(x) = ranges.next() {
        x
    } else {
        return Ok((None, None));
    };

    if ranges.next().is_some() {
        bail!("Only a single range is supported");
    }

    let skip = match start_bound {
        std::ops::Bound::Included(x) => x,
        std::ops::Bound::Excluded(x) => x + 1,
        std::ops::Bound::Unbounded => 0,
    };

    let take = match end_bound {
        std::ops::Bound::Included(x) => Some(x - skip + 1),
        std::ops::Bound::Excluded(x) => Some(x - skip),
        std::ops::Bound::Unbounded => None,
    };

    Ok((Some(skip), take))
}

async fn serve_file(app: &App, req: &Request<Body>) -> Result<Response<Body>, AnyError> {
    #[derive(PartialEq, Eq, Debug)]
    enum Format {
        File,
        Video,
        Image,
        Preview,
    }

    let raw_path = req.uri().path().trim_start_matches('/');
    let parts = raw_path.split('/').collect::<Vec<&str>>();
    debug_assert_eq!(parts.get(0), Some(&"blob"));

    let format = match parts.get(1).map(|x| *x) {
        Some("file") => Format::File,
        Some("video") => Format::Video,
        Some("image") => Format::Image,
        Some("preview") => Format::Preview,
        Some(other) => bail!("Unknown file format: {}", other),
        None => {
            return Ok(not_found());
        }
    };

    let id_opt = parts
        .get(2)
        .and_then(|x| uuid::Uuid::parse_str(x).ok())
        .map(Id::from_uuid);

    let id = if let Some(id) = id_opt {
        id
    } else {
        return Ok(not_found());
    };

    tracing::trace!(%id, "serving file");
    let file_map = app.require_db()?.entity(id).await?;
    let file = semantic_core::base::File::try_from_map(file_map)?;

    let blob = app.require_blob()?;

    let (blob_path, mime) = match format {
        Format::Preview => {
            if let Some(uri) = &file.preview_image_blob_uri {
                let content = if let Some(c) = blob.get(uri).await? {
                    c
                } else {
                    return Ok(not_found());
                };

                // FIXME: don't hardcode image type?
                let res = Response::builder()
                    .header(http::header::CONTENT_TYPE, "image/webp")
                    .body(hyper::Body::from(content))?;
                return Ok(res);
            } else {
                // TODO: return some stub placeholder?
                return Ok(not_found());
            }
        }
        Format::File => {
            if let Some(uri) = file.blob_uri.clone() {
                (uri, file.mime_type.clone())
            } else {
                return Ok(not_found());
            }
        }
        // Prefer blob_uri_web if available, otherwise use the default.
        Format::Video => {
            if let Some(web) = file.blob_uri_web.clone() {
                (web, Some("video/webm".to_string()))
            } else if let Some(uri) = file.blob_uri.clone() {
                (uri, file.mime_type.clone())
            } else {
                return Ok(not_found());
            }
        }
        Format::Image => {
            if let Some(uri) = file.blob_uri_web.clone().or_else(|| file.blob_uri.clone()) {
                (uri, None)
            } else {
                return Ok(not_found());
            }
        }
    };

    let blob_info = blob
        .get_meta(&blob_path)
        .await?
        .ok_or_else(|| anyhow!("Blob not found: {blob_path}"))?;

    let mime = mime
        .or_else(|| {
            mime_guess::from_path(&blob_path)
                .first()
                .map(|x| x.as_ref().to_string())
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let size = blob_info.size;

    let (range_skip, range_take) = extract_file_range(&req)?;
    let (is_partial, actual_length) = match (range_skip, range_take) {
        (None, Some(take)) if take < size => (true, take),
        (Some(offset), None) if offset > 0 && offset <= size => (true, size - offset),
        (Some(offset), Some(take)) if offset <= size && take <= size - offset => (true, take),
        _ => (false, size),
    };

    let stream = blob.get_stream(&blob_path, range_skip, range_take).await?;

    let body = Body::wrap_stream(stream);

    let status = if is_partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };

    let mut res = Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, mime)
        .header(http::header::CONTENT_LENGTH, actual_length)
        .header(
            http::header::ACCEPT_RANGES,
            http::HeaderValue::from_static("bytes"),
        )
        .body(body)?;

    if let Some(mime) = &file.mime_type {
        res.headers_mut()
            .insert(http::header::CONTENT_TYPE, mime.parse()?);
    }
    if is_partial {
        let start = range_skip.unwrap_or(0);
        let end = range_take.map(|x| start + x).unwrap_or(size);
        let h = headers::ContentRange::bytes(start..end, Some(size))?;
        res.headers_mut().typed_insert(h);
    }

    Ok(res)
}

async fn handler_file_read(Extension(state): ServerContext, req: Request<Body>) -> Response<Body> {
    if let Err(res) = request_validate_auth_cookie_http(&state, &req) {
        return res;
    }
    if req.method() == Method::OPTIONS {
        let mut res = cors_response();
        res.headers_mut().append(
            http::header::ACCEPT_RANGES,
            http::header::HeaderValue::from_static("bytes"),
        );
        return res;
    }

    serve_file(&state.app, &req).await.unwrap_or_else(|error| {
        tracing::warn!(?error, path=%req.uri(), "could not serve file blob");
        internal_server_error("Could not serve file")
    })
}

fn api_error(err: &AnyError) -> ApiError {
    ApiError {
        message: err.to_string(),
        code: None,
        details: Some(format!("{:?}", err)),
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
        .ok_or_else(|| anyhow!("Invalid token"))?;

    if config.db.clone().purge_secrets() != claims.config {
        return Err(anyhow!("Invalid token"));
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
            None => Err(anyhow!("Unauthorized")),
        }
    } else {
        Ok(None)
    }
}

async fn api_query(state: &ServerState, req: Request<Body>) -> Result<Response<Body>, AnyError> {
    let token_claims = match request_validate_auth_cookie(&state, &req) {
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

    let jd = &mut serde_json::Deserializer::from_slice(&body);
    let query: Query = serde_path_to_error::deserialize(jd)?;

    match &query {
        Query::ServerStatus(()) | Query::Initialize(_) => {}
        _ => {
            if state.needs_auth() && token_claims.is_none() {
                return Err(anyhow!("Unauthorized"));
            }
        }
    }

    tracing::trace!(?query, "running api query");

    // Handle queries that don't need authentication.

    let mut extra_headers = Vec::new();

    let app = &state.app;

    let res = match query {
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

            let schema = app.load_schema().await?;

            Ok(api::Reply::Initialize(schema))
        }
        api::Query::CloseBackend(()) => {
            app.close_backend().await?;
            extra_headers.push((header::SET_COOKIE, build_token_cookie("", true)?));
            Ok(api::Reply::CloseBackend(()))
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
