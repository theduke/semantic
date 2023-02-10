use crate::cmd::AsyncCliCommand;

use super::ClientOptions;

/// Delete entities.
#[derive(clap::Parser)]
pub struct CmdSql {
    #[clap(flatten)]
    client: ClientOptions,

    #[arg(value_enum, short, long)]
    format: Format,

    sql: String,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum Format {
    Json,
    JsonPretty,
}

impl AsyncCliCommand for CmdSql {
    async fn run(self) -> Result<(), anyhow::Error> {
        let client = self.client.build_client();

        let query = match factdb::Select::parse_sql(&self.sql) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Invalid query: {}", err);
                std::process::exit(1);
            }
        };

        let items = client.select(query).await?;

        match self.format {
            Format::Json => {
                let mut lock = std::io::stdout().lock();
                serde_json::to_writer(&mut lock, &items)?;
            }
            Format::JsonPretty => {
                let mut lock = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut lock, &items)?;
            }
        }

        Ok(())
    }
}
