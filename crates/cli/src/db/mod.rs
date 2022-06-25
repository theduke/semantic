mod log_recover_data;

#[derive(clap::Subcommand)]
pub(crate) enum DbCmd {
    LogRecoverData(log_recover_data::LogRecoverDataCmd),
}

impl DbCmd {
    pub fn run(self) {
        match self {
            Self::LogRecoverData(cmd) => {
                cmd.run();
            }
        }
    }
}
