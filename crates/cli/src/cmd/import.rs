use anyhow::bail;
use futures::{StreamExt, TryStreamExt};
use semantic_core::api;

use super::AsyncCliCommand;

#[derive(clap::Parser)]
pub struct CmdImport {
    /// The URL of the semantic server.
    #[arg(long)]
    address: Option<String>,

    /// Run in non-interactive mode without any prompts.
    #[arg(short = 'y', long)]
    auto_confirm: bool,

    /// How many uploads should run in parallel.
    #[arg(long)]
    concurrency: Option<usize>,

    /// Existing collection to upload files to.
    /// Can be the gallery title, ident or id.
    #[arg(long, short = 'c')]
    collection: Option<String>,

    /// The name, ident or ID of the parent entity.
    /// All uploaded files will be saved as children of the specified parent.
    #[arg(long)]
    parent: Option<String>,

    /// If the speicified collection can not be found, create it.
    #[arg(long)]
    collection_create: bool,

    /// The title to give the uploaded file.
    ///
    /// NOTE: only works if a SINGLE file is uploaded.
    /// Will produce an error if multiple files are selected.
    #[arg(long)]
    title: Option<String>,

    /// Tag(s) to add to the uploaded file(s).
    ///
    /// Each specified tag can be either the tag name, ident or ID.
    #[arg(short = 't', long)]
    tag: Vec<String>,

    // /// How many uploads should run in parallel.
    // #[arg(long)]
    // concurrency: Option<usize>,
    /// The URLs to import.
    urls: Vec<url::Url>,
}

impl AsyncCliCommand for CmdImport {
    async fn run(self) -> Result<(), anyhow::Error> {
        let cmd = self;

        if cmd.urls.is_empty() {
            bail!("Must specify at least one url");
        }

        let endpoint = cmd
            .address
            .unwrap_or_else(|| format!("http://localhost:{}", api::DEFAULT_PORT));
        let client = api::ApiClient::new(semantic::ApiClient::new(&endpoint)?);

        let urls = cmd.urls;
        let count = urls.len();

        eprintln!("Importing {count} urls...");

        let concurrency = cmd.concurrency.unwrap_or(1);

        futures::stream::iter(urls.as_slice())
            .enumerate()
            .map(Ok)
            .try_for_each_concurrent(concurrency, |(index, url)| {
                eprintln!("importing url {}/{}: {}...", index + 1, count, url,);

                let f = client.import(semantic_core::plugin::ImportJob {
                    url: url.clone(),
                    import_media: true,
                });

                async {
                    let out = f.await?;
                    eprintln!("{}", serde_json::to_string_pretty(&out).unwrap());
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await?;

        eprintln!("\nUpload complete!");

        Ok(())
    }
}
