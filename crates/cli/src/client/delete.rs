/// Delete entities.
#[derive(clap::Parser)]
pub struct DeleteCmd {
    #[clap(flatten)]
    client: crate::ClientOptions,

    /// Run in non-interactive mode without any prompts.
    #[clap(short = 'y', long)]
    auto_confirm: bool,

    ids: Vec<factdb::Id>,
}

impl DeleteCmd {
    pub fn run(self) {
        let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
        rt.block_on(self.delete());
    }

    async fn delete(self) {
        let client = self.client.build_client();

        let mut batch = factdb::Batch::new();
        for id in &self.ids {
            batch = batch.and_delete(factdb::query::mutate::Delete { id: id.clone() });
        }

        client.batch(batch).await.unwrap();

        if !self.auto_confirm {}

        eprintln!("{} items deleted!", self.ids.len());
    }
}
