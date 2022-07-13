mod export;
#[derive(clap::Subcommand)]
pub(crate) enum ArchiveCmd {
    Export(export::ArchiveExportCmd),
}

impl ArchiveCmd {
    pub fn run(self) {
        match self {
            ArchiveCmd::Export(cmd) => {
                cmd.run();
            }
        }
    }
}
