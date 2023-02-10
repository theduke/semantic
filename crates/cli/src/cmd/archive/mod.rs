use super::CliCommand;

mod export;
mod import;

#[derive(clap::Subcommand)]
pub(crate) enum CmdArchive {
    Export(export::CmdArchiveExport),
    Import(import::CmdArchiveImport),
}

impl CliCommand for CmdArchive {
    fn run(self) -> Result<(), anyhow::Error> {
        match self {
            CmdArchive::Export(cmd) => cmd.run(),
            CmdArchive::Import(cmd) => cmd.run(),
        }
    }
}
