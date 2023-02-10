use std::path::PathBuf;

use anyhow::bail;

use crate::cmd::{AsyncCliCommand, BackendOptions};

/// Delete entities.
#[derive(clap::Parser)]
pub struct CmdArchiveExport {
    #[clap(flatten)]
    backend: BackendOptions,

    /// Don't compress the archive with gzip.
    ///
    /// Useful for doing custom compression or for piping the output directly
    /// to another import command.
    #[arg(long)]
    no_gzip: bool,

    /// Do not include any blobs (files) in the archive, only semantic entities.
    #[arg(long)]
    skip_blobs: bool,

    /// Path where the export should be written.
    ///
    /// If not given, data is written to stdout.
    path: Option<PathBuf>,
}

impl AsyncCliCommand for CmdArchiveExport {
    async fn run(self) -> Result<(), anyhow::Error> {
        let backend = self.backend.build_backend_config().unwrap();
        let app_config = semantic::app::AppConfig {
            backend: Some(backend),
            // TODO: this is useless. should be moved to server config.
            token_key: "xxx".to_string(),
            // No need for deno when exporting.
            deno: None,
            tmp_dir: None,
        };

        tracing::debug!("opening database...");
        let handle = tokio::runtime::Handle::current();
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

            app.build_export(writer, compression, self.skip_blobs).await
        } else {
            let writer = std::io::stdout();
            app.build_export(writer, compression, self.skip_blobs).await
        }
    }
}
