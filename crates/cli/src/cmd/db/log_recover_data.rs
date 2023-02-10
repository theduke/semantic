use std::io::{BufWriter, Write};

use semantic::app::App;

use crate::cmd::{AsyncCliCommand, BackendOptions};

/// Recover entities from a possibly corrupted database that uses a log backend.
#[derive(clap::Parser)]
pub struct CmdLogRecoverData {
    #[clap(flatten)]
    backend: BackendOptions,
}

impl AsyncCliCommand for CmdLogRecoverData {
    async fn run(self) -> Result<(), anyhow::Error> {
        let conf = self.backend.build_backend_config()?;
        eprintln!("Recovering...");
        let items = App::recover_database_data(conf).await?;
        eprintln!("Recovered {} items!", items.len());

        let mut stdout = BufWriter::new(std::io::stdout().lock());

        for item in items {
            serde_json::to_writer(&mut stdout, &item)?;
            stdout.write(b"\n")?;
        }

        Ok(())
    }
}
