use futures::future::LocalBoxFuture;
#[cfg(feature = "desktop")]
use semantic_app::{AppError, DbOpenRequest, DbProvider};
use semantic_app::{
    AppRequestContext, AppSession, DbScopeId, FileContent, FileCreateRequest, Principal,
    SemanticApp,
};
#[cfg(feature = "desktop")]
use semantic_data::schema::DbOpenMode;
use semantic_data::value::Value;
use semantic_rpc::{
    RpcClientDyn, RpcClientError, RpcRequest,
    client::resolve_response,
    file::{
        FileUploadPhase, FileUploadProgressSender, FileUploadRequest, FileUploadResponse,
        emit_progress,
    },
};
use std::sync::Arc;

#[cfg(feature = "desktop")]
struct RedbDbProvider;

#[cfg(feature = "desktop")]
#[async_trait::async_trait]
impl DbProvider for RedbDbProvider {
    fn scheme(&self) -> &str {
        "redb"
    }

    async fn open(
        &self,
        request: DbOpenRequest,
        _principal: &Principal,
    ) -> std::result::Result<Arc<dyn semantic_app::SemanticDb>, AppError> {
        let path = request.uri.strip_prefix("redb://").ok_or_else(|| {
            AppError::InvalidRequest(format!("invalid redb uri '{}'", request.uri))
        })?;
        if path.is_empty() {
            return Err(AppError::InvalidRequest(
                "invalid redb uri: missing database path".to_string(),
            ));
        }
        let backend = semantic_db_redb::open_backend(path, request.mode)?;
        Ok(Arc::new(semantic_db_core::Db::new(backend)))
    }
}

#[derive(Clone)]
pub struct EmbeddedRpcClient {
    app: SemanticApp,
    session: Arc<AppSession>,
    principal: Principal,
    scope_id: DbScopeId,
}

#[cfg(feature = "desktop")]
#[derive(Clone)]
pub struct EmbeddedAppHandle {
    pub client: semantic_rpc::RpcClient,
    pub app: SemanticApp,
    pub session: Arc<AppSession>,
    pub principal: Principal,
    pub scope_id: DbScopeId,
}

impl EmbeddedRpcClient {
    pub fn new(
        app: SemanticApp,
        session: Arc<AppSession>,
        principal: Principal,
        scope_id: DbScopeId,
    ) -> Self {
        Self {
            app,
            session,
            principal,
            scope_id,
        }
    }
}

impl RpcClientDyn for EmbeddedRpcClient {
    fn invoke_value(
        &self,
        command: String,
        payload: Value,
    ) -> LocalBoxFuture<'static, std::result::Result<Value, RpcClientError>> {
        let app = self.app.clone();
        let session = Arc::clone(&self.session);
        let principal = self.principal.clone();
        let scope_id = self.scope_id.clone();
        Box::pin(async move {
            let ctx = AppRequestContext {
                app: app.clone(),
                principal,
                session: Some(session),
                request_scope: Some(scope_id),
            };
            let response = app
                .invoke(
                    ctx,
                    RpcRequest {
                        id: semantic_rpc::client::next_request_id(),
                        command,
                        payload,
                    },
                )
                .await;
            resolve_response(response)
        })
    }

    fn upload_file(
        &self,
        request: FileUploadRequest,
        progress: Option<FileUploadProgressSender>,
    ) -> LocalBoxFuture<'static, std::result::Result<FileUploadResponse, RpcClientError>> {
        let app = self.app.clone();
        let session = Arc::clone(&self.session);
        let principal = self.principal.clone();
        let scope_id = self.scope_id.clone();
        Box::pin(async move {
            let total = request.bytes.len() as u64;
            emit_progress(&progress, FileUploadPhase::Preparing, 0, Some(total));
            let ctx = AppRequestContext {
                app: app.clone(),
                principal,
                session: Some(session),
                request_scope: Some(scope_id),
            };
            emit_progress(&progress, FileUploadPhase::Uploading, total, Some(total));
            emit_progress(&progress, FileUploadPhase::Finalizing, total, Some(total));
            let record = app
                .files()
                .create(
                    &ctx,
                    FileCreateRequest {
                        scope_id: request.scope_id.map(DbScopeId::new),
                        id: request.id,
                        filestore_locator: None,
                        filename: request.filename,
                        mime_type: request.mime_type,
                        entity: request.entity,
                        content: FileContent::Bytes(request.bytes),
                    },
                )
                .await
                .map_err(|err| RpcClientError::Remote("app_error".to_string(), err.to_string()))?;
            let response = FileUploadResponse {
                id: record.id,
                collection: record.collection,
                object: record.object,
            };
            emit_progress(&progress, FileUploadPhase::Done, total, Some(total));
            Ok(response)
        })
    }

    fn file_url(&self, id: &str) -> Option<String> {
        Some(format!(
            "semantic-file://localhost/{}",
            percent_encode_path(id)
        ))
    }
}

fn percent_encode_path(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push('%');
                encoded.push(hex_char(byte >> 4));
                encoded.push(hex_char(byte & 0x0f));
            }
        }
    }
    encoded
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => unreachable!("hex nibble out of range"),
    }
}

#[cfg(feature = "desktop")]
pub fn build_embedded_client(
    db_path: impl AsRef<std::path::Path>,
) -> std::result::Result<(semantic_rpc::RpcClient, String), String> {
    let blob_uri = semantic_app::AppConfig::from_env().default_blob_uri()?;
    build_embedded_client_with_blob_store(db_path, blob_uri)
}

#[cfg(feature = "desktop")]
pub fn build_embedded_client_with_blob_store(
    db_path: impl AsRef<std::path::Path>,
    blob_uri: String,
) -> std::result::Result<(semantic_rpc::RpcClient, String), String> {
    let handle = build_embedded_handle_with_blob_store(db_path, blob_uri)?;
    Ok((handle.client, handle.scope_id.to_string()))
}

#[cfg(feature = "desktop")]
pub fn build_embedded_handle_with_blob_store(
    db_path: impl AsRef<std::path::Path>,
    blob_uri: String,
) -> std::result::Result<EmbeddedAppHandle, String> {
    let scope_id = DbScopeId::new("local");
    let db_uri = format!("redb://{}", db_path.as_ref().to_string_lossy());
    let app = SemanticApp::builder()
        .with_provider(RedbDbProvider)
        .with_default_scope_request(
            scope_id.clone(),
            DbOpenRequest {
                uri: db_uri,
                mode: DbOpenMode::AutoCreate,
            },
        )
        .with_default_file_store_uri(scope_id.clone(), blob_uri)
        .register_builtin_commands()
        .map_err(|err| err.to_string())?
        .build()
        .map_err(|err| err.to_string())?;
    let session = app.new_session("semantic-ui");
    let principal = Principal::system();
    let client = semantic_rpc::RpcClient::new(EmbeddedRpcClient::new(
        app.clone(),
        Arc::clone(&session),
        principal.clone(),
        scope_id.clone(),
    ));
    Ok(EmbeddedAppHandle {
        client,
        app,
        session,
        principal,
        scope_id,
    })
}

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn embedded_client_loads_base_catalog() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("semantic-ui-standalone-{suffix}.redb"));
        let (client, scope_id) = build_embedded_client(&db_path).unwrap();

        let catalog = futures::executor::block_on(semantic_ui_core::ui_catalog::load_catalog(
            client,
            Some(scope_id),
            None,
        ))
        .unwrap();

        assert!(catalog.class_by_id("semantic.base.person").is_some());
        assert!(catalog.class_by_id("semantic.filestore.file").is_some());
    }
}
