use std::collections::BTreeMap;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use bytes::{Bytes, BytesMut};
use futures_util::TryStreamExt as _;
use http::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, HeaderName, HeaderValue,
    RANGE,
};
use http::{HeaderMap, StatusCode};
use semantic_app::{AppRequestContext, FileContent, FileCreateRequest};
use semantic_data::value::{Object, Value};

use crate::ServerError;
use crate::router::{ServerState, scope_from_parts};

pub async fn upload_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Response {
    let ctx = match request_context(&state, &headers, &query).await {
        Ok(ctx) => ctx,
        Err(err) => return server_error_response(err),
    };
    let entity = match file_entity_from_headers(&headers, &state.config.file_entity_header) {
        Ok(entity) => entity,
        Err(err) => return server_error_response(err),
    };
    let request = FileCreateRequest {
        scope_id: None,
        id: header_string(&headers, &state.config.file_id_header),
        path: header_string(&headers, &state.config.file_path_header),
        filename: content_disposition_filename(&headers),
        mime_type: content_type(&headers),
        entity,
        content: FileContent::Bytes(body),
    };

    match state.app.files().create(&ctx, request).await {
        Ok(record) => {
            let mut out = Object::new();
            out.insert("id", Value::String(record.id));
            out.insert("collection", Value::String(record.collection));
            out.insert("object", Value::Object(record.object));
            (StatusCode::CREATED, Json(Value::Object(out))).into_response()
        }
        Err(err) => app_error_response(err),
    }
}

pub async fn download_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Query(query): Query<BTreeMap<String, String>>,
    Path(id): Path<String>,
) -> Response {
    let ctx = match request_context(&state, &headers, &query).await {
        Ok(ctx) => ctx,
        Err(err) => return server_error_response(err),
    };
    let file = match state.app.files().read(&ctx, None, id).await {
        Ok(file) => file,
        Err(err) => return app_error_response(err),
    };
    let bytes = match file.stream.try_collect::<BytesMut>().await {
        Ok(bytes) => bytes.freeze(),
        Err(err) => return app_error_response(err),
    };
    let total_len = bytes.len() as u64;
    let selected = match headers.get(RANGE) {
        Some(range) => match parse_range_header(range, total_len) {
            Ok(range) => Some(range),
            Err(err) => return app_error_response(err),
        },
        None => None,
    };

    let (status, body, content_range) = match selected {
        Some(range) => {
            let start = range.start as usize;
            let end = range.end as usize;
            let body = bytes.slice(start..=end);
            (
                StatusCode::PARTIAL_CONTENT,
                body,
                Some(format!("bytes {}-{}/{}", range.start, range.end, total_len)),
            )
        }
        None => (StatusCode::OK, bytes, None),
    };

    let mut builder = Response::builder().status(status);
    let headers_out = builder.headers_mut().expect("response builder headers");
    headers_out.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers_out.insert(
        CONTENT_TYPE,
        header_value(
            file.mime_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
        ),
    );
    headers_out.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string()).expect("valid content length"),
    );
    if let Some(content_range) = content_range {
        headers_out.insert(CONTENT_RANGE, header_value(&content_range));
    }
    if let Some(hash) = file.content_hash_sha256 {
        headers_out.insert(ETAG, header_value(&format!("sha256:{hash}")));
    }

    builder
        .body(Body::from(body))
        .expect("file response should be buildable")
}

async fn request_context(
    state: &ServerState,
    headers: &HeaderMap,
    query: &BTreeMap<String, String>,
) -> std::result::Result<AppRequestContext, ServerError> {
    let principal = state.resolver.resolve_http(headers).await?;
    let request_scope = scope_from_parts(headers, query, &state.config);
    Ok(AppRequestContext {
        app: state.app.clone(),
        principal,
        session: None,
        request_scope,
    })
}

