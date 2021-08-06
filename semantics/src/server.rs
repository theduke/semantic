use std::{convert::Infallible, net::SocketAddr};

use anyhow::{Context, Result};
use factordb::AnyError;
use hyper::{
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Body, Method, Request, Response, Server, StatusCode,
};

use semantics_core::api::{ApiError, ApiResponse, BackendConfig, Query, Reply};

use crate::app::App;

pub async fn run_server(app: App) {
    // A `MakeService` that produces a `Service` to handle each connection.
    let make_service = make_service_fn(move |conn: &AddrStream| {
        let app = app.clone();

        let addr = conn.remote_addr();
        let service = service_fn(move |req| handler(app.clone(), addr, req));

        // Return the service to hyper.
        async move { Ok::<_, Infallible>(service) }
    });

    // Run the server like above...
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    tracing::info!(interface=%addr, "starting web server");

    let server = Server::bind(&addr).serve(make_service);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

async fn handler(
    app: App,
    _addr: SocketAddr,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    tracing::trace!(method=?req.method(), path=%req.uri(), "handling request");

    let res = match req.uri().path() {
        "/api/query" if req.method() == Method::POST => handler_api_query(&app, req).await,
        "/api/upload-file" if req.method() == Method::OPTIONS => handler_file_upload_cors(),
        "/api/upload-file" if req.method() == Method::POST => handler_file_upload(&app, req).await,
        path if req.method() == Method::GET && path.starts_with("/blob/") => {
            let blob_path = path.strip_prefix("/blob/").unwrap();
            handler_blob_read(&app, blob_path).await
        }
        _other => not_found(),
    };

    Ok(res)
}

fn handler_file_upload_cors() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .header(
            hyper::header::ACCESS_CONTROL_ALLOW_HEADERS,
            format!(
                "{},content-type",
                semantics_core::api::FileUploadMetadata::HEADER_NAME
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

async fn handler_file_upload(app: &App, req: Request<Body>) -> Response<Body> {
    tracing::trace!("handler_blob_upload");
    let reply = match file_upload(app, req).await {
        Ok(item) => ApiResponse::Ok(item),
        Err(err) => {
            tracing::warn!(?err, "file upload failed");
            ApiResponse::Err(api_error(&err))
        }
    };

    tracing::info!(?reply, "blob upload reply");

    api_response(reply)
}

async fn file_upload(
    app: &App,
    req: Request<Body>,
) -> Result<semantics_core::base::TypedFile, AnyError> {
    // FIXME: check authentication

    tracing::trace!("file upload started");

    use semantics_core::api::FileUploadMetadata;

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

async fn handler_blob_read(app: &App, blob_path: &str) -> Response<Body> {
    let blob = if let Some(b) = app.blob() {
        b
    } else {
        return internal_server_error("Blobstore not initialized");
    };

    match blob.get(blob_path).await {
        Ok(Some(data)) => Response::builder()
            .status(StatusCode::OK)
            .body(data.into())
            .unwrap(),
        Ok(None) => not_found(),
        Err(err) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(format!("Error: {}", err).into_bytes().into())
            .unwrap(),
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

fn api_response<T>(res: ApiResponse<T>) -> Response<Body>
where
    T: serde::Serialize,
{
    // TODO: no unwrap?
    let res_json = serde_json::to_vec(&res).unwrap();

    Response::builder()
        .status(StatusCode::OK)
        .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
        .header(hyper::header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
        .body(Body::from(res_json))
        .unwrap()
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

async fn handler_api_query(app: &App, req: Request<Body>) -> Response<Body> {
    match api_query(app, req).await {
        Ok(res) => res,
        Err(err) => api_response(api_response_err(&err)),
    }
}

const TOKEN_COOKIE_NAME: &'static str = "token";

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

    if let Query::Initialize { config } = &query {
        app.configure_backend(config.clone()).await?;
        // TODO: no unwrap?
        let res_json = serde_json::to_vec(&Reply::Initialize)?;

        let exp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)?
            .as_secs();

        let key = &app.config().token_key;

        let new_token = TokenClaims {
            sub: "semantic".into(),
            exp,
            config: config.clone(),
        }
        .encode(key)?;

        let cookie = format!("{}={}; HttpOnly", TOKEN_COOKIE_NAME, new_token);

        let res = Response::builder()
            .status(StatusCode::OK)
            .header(hyper::header::SET_COOKIE, cookie)
            .header(hyper::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(hyper::header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
            .body(Body::from(res_json))
            .unwrap();

        return Ok(res);
    };

    let reply = app
        .run_api_query(query, token.is_some())
        .await
        .map_err(|err| {
            tracing::error!(?err, "api query failed");
            err
        })?;
    let res = api_response(ApiResponse::Ok(reply));
    Ok(res)
}
