use std::sync::Arc;

use semantic::{
    app::{AppConfig, DenoConfig},
    App,
};
use semantic_core::api::FileImportMetadata;

use super::{AsyncCliCommand, BackendOptions};

#[derive(clap::Parser)]
/// Import files into a semantic database.
pub struct CmdImportFiles {
    #[clap(flatten)]
    backend: BackendOptions,

    paths: Vec<std::path::PathBuf>,
}

impl AsyncCliCommand for CmdImportFiles {
    async fn run(self) -> Result<(), anyhow::Error> {
        let data_dir = App::default_data_dir().unwrap();

        let backend_config = self.backend.build_backend_config().unwrap();

        let app_config = AppConfig {
            backend: Some(backend_config),
            token_key: "".to_string(),
            deno: Some(DenoConfig {
                data_dir: data_dir.join("deno"),
                plugin_dir: None,
            }),
            tmp_dir: None,
        };

        let meta = FileImportMetadata {
            collection_id: None,
            tags: Vec::new(),
            url: None,
            parent: None,
        };

        let handle = tokio::runtime::Handle::current();
        let app = semantic::app::App::build(app_config, handle).await?;

        app.import_files(
            self.paths,
            meta,
            Arc::new(|path, _file| {
                tracing::info!(?path, "Imported file");
            }),
        )
        .await?;

        Result::<(), anyhow::Error>::Ok(()).expect("Export failed");

        tracing::info!("All paths imported");
        Ok(())
    }
}
