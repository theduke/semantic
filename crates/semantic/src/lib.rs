pub mod app;
mod blobstore;
pub mod db;
mod file_import;
mod jobs;
pub mod plugin;
pub mod server;
pub mod util;

pub use app::App;
pub use util::api_client::ApiClient;

#[cfg(test)]
async fn test_app(name: &str, handle: &tokio::runtime::Handle) -> (std::path::PathBuf, App) {
    use semantic_core::api::{BackendCryptoConfig, DbConfig};

    let data_dir = std::env::temp_dir()
        .join("semantic")
        .join("tests")
        .join(name);
    if data_dir.is_dir() {
        std::fs::remove_dir_all(&data_dir).unwrap();
    }

    let token_key = App::random_token_key();

    let app_config = app::AppConfig {
        backend: Some(semantic_core::api::BackendConfig {
            db: DbConfig::Crypto(BackendCryptoConfig {
                data_path: Some(data_dir.join("db").to_str().unwrap().to_string()),
                key: "key".to_string(),
                key_iterations: Some(1),
                salt: None,
                raw: false,
                offset: None,
                full_index_write_interval: None,
                readonly: false,
            }),
            idle_timeout: None,
        }),
        token_key,
        deno: Some(crate::app::DenoConfig {
            data_dir: data_dir.join("deno"),
            plugin_dir: None,
        }),
        tmp_dir: None,
    };

    let app = App::build(app_config.clone(), handle.clone())
        .await
        .unwrap();

    (data_dir, app)
}
