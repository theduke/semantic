mod create;
mod log_recover_data;

#[derive(clap::Subcommand)]
pub(crate) enum DbCmd {
    LogRecoverData(log_recover_data::LogRecoverDataCmd),
    Create(create::CreateCmd),
}

impl DbCmd {
    pub fn run(self) {
        match self {
            Self::LogRecoverData(cmd) => {
                cmd.run();
            }
            Self::Create(cmd) => {
                cmd.run();
            }
        }
    }
}
