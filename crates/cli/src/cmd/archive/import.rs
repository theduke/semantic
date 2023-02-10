use std::path::PathBuf;

use semantic::util::Compression;

use crate::cmd::{AsyncCliCommand, BackendOptions};

/// Delete entities.
#[derive(clap::Parser)]
pub struct CmdArchiveImport {
    #[clap(flatten)]
    backend: BackendOptions,

    #[arg(long)]
    no_gzip: bool,

    /// Path of the export archive.
    ///
    /// If not given, data is read from stdin.
    path: Option<PathBuf>,
}

impl AsyncCliCommand for CmdArchiveImport {
    async fn run(self) -> Result<(), anyhow::Error> {
        let backend = self.backend.build_backend_config()?;
        let app_config = semantic::app::AppConfig {
            backend: Some(backend),
            // TODO: this is useless. should be moved to server config.
            token_key: "xxx".to_string(),
            // No need for deno when exporting.
            deno: None,
            tmp_dir: None,
        };

        let handle = tokio::runtime::Handle::current();
        let app = semantic::app::App::build(app_config, handle).await?;

        if let Some(path) = self.path {
            let compression = if self.no_gzip {
                None
            } else if path
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x == "gz")
                .unwrap_or_default()
            {
                Some(Compression::Gzip)
            } else {
                None
            };
            let f = std::fs::File::open(path)?;
            semantic::util::archive::import_archive(&app, f, compression).await?;
        } else {
            let compression = None;

            semantic::util::archive::import_archive(&app, std::io::stdin().lock(), compression)
                .await?;
        }

        Ok(())
    }
}
