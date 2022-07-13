mod export;
mod import;

#[derive(clap::Subcommand)]
pub(crate) enum ArchiveCmd {
    Export(export::ArchiveExportCmd),
    Import(import::ArchiveImportCmd),
}

impl ArchiveCmd {
    pub fn run(self) {
        match self {
            ArchiveCmd::Export(cmd) => {
                cmd.run();
            }
            ArchiveCmd::Import(cmd) => {
                cmd.run();
            }
        }
    }
}
