use semantic::app::App;

use crate::cmd::{AppOptions, AsyncCliCommand};

#[derive(clap::Parser)]
pub struct CmdLogHistoryCompact {
    #[clap(flatten)]
    options: AppOptions,

    #[arg(long)]
    select_window_size: Option<u64>,

    #[arg(long)]
    batch_size: Option<u64>,
}

impl AsyncCliCommand for CmdLogHistoryCompact {
    async fn run(self) -> Result<(), anyhow::Error> {
        let config = self.options.build()?;
        let handle = tokio::runtime::Handle::current();

        let app = App::build(config, handle.clone()).await?;

        let db = app.require_db()?;
        let plugins = app.require_plugins()?;

        let log = db
            .client()
            .as_any()
            .downcast_ref::<factor_engine::Engine>()
            .unwrap()
            .backend()
            .as_any()
            .unwrap()
            .downcast_ref::<factor_engine::backend::log::LogDb>()
            .unwrap()
            .with_store(|s| {
                s.as_any()
                    .downcast_ref::<semantic::db::logdb::LogDbStore>()
                    .unwrap()
                    .clone()
            })
            .await;

        let batch_size = self.batch_size.unwrap_or(10000);
        let select_window_size = self.select_window_size.unwrap_or(10000);

        semantic::db::compact_db_history(&db, log.log(), &plugins, select_window_size, batch_size)
            .await?;

        Ok(())
    }
}
