mod create;
mod log_compact;
mod log_recover_data;

#[derive(clap::Subcommand)]
pub(crate) enum DbCmd {
    LogRecoverData(log_recover_data::LogRecoverDataCmd),
    LogCompact(log_compact::LogCompactCmd),
    Create(create::CreateCmd),
}

impl DbCmd {
    pub fn run(self) {
        match self {
            Self::LogRecoverData(cmd) => {
                cmd.run();
            }
            Self::LogCompact(cmd) => {
                cmd.run().unwrap();
            }
            Self::Create(cmd) => {
                cmd.run();
            }
        }
    }
}
