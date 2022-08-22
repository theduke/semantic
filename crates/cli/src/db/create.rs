use std::{io::Read, path::PathBuf};

use anyhow::Context;
use factdb::{AttrMapExt, DataMap, Id};

/// Create entities from JSON.
///
/// The import source may either be stdin or a file.
///
/// The data format is either:
///
/// * A single JSON encoded entity
/// * jsonlines with each line containing a JSON entity
#[derive(clap::Parser)]
pub struct CreateCmd {
    #[clap(flatten)]
    app: crate::AppOptions,

    /// The path to a file containing entities.
    /// If not specified the data will be read from stdin.
    path: Option<PathBuf>,
}

impl CreateCmd {
    pub fn run(self) {
        let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
        rt.block_on(self.create(rt.handle()));
    }

    async fn create(self, rt: &tokio::runtime::Handle) {
        eprintln!("Reading input...");

        // TODO: don't require reading the entire input into memory.
        let input = if let Some(path) = self.path {
            std::fs::read_to_string(&path)
                .with_context(|| format!("Could not read file at {}", path.display()))
                .unwrap()
        } else {
            let mut buf = String::new();
            std::io::stdin().lock().read_to_string(&mut buf).unwrap();
            buf
        };

        let items = if let Ok(data) = serde_json::from_str::<DataMap>(&input) {
            vec![data]
        } else if let Ok(items) = serde_json::from_str::<Vec<DataMap>>(&input) {
            items
        } else {
            let res = input
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str::<DataMap>(line))
                .collect::<Result<Vec<_>, _>>();

            if let Ok(items) = res {
                items
            } else {
                eprintln!("ERROR: Invalid input: expected either a single JSON dict, a JSON array of dicts, or a jsonlines file with one dict per line");
                std::process::exit(1);
            }
        };

        eprintln!("Found {} items!", items.len());

        let options = self.app.build().unwrap();
        eprintln!("Opening app...");
        let app = semantic::app::App::build(options, rt.clone())
            .await
            .unwrap();
        let db = app.require_db().unwrap();

        eprintln!("Creating...");
        let len = items.len();
        let mut batch = factdb::Batch::new();
        for item in items {
            let id = item.get_id().unwrap_or_else(|| Id::random());
            batch = batch.and_create(factdb::query::mutate::Create { id, data: item });
        }

        db.batch(batch).await.unwrap();

        eprintln!("{} items created!", len);
    }
}
