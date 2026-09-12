use std::path::PathBuf;

use napi_derive::napi;
use semantic_app::AppConfig;
use semantic_app::storage::StorageConfig;
use semantic_data::schema::DbOpenMode;

const DEFAULT_MAX_CONCURRENT_REQUESTS: u32 = 32;
const DEFAULT_MAX_BUFFERED_FILE_BYTES: u32 = 64 * 1024 * 1024;

#[napi(object)]
pub struct EmbeddedOptions {
    pub data_dir: String,
    pub db_uri: Option<String>,
    pub blob_uri: Option<String>,
    pub blob_password: Option<String>,
    pub mode: Option<String>,
    pub temp_dir: Option<String>,
    pub auto_analyze_media: Option<bool>,
    pub max_concurrent_requests: Option<u32>,
    pub max_buffered_file_bytes: Option<u32>,
}

pub struct ValidatedOptions {
    pub app: AppConfig,
    pub storage: StorageConfig,
    pub max_concurrent_requests: usize,
    pub max_buffered_file_bytes: usize,
}

impl EmbeddedOptions {
    pub fn validate(self) -> Result<ValidatedOptions, String> {
        if self.data_dir.is_empty() {
            return Err("dataDir must not be empty".to_string());
        }
        let data_dir = absolute_path(PathBuf::from(self.data_dir))?;
        let mut app = AppConfig::new().with_data_dir(data_dir);
        if let Some(temp_dir) = self.temp_dir {
            app = app.with_temp_dir(absolute_path(PathBuf::from(temp_dir))?);
        }
        if let Some(enabled) = self.auto_analyze_media {
            app = app.with_auto_analyze_media(enabled);
        }
        let mode = match self.mode.as_deref().unwrap_or("autoCreate") {
            "autoCreate" => DbOpenMode::AutoCreate,
            "openExisting" => DbOpenMode::OpenExisting,
            other => return Err(format!("unsupported open mode: {other}")),
        };
        let max_concurrent_requests = positive(
            self.max_concurrent_requests,
            DEFAULT_MAX_CONCURRENT_REQUESTS,
            "maxConcurrentRequests",
        )?;
        let max_buffered_file_bytes = positive(
            self.max_buffered_file_bytes,
            DEFAULT_MAX_BUFFERED_FILE_BYTES,
            "maxBufferedFileBytes",
        )?;
        Ok(ValidatedOptions {
            app,
            storage: StorageConfig {
                db_uri: self.db_uri,
                blob_uri: self.blob_uri,
                blob_password: self.blob_password,
                mode,
            },
            max_concurrent_requests,
            max_buffered_file_bytes,
        })
    }
}

fn positive(value: Option<u32>, default: u32, name: &str) -> Result<usize, String> {
    let value = value.unwrap_or(default);
    if value == 0 {
        return Err(format!("{name} must be positive"));
    }
    Ok(value as usize)
}

fn absolute_path(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|err| format!("could not resolve relative path: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> EmbeddedOptions {
        EmbeddedOptions {
            data_dir: "data".to_string(),
            db_uri: None,
            blob_uri: None,
            blob_password: None,
            mode: None,
            temp_dir: None,
            auto_analyze_media: None,
            max_concurrent_requests: None,
            max_buffered_file_bytes: None,
        }
    }

    #[test]
    fn validates_defaults_and_absolutizes_data_dir() {
        let validated = options().validate().unwrap();
        assert!(validated.app.data_dir.unwrap().is_absolute());
        assert_eq!(validated.max_concurrent_requests, 32);
        assert_eq!(validated.max_buffered_file_bytes, 64 * 1024 * 1024);
    }

    #[test]
    fn rejects_zero_limits_and_unknown_modes() {
        let mut value = options();
        value.max_concurrent_requests = Some(0);
        assert!(
            value
                .validate()
                .err()
                .unwrap()
                .contains("maxConcurrentRequests")
        );

        let mut value = options();
        value.mode = Some("readonly".to_string());
        assert!(value.validate().err().unwrap().contains("readonly"));
    }
}
