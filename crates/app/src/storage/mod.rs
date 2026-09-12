mod uri;

#[cfg(feature = "storage-logfs")]
mod log;
#[cfg(feature = "storage-logfs")]
mod logfs;
#[cfg(feature = "storage-logfs")]
mod logfs_blob;
#[cfg(feature = "storage-redb")]
mod redb;

#[cfg(feature = "storage-logfs")]
use std::sync::Arc;

use objstore::ObjStoreBuilder;
use semantic_data::schema::DbOpenMode;

use crate::{AppConfig, AppError, DbOpenRequest, DbScopeId, Principal, SemanticApp};

#[cfg(feature = "storage-logfs")]
pub use log::{LogDbConfig, LogDbProvider};
#[cfg(feature = "storage-logfs")]
pub use logfs::LogFsDbProvider;
#[cfg(feature = "storage-redb")]
pub use redb::RedbDbProvider;
pub use uri::{LocalDbConfig, is_logfs_blob_uri};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbUriSelection {
    Explicit,
    DefaultRedb,
    SharedBlob,
}

#[derive(Clone)]
pub struct StorageConfig {
    pub db_uri: Option<String>,
    pub blob_uri: Option<String>,
    pub blob_password: Option<String>,
    pub mode: DbOpenMode,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_uri: None,
            blob_uri: None,
            blob_password: None,
            mode: DbOpenMode::AutoCreate,
        }
    }
}

impl std::fmt::Debug for StorageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageConfig")
            .field("db_uri", &self.db_uri.as_deref().map(redact_uri))
            .field("blob_uri", &self.blob_uri.as_deref().map(redact_uri))
            .field(
                "blob_password",
                &self.blob_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field("mode", &self.mode)
            .finish()
    }
}

#[derive(Clone)]
pub struct ResolvedStorageConfig {
    db_uri: String,
    blob_uri: String,
    blob_password: Option<String>,
    mode: DbOpenMode,
    selection: DbUriSelection,
}

impl ResolvedStorageConfig {
    pub fn db_uri(&self) -> &str {
        &self.db_uri
    }
    pub fn blob_uri(&self) -> &str {
        &self.blob_uri
    }
    pub fn mode(&self) -> DbOpenMode {
        self.mode
    }
    pub fn db_uri_selection(&self) -> DbUriSelection {
        self.selection
    }
    pub fn with_blob_password(mut self, password: Option<String>) -> Self {
        self.blob_password = password;
        self
    }
}

impl std::fmt::Debug for ResolvedStorageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedStorageConfig")
            .field("db_uri", &redact_uri(&self.db_uri))
            .field("blob_uri", &redact_uri(&self.blob_uri))
            .field(
                "blob_password",
                &self.blob_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field("mode", &self.mode)
            .field("selection", &self.selection)
            .finish()
    }
}

fn redact_uri(uri: &str) -> String {
    uri.split_once('?')
        .map_or_else(|| uri.to_owned(), |(base, _)| format!("{base}?[REDACTED]"))
}

pub fn resolve_storage(
    app_config: &AppConfig,
    storage: StorageConfig,
) -> Result<ResolvedStorageConfig, AppError> {
    let blob_uri = match storage.blob_uri {
        Some(uri) => uri,
        None => app_config
            .default_blob_uri()
            .map_err(AppError::InvalidRequest)?,
    };
    let (db_uri, selection) = match storage.db_uri {
        Some(uri) => (uri, DbUriSelection::Explicit),
        None if is_logfs_blob_uri(&blob_uri) => ("log:<blob>".into(), DbUriSelection::SharedBlob),
        None => {
            let path = app_config.default_db_path();
            let path = path.to_str().ok_or_else(|| {
                AppError::InvalidRequest("database URI requires a UTF-8 path".into())
            })?;
            (format!("redb:{path}"), DbUriSelection::DefaultRedb)
        }
    };
    Ok(ResolvedStorageConfig {
        db_uri,
        blob_uri,
        blob_password: storage.blob_password,
        mode: storage.mode,
        selection,
    })
}

pub async fn open_app(
    app_config: AppConfig,
    storage: ResolvedStorageConfig,
) -> Result<SemanticApp, AppError> {
    if storage.blob_password.is_some() && !is_logfs_blob_uri(&storage.blob_uri) {
        return Err(AppError::InvalidRequest(
            "blob-store passwords are only supported for logfs".into(),
        ));
    }
    #[cfg(not(feature = "storage-logfs"))]
    if is_logfs_blob_uri(&storage.blob_uri)
        || storage.db_uri.starts_with("logfs:")
        || storage.db_uri.starts_with("log:")
    {
        return Err(AppError::InvalidRequest(
            "logfs support is not enabled".into(),
        ));
    }
    #[cfg(not(feature = "storage-redb"))]
    if storage.db_uri.starts_with("redb:") {
        return Err(AppError::InvalidRequest(
            "redb support is not enabled".into(),
        ));
    }

    let mut stores = ObjStoreBuilder::new();
    stores.register_provider(objstore_fs::FsProvider::new());
    #[cfg(feature = "storage-logfs")]
    stores.register_provider(logfs_blob::LogFsBlobProvider::new(storage.blob_password));
    let blob_uri = storage.blob_uri;
    let blob_store = tokio::task::spawn_blocking(move || stores.build(&blob_uri))
        .await
        .map_err(|err| AppError::InvalidRequest(format!("object store open task failed: {err}")))?
        .map_err(AppError::from)?;

    let scope_id = DbScopeId::new("default");
    let builder = SemanticApp::builder().with_config(app_config);
    #[cfg(feature = "storage-redb")]
    let builder = builder.with_provider(RedbDbProvider);
    #[cfg(feature = "storage-logfs")]
    let builder = builder.with_provider(LogFsDbProvider);
    #[cfg(feature = "storage-logfs")]
    let (builder, blob_store) =
        if storage.db_uri.split_once(':').map(|(scheme, _)| scheme) == Some("log") {
            let provider = LogDbProvider::new(Arc::clone(&blob_store));
            let blobs = Arc::new(objstore::wrapper::prefix::PrefixObjStore::new(
                "blob/default",
                blob_store,
            )) as objstore::DynObjStore;
            (builder.with_provider(provider), blobs)
        } else {
            (builder, blob_store)
        };

    let app = builder
        .with_default_scope_request(
            scope_id.clone(),
            DbOpenRequest {
                uri: storage.db_uri,
                mode: storage.mode,
            },
        )
        .with_default_file_store(scope_id, blob_store)
        .register_builtin_commands()?
        .build()?;
    app.scopes()
        .resolve_scope(&Principal::system(), None, None)
        .await?;
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_and_debug_redaction() {
        let app = AppConfig::new().with_data_dir("data");
        let resolved = resolve_storage(
            &app,
            StorageConfig {
                blob_uri: Some("logfs:///x?key=secret".into()),
                blob_password: Some("actual-secret-value".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(resolved.db_uri_selection(), DbUriSelection::SharedBlob);
        let debug = format!("{resolved:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("actual-secret-value"));
    }
}
