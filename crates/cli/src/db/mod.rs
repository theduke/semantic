mod create;
mod log_history_compact;
mod log_recover_data;
mod logfs_compact;

#[derive(clap::Subcommand)]
pub(crate) enum DbCmd {
    LogRecoverData(log_recover_data::LogRecoverDataCmd),
    LogFsCompact(logfs_compact::LogCompactCmd),
    LogHistoryCompact(log_history_compact::LogHistoryCompactCmd),
    Create(create::CreateCmd),
}

impl DbCmd {
    pub fn run(self) {
        match self {
            Self::LogRecoverData(cmd) => {
                cmd.run();
            }
            Self::LogFsCompact(cmd) => {
                cmd.run().unwrap();
            }
            Self::Create(cmd) => {
                cmd.run();
            }
            Self::LogHistoryCompact(cmd) => {
                cmd.run().unwrap();
            }
        }
    }
}
