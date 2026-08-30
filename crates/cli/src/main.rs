use clap::Parser;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = semantic_cli::cmd::Args::parse();
    match semantic_cli::run(args).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("semantic: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
