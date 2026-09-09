use std::path::PathBuf;
#[cfg(feature = "logfs")]
use std::sync::Arc;

use objstore::ObjStoreBuilder;
use semantic_app::{AppConfig, AppError, DbOpenRequest, DbScopeId, Principal, SemanticApp};
use semantic_data::schema::DbOpenMode;

use crate::{SemanticServer, ServerError};

/// Resolve startup selectors, preserving an explicit database choice.
pub fn resolve_db_uri(
    db_uri: Option<String>,
    blob_uri: &str,
    app_config: &AppConfig,
) -> Result<String, ServerError> {
    if let Some(uri) = db_uri {
        return Ok(uri);
    }
    if is_logfs_blob_uri(blob_uri) {
        eprintln!(
            "No database URI specified; automatically selected 'log:<blob>' because the blob store uses logfs. Database records and blobs will share one open store."
        );
        return Ok("log:<blob>".into());
    }
    let path = app_config.default_db_path();
    let path = path
        .to_str()
        .ok_or_else(|| AppError::InvalidRequest("database URI requires a UTF-8 path".into()))?;
    Ok(format!("redb:{path}"))
}

pub fn is_logfs_blob_uri(uri: &str) -> bool {
    uri.split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("logfs"))
}

/// Configuration shared by database backends that open a local file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDbConfig {
    pub path: PathBuf,
}

impl LocalDbConfig {
    /// Parse `scheme:path`, preserving paths literally without URL decoding or normalization.
    pub fn from_uri(scheme: &str, uri: &str) -> Result<Self, AppError> {
        let (actual_scheme, path) = uri.split_once(':').ok_or_else(|| {
            AppError::InvalidRequest("database URI must contain a scheme followed by ':'".into())
        })?;
        if actual_scheme != scheme {
            return Err(AppError::InvalidRequest(format!(
                "expected database URI scheme '{scheme}'"
            )));
        }
        if path.starts_with("//") {
            return Err(AppError::InvalidRequest(format!(
                "use '{scheme}:PATH' for a local database URI"
            )));
        }
        if path.is_empty() || path.contains('\0') || path.starts_with('<') {
            return Err(AppError::InvalidRequest(format!(
                "{scheme} database URI requires a local path"
            )));
        }
        Ok(Self { path: path.into() })
    }

    #[cfg(any(feature = "redb", feature = "logfs"))]
    pub(crate) fn ensure_parent(&self, mode: DbOpenMode) -> Result<(), AppError> {
        if mode == DbOpenMode::AutoCreate
            && let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::Db(semantic_db_core::DbError::Storage(format!(
                    "create database parent '{}': {err}",
                    parent.display()
                )))
            })?;
        }
        Ok(())
    }
}

impl SemanticServer {
    /// Open the default database and blob store before accepting requests.
    ///
    /// `log:<blob>` shares one physical object store between the WAL and blobs.
    /// Other database schemes retain the blob store's existing root key layout.
    /// `blob_password` supplies logfs encryption credentials without embedding
    /// them in the URI. `None` opens without encryption; this method never prompts.
    pub async fn from_uris(
        db_uri: String,
        blob_uri: String,
        blob_password: Option<String>,
        app_config: AppConfig,
    ) -> Result<Self, ServerError> {
        if blob_password.is_some() && !is_logfs_blob_uri(&blob_uri) {
            return Err(AppError::InvalidRequest(
                "blob-store passwords are only supported for logfs".into(),
            )
            .into());
        }
        #[cfg(not(feature = "logfs"))]
        if blob_password.is_some() {
            return Err(AppError::InvalidRequest("logfs support is not enabled".into()).into());
        }
        let mut stores = ObjStoreBuilder::new();
        stores.register_provider(objstore_fs::FsProvider::new());
        #[cfg(feature = "logfs")]
        stores.register_provider(crate::logfs_blob::LogFsBlobProvider::new(blob_password));

        let blob_store = tokio::task::spawn_blocking(move || stores.build(&blob_uri))
            .await
            .map_err(|err| {
                AppError::InvalidRequest(format!("object store open task failed: {err}"))
            })?
            .map_err(AppError::from)?;

        let scope_id = DbScopeId::new("default");
        let builder = SemanticApp::builder().with_config(app_config);
        #[cfg(feature = "redb")]
        let builder = builder.with_provider(crate::RedbDbProvider);
        #[cfg(feature = "logfs")]
        let builder = builder.with_provider(crate::LogFsDbProvider);

        // Only expose the shared backend when selected at startup: every scope
        // using it then receives the same database instance and WAL writer.
        #[cfg(feature = "logfs")]
        let (builder, blob_store) =
            if db_uri.split_once(':').map(|(scheme, _)| scheme) == Some("log") {
                let provider = crate::LogDbProvider::new(Arc::clone(&blob_store));
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
                    uri: db_uri,
                    mode: DbOpenMode::AutoCreate,
                },
            )
            .with_default_file_store(scope_id, blob_store)
            .register_builtin_commands()?
            .build()?;
        app.scopes()
            .resolve_scope(&Principal::system(), None, None)
            .await?;
        Ok(Self::new(app))
    }
}
