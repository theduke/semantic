use clap::Args;
use semantic_db_core::DEFAULT_COLLECTION;

use crate::{CliError, CommonArgs, open_db};

#[derive(Debug, Clone, Args)]
pub struct DeleteArgs {
    #[clap(flatten)]
    pub common: CommonArgs,
    #[arg(long, default_value = DEFAULT_COLLECTION)]
    pub collection: String,
    #[arg(value_name = "ENTITY_ID", num_args = 1..)]
    pub entity_ids: Vec<String>,
}

pub async fn run(args: DeleteArgs) -> std::result::Result<(), CliError> {
    let db = open_db(&args.common.db_uri)?;
    let mut deleted = 0usize;

    for entity_id in &args.entity_ids {
        db.delete(args.collection.as_str(), entity_id.clone())
            .await?;
        deleted = deleted.saturating_add(1);
    }

    println!("deleted {deleted} entities");
    Ok(())
}
