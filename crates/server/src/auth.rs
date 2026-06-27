use async_trait::async_trait;
use http::{HeaderMap, HeaderName};
use semantic_app::Principal;

use crate::ServerError;

#[async_trait]
pub trait PrincipalResolver: Send + Sync + 'static {
    async fn resolve_http(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<Principal, ServerError>;

    async fn resolve_ws(&self, headers: &HeaderMap) -> std::result::Result<Principal, ServerError>;
}

#[derive(Clone, Default)]
pub struct NoAuthPrincipalResolver;

#[async_trait]
impl PrincipalResolver for NoAuthPrincipalResolver {
    async fn resolve_http(
        &self,
        _headers: &HeaderMap,
    ) -> std::result::Result<Principal, ServerError> {
        Ok(Principal::system())
    }

    async fn resolve_ws(
        &self,
        _headers: &HeaderMap,
    ) -> std::result::Result<Principal, ServerError> {
        Ok(Principal::system())
    }
}

/// Header-based test/integration scaffolding. This is not an authentication scheme.
#[derive(Clone)]
pub struct HeaderPrincipalResolver {
    header: HeaderName,
}

impl HeaderPrincipalResolver {
    pub fn new(header: HeaderName) -> Self {
        Self { header }
    }
}

#[async_trait]
impl PrincipalResolver for HeaderPrincipalResolver {
    async fn resolve_http(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<Principal, ServerError> {
        resolve_header(headers, &self.header)
    }

    async fn resolve_ws(&self, headers: &HeaderMap) -> std::result::Result<Principal, ServerError> {
        resolve_header(headers, &self.header)
    }
}

fn resolve_header(
    headers: &HeaderMap,
    header: &HeaderName,
) -> std::result::Result<Principal, ServerError> {
    let Some(value) = headers.get(header) else {
        return Err(semantic_app::AppError::AuthenticationRequired.into());
    };
    let value = value
        .to_str()
        .map_err(|err| ServerError::InvalidHeader(err.to_string()))?;
    if value.is_empty() {
        return Err(semantic_app::AppError::AuthenticationRequired.into());
    }
    Ok(Principal::user(value))
}
