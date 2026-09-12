use semantic_app::AppConfig;
use semantic_app::storage::{StorageConfig, open_app, resolve_storage};
use semantic_data::schema::DbOpenMode;

use crate::{SemanticServer, ServerError};

pub fn resolve_db_uri(
    db_uri: Option<String>,
    blob_uri: &str,
    app_config: &AppConfig,
) -> Result<String, ServerError> {
    Ok(resolve_storage(
        app_config,
        StorageConfig {
            db_uri,
            blob_uri: Some(blob_uri.to_owned()),
            ..Default::default()
        },
    )?
    .db_uri()
    .to_owned())
}

impl SemanticServer {
    pub async fn from_uris(
        db_uri: String,
        blob_uri: String,
        blob_password: Option<String>,
        app_config: AppConfig,
    ) -> Result<Self, ServerError> {
        let storage = resolve_storage(
            &app_config,
            StorageConfig {
                db_uri: Some(db_uri),
                blob_uri: Some(blob_uri),
                blob_password,
                mode: DbOpenMode::AutoCreate,
            },
        )?;
        Ok(Self::new(open_app(app_config, storage).await?))
    }
}
