use std::path::PathBuf;

#[cfg(any(feature = "storage-redb", feature = "storage-logfs"))]
use semantic_data::schema::DbOpenMode;

use crate::AppError;

pub fn is_logfs_blob_uri(uri: &str) -> bool {
    uri.split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("logfs"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDbConfig {
    pub path: PathBuf,
}

impl LocalDbConfig {
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

    #[cfg(any(feature = "storage-redb", feature = "storage-logfs"))]
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
