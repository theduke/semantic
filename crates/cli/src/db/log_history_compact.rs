use semantic::app::App;

use crate::AppOptions;

#[derive(clap::Parser)]
pub struct LogHistoryCompactCmd {
    #[clap(flatten)]
    options: AppOptions,

    #[clap(long)]
    select_window_size: Option<u64>,

    #[clap(long)]
    batch_size: Option<u64>,
}

impl LogHistoryCompactCmd {
    pub fn run(self) -> Result<(), anyhow::Error> {
        let config = self.options.build()?;

        let rt = tokio::runtime::Runtime::new()?;

        let handle = rt.handle().clone();

        rt.block_on(async move {
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

            semantic::db::compact_db_history(
                &db,
                log.log(),
                &plugins,
                select_window_size,
                batch_size,
            )
            .await
        })?;

        Ok(())
    }
}
