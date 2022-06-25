use std::io::{BufWriter, Write};

use semantic::app::App;

/// Recover entities from a possibly corrupted database that uses a log backend.
#[derive(clap::Parser)]
pub struct LogRecoverDataCmd {
    #[clap(flatten)]
    backend: crate::BackendOptions,
}

impl LogRecoverDataCmd {
    pub fn run(self) {
        let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
        rt.block_on(self.log_recover_data());
    }

    async fn log_recover_data(self) {
        let conf = self.backend.build_backend_config().unwrap();
        eprintln!("Recovering...");
        let items = App::recover_database_data(conf).await.unwrap();
        eprintln!("Recovered {} items!", items.len());

        let mut stdout = BufWriter::new(std::io::stdout().lock());

        for item in items {
            serde_json::to_writer(&mut stdout, &item).unwrap();
            stdout.write(b"\n").unwrap();
        }
    }
}
