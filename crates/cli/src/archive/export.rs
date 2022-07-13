use std::path::PathBuf;

use anyhow::bail;

use crate::BackendOptions;

/// Delete entities.
#[derive(clap::Parser)]
pub struct ArchiveExportCmd {
    #[clap(flatten)]
    backend: BackendOptions,

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
        rt.block_on(async move {
            let app = semantic::app::App::build(app_config, handle).await?;

            if let Some(path) = self.path {
                if path.is_dir() {
                    bail!("Given path is a directory: {}", path.display());
                } else if path.is_file() {
                    bail!("Given path already exists: {}", path.display());
                }
                let f = std::fs::File::create(path)?;
                let writer = std::io::BufWriter::new(f);

                app.build_export(writer).await
            } else {
                let writer = std::io::stdout();
                app.build_export(writer).await
            }
        })
        .expect("Export failed");
    }
}
