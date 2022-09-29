/// Delete entities.
#[derive(clap::Parser)]
pub struct SqlCmd {
    #[clap(flatten)]
    client: crate::ClientOptions,

    #[arg(value_enum, short, long)]
    format: Format,

    sql: String,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum Format {
    Json,
    JsonPretty,
}

impl SqlCmd {
    pub fn run(self) {
        let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
        rt.block_on(self.sql_query());
    }

    async fn sql_query(self) {
        let client = self.client.build_client();

        let query = match factdb::Select::parse_sql(&self.sql) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("Invalid query: {}", err);
                std::process::exit(1);
            }
        };

        let items = client.select(query).await.unwrap();

        match self.format {
            Format::Json => {
                let mut lock = std::io::stdout().lock();
                serde_json::to_writer(&mut lock, &items).unwrap();
            }
            Format::JsonPretty => {
                let mut lock = std::io::stdout().lock();
                serde_json::to_writer_pretty(&mut lock, &items).unwrap();
            }
        }
    }
}
