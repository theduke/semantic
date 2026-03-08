#[tokio::main]
async fn main() -> std::process::ExitCode {
    match semantic_db_cli::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            std::process::ExitCode::FAILURE
        }
    }
}
