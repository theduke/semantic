use std::path::PathBuf;

use anyhow::bail;
use semantic::app::AppConfig;

use crate::BackendOptions;

/// Delete entities.
#[derive(clap::Parser)]
pub struct ArchiveExportCmd {
    #[clap(flatten)]
    backend: BackendOptions,

    /// Don't compress the archive with gzip.
    ///
    /// Useful for doing custom compression or for piping the output directly
    /// to another import command.
    #[clap(long)]
    no_gzip: bool,

    /// Path where the export should be written.
    ///
    /// If not given, data is written to stdout.
    path: Option<PathBuf>,
}

impl ArchiveExportCmd {
    pub fn run(self) {
        let backend = self.backend.build_backend_config().unwrap();
        let app_config = semantic::app::AppConfig {
            backend: Some(backend),
            // TODO: this is useless. should be moved to server config.
            token_key: "xxx".to_string(),
            // No need for deno when exporting.
            deno: None,
            tmp_dir: None,
        };

        let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
        let handle = rt.handle().clone();
        rt.block_on(self.export(app_config, handle))
            .expect("Export failed");
    }

    async fn export(
        self,
        app_config: AppConfig,
        handle: tokio::runtime::Handle,
    ) -> Result<(), anyhow::Error> {
        tracing::debug!("opening database...");
        let app = semantic::app::App::build(app_config, handle).await?;

        let compression = if self.no_gzip {
            None
        } else {
            Some(semantic::util::Compression::Gzip)
        };

        if let Some(path) = self.path {
            if path.is_dir() {
                bail!("Given path is a directory: {}", path.display());
            } else if path.is_file() {
                bail!("Given path already exists: {}", path.display());
            }
            let f = std::fs::File::create(path)?;
            let writer = std::io::BufWriter::new(f);

            app.build_export(writer, compression).await
        } else {
            let writer = std::io::stdout();
            app.build_export(writer, compression).await
        }
    }
}
