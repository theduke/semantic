use crate::cmd::AsyncCliCommand;

use super::ClientOptions;

/// Delete entities.
#[derive(clap::Parser)]
pub struct CmdDelete {
    #[clap(flatten)]
    client: ClientOptions,

    /// Run in non-interactive mode without any prompts.
    #[arg(short = 'y', long)]
    auto_confirm: bool,

    ids: Vec<factdb::Id>,
}

impl AsyncCliCommand for CmdDelete {
    async fn run(self) -> Result<(), anyhow::Error> {
        let client = self.client.build_client();

        let mut batch = factdb::Batch::new();
        for id in &self.ids {
            batch = batch.and_delete(factdb::query::mutate::Delete { id: id.clone() });
        }

        client.batch(batch).await?;

        if !self.auto_confirm {}

        eprintln!("{} items deleted!", self.ids.len());

        Ok(())
    }
}
