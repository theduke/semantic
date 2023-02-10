use super::CliCommand;

mod create;
mod log_history_compact;
mod log_recover_data;
mod logfs_compact;

#[derive(clap::Subcommand)]
pub(crate) enum CmdDb {
    LogRecoverData(log_recover_data::CmdLogRecoverData),
    LogFsCompact(logfs_compact::CmdLogCompact),
    LogHistoryCompact(log_history_compact::CmdLogHistoryCompact),
    Create(create::CmdCreate),
}

impl CliCommand for CmdDb {
    fn run(self) -> Result<(), anyhow::Error> {
        match self {
            Self::LogRecoverData(cmd) => cmd.run(),
            Self::LogFsCompact(cmd) => cmd.run(),
            Self::Create(cmd) => cmd.run(),
            Self::LogHistoryCompact(cmd) => cmd.run(),
        }
    }
}
