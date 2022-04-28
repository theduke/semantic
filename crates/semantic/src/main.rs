use anyhow::{anyhow, bail, Context};
use factordb::AnyError;
use semantic_core::{
    api::{self, DbConfig},
    base::SemanticBasePlugin,
    plugin::PluginDescriptor,
};
use std::{io::Write, path::PathBuf, sync::Arc};
use structopt::StructOpt;

use semantic::{
    app::{self, App},
    server,
};

fn main() {
    if std::env::var("RUST_LOG").is_err() {
        #[cfg(not(debug_assertions))]
        let default = "semantic=info";
        #[cfg(debug_assertions)]
        let default = "semantic=trace,logfs=trace,factordb=debug,semantic_core=trace";

        std::env::set_var("RUST_LOG", default);
    }

    // Initialize logger.
    // TODO: tracing-tree disabled until it supports tracing_subscriber 0.3
    // let subscriber =
    //     tracing_subscriber::Registry::default().with(tracing_tree::HierarchicalLayer::new(2));
    // tracing::subscriber::set_global_default(subscriber).unwrap();
    tracing_subscriber::fmt::init();

    let args = CliArgs::from_args();

    match args.command {
        CliCommand::Server(subargs) => {
            let data_dir = app::App::default_data_dir().unwrap();

            let backend_config = if subargs.no_backend {
                None
            } else {
                Some(subargs.backend.build_backend_config().unwrap())
            };

            let tmp_dir = subargs.tmp_dir.map(PathBuf::from);

            let app_config = app::AppConfig {
                backend: backend_config,
                token_key: subargs.token_key.unwrap_or_else(app::App::random_token_key),
                deno: Some(app::DenoConfig {
                    data_dir: data_dir.join("deno"),
                    plugin_dir: None,
                }),
                tmp_dir,
            };
            let config = server::ServerConfig {
                // Enable authentication when no backend is provided.
                require_auth: app_config.backend.is_none(),
                app: app_config,
                address: subargs.address.unwrap_or(format!("127.0.0.1:3000")),
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            rt.block_on(server::run_server(config, rt.handle().clone()))
                .expect("Server failed");
        }
        CliCommand::GenerateTypescript(_) => {
            let builtin = factordb::schema::builtin::builtin_db_schema();
            let base = SemanticBasePlugin::new().schema().db.unwrap();

            let schema = builtin.merge(base);

            let ts = factor_tools::typescript::schema_to_typescript(&schema, None).unwrap();

            write!(std::io::stdout(), "{}", ts).unwrap();
        }
        #[cfg(feature = "webkit")]
        CliCommand::Gtk => {
            let config = app::AppConfig {
                backend: None,
                token_key: uuid::Uuid::new_v4().to_string(),
                server: None,
            };
            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let app = rt
                .block_on(app::App::build(config, rt.handle().clone()))
                .expect("Could not build app");
            app.run_webview_gtk().expect("Could not run GTK app");
        }
        CliCommand::Export { backend, path } => {
            let backend = backend.build_backend_config().unwrap();
            let app_config = semantic::app::AppConfig {
                backend: Some(backend),
                // TODO: this is useless. should be moved to server config.
                token_key: "xxx".to_string(),
                // No need for deno when exporting.
                deno: None,
                tmp_dir: None,
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let handle = rt.handle().clone();
            rt.block_on(async move {
                let app = semantic::app::App::build(app_config, handle).await?;

                if let Some(path) = path {
                    let pathb = std::path::PathBuf::from(&path);
                    if pathb.is_dir() {
                        bail!("Given path is a directory: {path}");
                    } else if pathb.is_file() {
                        bail!("Given path already exists: {path}");
                    }
                    let f = std::fs::File::create(path)?;
                    let writer = std::io::BufWriter::new(f);

                    app.build_export(writer).await
                } else {
                    let writer = std::io::stdout();
                    app.build_export(writer).await
                }
            })
            .expect("Export failed");
        }
        CliCommand::ImportFiles(args) => {
            let data_dir = app::App::default_data_dir().unwrap();

            let backend_config = args.backend.build_backend_config().unwrap();

            let app_config = app::AppConfig {
                backend: Some(backend_config),
                token_key: "".to_string(),
                deno: Some(app::DenoConfig {
                    data_dir: data_dir.join("deno"),
                    plugin_dir: None,
                }),
                tmp_dir: None,
            };

            let meta = api::FileImportMetadata {
                collection_id: None,
                tags: Vec::new(),
            };

            let rt = tokio::runtime::Runtime::new().expect("Could not start runtime");
            let handle = rt.handle().clone();
            rt.block_on(async move {
                let app = semantic::app::App::build(app_config, handle).await?;

                app.import_files(
                    args.paths,
                    meta,
                    Arc::new(|path, _file| {
                        tracing::info!(?path, "Imported file");
                    }),
                )
                .await?;

                Result::<(), AnyError>::Ok(())
            })
            .expect("Export failed");

            tracing::info!("All paths imported");
        }
        CliCommand::CompactDb(cmd) => {
            compact(cmd).expect("Compaction failed");
        }
    }
}

fn compact(cmd: CommandCompactDb) -> Result<(), anyhow::Error> {
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

/// Semantic CLI
#[derive(StructOpt)]
struct CliArgs {
    #[structopt(subcommand)]
    command: CliCommand,
}

#[derive(StructOpt)]
enum CliCommand {
    Server(CommandServer),
    ImportFiles(CommandImportFiles),
    #[cfg(feature = "webkit")]
    Webkit(CommandWebkit),
    GenerateTypescript(GenerateTypescript),
    CompactDb(CommandCompactDb),
    /// Generate an archive that contains all data and blobs.
    Export {
        #[structopt(flatten)]
        backend: BackendOptions,

        /// Path where the export should be written.
        /// If not given, data is written to stdout.
        path: Option<String>,
    },
}

#[derive(StructOpt)]
struct GenerateTypescript {}

#[derive(StructOpt, Clone)]
struct BackendOptions {
    #[structopt(long, env = "SEMANTIC_DATA_PATH")]
    data_path: Option<String>,
    #[structopt(long, short, env = "SEMANTIC_KEY")]
    key: Option<String>,
    #[structopt(long, env = "SEMANTIC_KEY_ITERATIONS")]
    key_iterations: Option<u32>,
    #[structopt(long, env = "SEMANTIC_SALT")]
    salt: Option<String>,
    /// Binary offset in the storage file.
    /// Either a number of bytes, or a parsable pretty byte number like "300mb".
    #[structopt(long)]
    offset: Option<String>,
}

impl BackendOptions {
    fn build_backend_config(self) -> Result<api::BackendConfig, AnyError> {
        let offset = if let Some(off) = self.offset {
            let size = off
                .parse::<bytesize::ByteSize>()
                .map_err(|err| anyhow!("Invalid offset: {err}"))?;
            Some(size.0)
        } else {
            None
        };

        let db = DbConfig::Crypto(api::BackendCryptoConfig {
            offset,
            data_path: self.data_path,
            key: self.key.expect("Must specify --key"),
            raw: false,
            key_iterations: self.key_iterations,
            salt: self.salt,
        });

        let c = api::BackendConfig {
            db,
            // TODO: make configurable.
            idle_timeout: None,
        };

        Ok(c)
    }
}

pub struct DenoOptions {}

#[derive(StructOpt)]
/// Import files into a semantic database.
struct CommandImportFiles {
    #[structopt(flatten)]
    backend: BackendOptions,

    paths: Vec<std::path::PathBuf>,
}

/// Run the semantic server backend.
#[derive(StructOpt)]
struct CommandServer {
    #[structopt(flatten)]
    backend: BackendOptions,

    /// Do not initialize a backend.
    /// The backend will have to be configured via the UI.
    #[structopt(long)]
    no_backend: bool,

    /// The server interface to listen on.
    /// eg: `0.0.0.0:3000`
    #[structopt(long, env = "SEMANTIC_ADDRESS")]
    address: Option<String>,

    /// The key used for JWT token encryption.
    #[structopt(long, env = "SEMANTIC_TOKEN_KEY")]
    token_key: Option<String>,

    #[structopt(long, env = "SEMANTIC_TMP_DIR")]
    tmp_dir: Option<String>,
}

#[derive(StructOpt)]
struct CommandCompactDb {
    #[structopt(flatten)]
    backend: BackendOptions,
    /// The new password to use.
    /// If not set, the old one will be reused.
    #[structopt(long)]
    new_password: Option<String>,
    #[structopt(long)]
    force: bool,
    /// The path for the new, compacted database.
    new_path: String,
}

/// Run a semantic UI inside webkit.
#[cfg(feature = "webkit")]
#[derive(StructOpt)]
#[structopt(about = "Semantic CLI")]
struct CommandWebkit {}
