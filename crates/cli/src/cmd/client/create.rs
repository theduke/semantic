use std::io::Read;

use anyhow::Context;
use factdb::{AttrMapExt, DataMap, Id, Mutate};

use crate::cmd::AsyncCliCommand;

use super::ClientOptions;

#[derive(clap::Parser, Debug)]
pub struct CmdClientCreate {
    #[clap(flatten)]
    client: ClientOptions,

    /// Read contents from a file instead.
    #[clap(short = 'f', long)]
    file: bool,

    /// Data of the entity to crate.
    ///
    /// Either a JSON object, or a path to a file containing a JSON object
    /// (if --file/-f is specified).
    data: String,
}

impl AsyncCliCommand for CmdClientCreate {
    async fn run(self) -> Result<(), anyhow::Error> {
        let contents = if self.file {
            if self.data == "-" {
                let mut buf = String::new();
                std::io::stdin().lock().read_to_string(&mut buf)?;
                buf
            } else {
                std::fs::read_to_string(&self.data)
                    .with_context(|| format!("Could not read file '{}'", self.data))?
            }
        } else {
            self.data
        };

        let map: DataMap = serde_json::from_str(&contents).context("could not parse JSON")?;

        let id = map.get_id().unwrap_or_else(Id::random);

        let client = self.client.build_client();
        client.mutate(Mutate::merge(id, map)).await?;

        let out = client.entity(id).await?;
        let pretty = serde_json::to_string_pretty(&out)?;
        println!("{}", pretty);

        Ok(())
    }
}
