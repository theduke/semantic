use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};

use factordb::AnyError;
use semantic::app::App;
use semantic_core::api::DbConfig;

use crate::BackendOptions;

#[derive(clap::Parser)]
pub struct LogCompactCmd {
    #[clap(flatten)]
    backend: BackendOptions,
    /// The new password to use.
    /// If not set, the old one will be reused.
    #[clap(long)]
    new_password: Option<String>,
    #[clap(long)]
    force: bool,
    /// The path for the new, compacted database.
    new_path: String,
}

impl LogCompactCmd {
    pub fn run(self) -> Result<(), AnyError> {
        let cmd = self;

        // TODO: this should also "compact" the event log of the factordb, if allowed by config.

        let backend_config = cmd
            .backend
            .clone()
            .build_backend_config()
            .context("Could not build backend config")?;

        let crypto = match backend_config.db {
            DbConfig::Crypto(c) => c,
        };

        tracing::debug!("opening old database...");
        let old_db = App::build_logfs(&crypto).context("Could not open old database...")?;
        tracing::info!("old database opened");

        let mut new_config = crypto.clone();
        if let Some(new_pw) = cmd.new_password {
            new_config.key = new_pw;
        }

        if PathBuf::from(&cmd.new_path).exists() && !cmd.force {
            bail!(
                "New database location {} already exists! Add --force to overwrite",
                cmd.new_path
            );
        }

        new_config.data_path = Some(cmd.new_path.clone());

        tracing::info!("Creating new database at {}", cmd.new_path);
        let new_db = App::build_logfs(&new_config).context("Could not open new database")?;

        let keys = old_db.paths_offset(0, usize::MAX)?;

        use sha2::Digest;
        let mut old_hash = sha2::Sha512::new();
        let mut old_size = 0;

        tracing::info!("Copying {} blobs", keys.len());

        for (index, key) in keys.iter().enumerate() {
            tracing::debug!(path=%key, "copying blob {}/{}", index+1, keys.len());
            let old_data = old_db
                .get(&key)?
                .ok_or_else(|| anyhow!("Could not read key"))?;
            old_hash.update(&old_data);
            old_size += old_data.len();

            new_db.insert(key, old_data)?;
        }

        std::mem::drop(old_db);
        std::mem::drop(new_db);

        let old_hash = old_hash.finalize();

        // Sanity check.
        // Compare hash, size and keys.
        let new_db = App::build_logfs(&new_config).context("Could not open new database")?;

        let new_keys = new_db.paths_offset(0, usize::MAX)?;
        if new_keys != keys {
            bail!("Key mismatch: new database does not have the same keys as the old one");
        }
        std::mem::drop(keys);

        let mut new_hash = sha2::Sha512::new();
        let mut new_size = 0;
        for key in new_keys.into_iter() {
            let data = new_db
                .get(&key)?
                .ok_or_else(|| anyhow!("Could not get key"))?;
            new_hash.update(&data);
            new_size += data.len();
        }

        let new_hash = new_hash.finalize();

        if new_hash != old_hash {
            bail!("Copy did not suceed: hash mismatch! expected {old_hash:?}, but new hash is {new_hash:?}");
        }
        if old_size != new_size {
            bail!("Copy did not suceed: size mismatch: expected {old_size}, but new db has size of {new_size}");
        }

        tracing::info!("database compacted into {}!", cmd.new_path);

        Ok(())
    }
}
