use std::path::PathBuf;

use semantic::util::Compression;

use crate::BackendOptions;

/// Delete entities.
#[derive(clap::Parser)]
pub struct ArchiveImportCmd {
    #[clap(flatten)]
    backend: BackendOptions,

    #[clap(long)]
    no_gzip: bool,

    /// Path of the export archive.
    ///
    /// If not given, data is read from stdin.
    path: Option<PathBuf>,
}

impl ArchiveImportCmd {
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
        rt.block_on(async move {
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

            Ok::<(), anyhow::Error>(())
        })
        .expect("Export failed");
    }
}