fn file_entity_from_headers(
    headers: &HeaderMap,
    header: &HeaderName,
) -> std::result::Result<Object, ServerError> {
    let Some(value) = headers.get(header) else {
        return Ok(Object::new());
    };
    let value = value
        .to_str()
        .map_err(|err| ServerError::InvalidHeader(err.to_string()))?;
    let value = serde_json::from_str::<Value>(value)
        .map_err(|err| ServerError::InvalidHeader(err.to_string()))?;
    match value {
        Value::Object(object) => Ok(object),
        _ => Err(ServerError::InvalidHeader(format!(
            "header '{}' must contain a JSON object",
            header
        ))),
    }
}

fn header_string(headers: &HeaderMap, header: &HeaderName) -> Option<String> {
    headers
        .get(header)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn content_type(headers: &HeaderMap) -> Option<String> {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn content_disposition_filename(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(http::header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())?;
    value.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        if name.eq_ignore_ascii_case("filename") {
            Some(value.trim_matches('"').to_string())
        } else {
            None
        }
    })
}

#[derive(Clone, Copy)]
struct ByteRange {
    start: u64,
    end: u64,
}

fn parse_range_header(
    value: &HeaderValue,
    total_len: u64,
) -> std::result::Result<ByteRange, semantic_app::AppError> {
    let raw = value
        .to_str()
        .map_err(|err| semantic_app::AppError::InvalidRange(err.to_string()))?;
    let Some(spec) = raw.strip_prefix("bytes=") else {
        return Err(semantic_app::AppError::InvalidRange(raw.to_string()));
    };
    if spec.contains(',') {
        return Err(semantic_app::AppError::InvalidRange(
            "multiple ranges are not supported".to_string(),
        ));
    }
    if total_len == 0 {
        return Err(semantic_app::AppError::InvalidRange(
            "empty file has no satisfiable range".to_string(),
        ));
    }

    if let Some(suffix) = spec.strip_prefix('-') {
        let suffix_len = suffix
            .parse::<u64>()
            .map_err(|_| semantic_app::AppError::InvalidRange(raw.to_string()))?;
        if suffix_len == 0 {
            return Err(semantic_app::AppError::InvalidRange(raw.to_string()));
        }
        let start = total_len.saturating_sub(suffix_len);
        return Ok(ByteRange {
            start,
            end: total_len - 1,
        });
    }

    let (start, end) = spec
        .split_once('-')
        .ok_or_else(|| semantic_app::AppError::InvalidRange(raw.to_string()))?;
    let start = start
        .parse::<u64>()
        .map_err(|_| semantic_app::AppError::InvalidRange(raw.to_string()))?;
    let end = if end.is_empty() {
        total_len - 1
    } else {
        end.parse::<u64>()
            .map_err(|_| semantic_app::AppError::InvalidRange(raw.to_string()))?
    };
    if start >= total_len || end < start {
        return Err(semantic_app::AppError::InvalidRange(raw.to_string()));
    }
    Ok(ByteRange {
        start,
        end: end.min(total_len - 1),
    })
}

fn server_error_response(err: ServerError) -> Response {
    match err {
        ServerError::InvalidHeader(_) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
        ServerError::App(err) => app_error_response(err),
        ServerError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

fn app_error_response(err: semantic_app::AppError) -> Response {
    match err {
        semantic_app::AppError::FileNotFound(_) => {
            (StatusCode::NOT_FOUND, err.to_string()).into_response()
        }
        semantic_app::AppError::InvalidRange(_) => {
            (StatusCode::RANGE_NOT_SATISFIABLE, err.to_string()).into_response()
        }
        semantic_app::AppError::InvalidFileEntity(_)
        | semantic_app::AppError::InvalidFileMetadata(_)
        | semantic_app::AppError::InvalidRequest(_) => {
            (StatusCode::BAD_REQUEST, err.to_string()).into_response()
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

fn header_value(value: &str) -> HeaderValue {
    HeaderValue::from_str(value)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"))
}
