use clap::Parser;
use tracing_subscriber::EnvFilter;

fn init_tracing() {
    let filter = match std::env::var(EnvFilter::DEFAULT_ENV) {
        Ok(value) => EnvFilter::try_new(value).unwrap_or_else(|error| {
            eprintln!(
                "semantic: invalid {} filter: {error}; using the default filter",
                EnvFilter::DEFAULT_ENV
            );
            EnvFilter::default()
        }),
        Err(std::env::VarError::NotPresent) => EnvFilter::default(),
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!(
                "semantic: {} is not valid Unicode; using the default filter",
                EnvFilter::DEFAULT_ENV
            );
            EnvFilter::default()
        }
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    init_tracing();
    let args = semantic_cli::cmd::Args::parse();
    match semantic_cli::run(args).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("semantic: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
