pub mod cmd;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Fuse(#[from] semantic_fuse::FuseError),
}

pub async fn run(args: cmd::Args) -> std::result::Result<(), CliError> {
    match args.command {
        cmd::SubCmd::Fuse(args) => cmd::fuse::run(args).await,
    }
}
