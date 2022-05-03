use anyhow::{anyhow, bail, Context};
use bytesize::ByteSize;
use factordb::{
    prelude::{AttrId, AttrIdent, AttributeDescriptor, EntityContainer, Id},
    AnyError,
};
use semantic_core::{
    api::{self, ApiClientExecutor, DbConfig},
    base::{AttrTagName, SemanticBasePlugin, Tag},
    plugin::PluginDescriptor,
};
use std::{
    io::Write,
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
    sync::Arc,
};

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

    let args = <CliArgs as clap::Parser>::parse();

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
        CliCommand::Upload(cmd) => {
            run_upload(cmd);
        }
    }
}

fn run_upload(cmd: CommandUpload) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    match rt.block_on(upload(cmd)) {
        Ok(_) => {}
        Err(err) => {
            eprintln!("Upload failed!\n{err}");
            std::process::exit(1);
        }
    }
}

async fn resolve_tags<E, L, V>(
    client: &api::ApiClient<E>,
    tag_names: L,
) -> Result<Vec<Tag>, anyhow::Error>
where
    E: ApiClientExecutor,
    L: AsRef<[V]>,
    V: AsRef<str>,
{
    use factordb::prelude as db;

    let mut tags = Vec::<Tag>::new();
    for name in tag_names.as_ref() {
        let name = name.as_ref().trim();

        let filter = db::Expr::is_entity::<Tag>().and_with(
            db::Expr::eq(AttrId::expr(), name)
                .or_with(db::Expr::eq(AttrIdent::expr(), name))
                .or_with(db::Expr::eq(AttrTagName::expr(), name)),
        );
        let select = db::Select::new().with_filter(filter);
        let page = client.select(select).await?;

        if let Some(item) = page.items.first() {
            let tag = Tag::try_from_map(item.data.clone())?;

            if page.items.len() == 1 {
                tags.push(tag.clone());
            } else {
                bail!("Cound not resolve tag '{name}': found multiple matches");
            };
        } else {
            bail!("Could not resolve tag '{name}': not found");
        }
    }

    Ok(tags)
}

async fn upload(cmd: CommandUpload) -> Result<(), AnyError> {
    if cmd.paths.is_empty() {
        bail!("Must specify at least one path");
    }

    let endpoint = cmd
        .address
        .unwrap_or_else(|| format!("http://localhost:{}", api::DEFAULT_PORT));
    let client = api::ApiClient::new(semantic::ApiClient::new(&endpoint)?);

    // If tags were specified, resolve them.
    let tags = if cmd.tag.is_empty() {
        Vec::new()
    } else {
        eprintln!("Resolving tags...");
        let items = resolve_tags(&client, &cmd.tag).await?;
        items
    };

    // Resolve the target gallery, if specified.
    let collection = if let Some(identifier) = cmd.collection {
        eprintln!("Resolving collection...");
        let identifier = identifier.trim();
        let select = semantic_core::base::Collection::query_resolve_collection_name(&identifier);
        let page = client.select(select).await?;

        if let Some(item) = page.items.first() {
            if page.items.len() == 1 {
                Some(semantic_core::base::Collection::try_from_map(
                    item.data.clone(),
                )?)
            } else {
                bail!("Could not resolve collection '{identifier}': found multiple matches");
            }
        } else if !cmd.auto_confirm || (cmd.auto_confirm && cmd.collection_create) {
            eprintln!("Collection not found!");
            eprintln!("Create collection with title '{identifier}'?");
            eprint!("Confirm [y|yes]: ");
            let mut input = String::new();
            {
                std::io::stdin().read_line(&mut input)?;
            }
            eprint!("\n");

            let input = input.trim();
            if !(input == "y" || input == "yes") {
                eprintln!("Aborting...");
                return Ok(());
            }

            let collection = semantic_core::base::Collection {
                id: Id::random(),
                ident: None,
                url: None,
                title: identifier.to_string(),
                description: None,
                item_ids: Vec::new(),
                extra: Default::default(),
            };

            client.entity_create(collection.clone()).await?;
            eprintln!("Collection created!");
            Some(collection)
        } else {
            bail!("Could not resolve collection '{identifier}: not found");
        }
    } else {
        None
    };

    eprintln!("Searching for files...");
    let mut files = Vec::<FileItem>::new();
    for path in cmd.paths {
        let canonical = std::fs::canonicalize(path)?;
        find_files(&canonical, &mut files)?;
    }

    if files.is_empty() {
        bail!("No files found.");
    }

    {
        let mut lock = std::io::stderr().lock();

        write!(lock, "Found {} files:\n", files.len()).unwrap();
        for file in &files {
            write!(
                lock,
                "* {} ({})\n",
                file.path.display(),
                ByteSize(file.size)
            )
            .unwrap();
        }

        write!(lock, "\n").unwrap();
    }

    if !cmd.auto_confirm {
        let total_size: u64 = files.iter().map(|f| f.size).sum();
        let tag_info = if tags.is_empty() {
            "".to_string()
        } else {
            let names = tags
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            format!("Adding tags: {}\n", names)
        };
        let collection_info = collection
            .as_ref()
            .map(|c| format!("Adding to collection: {}\n", c.title))
            .unwrap_or_default();
        eprint!(
            "{} file(s) with total size of {}\n{collection_info}{tag_info}\nReally upload: [y|yes]: ",
            files.len(),
            ByteSize(total_size),
        );
        let mut buf = String::new();
        std::io::stdin().read_line(&mut buf)?;
        eprint!("\n");

        let clean = buf.trim();
        if !(clean == "y" || clean == "yes") {
            eprintln!("Aborting...");
            return Ok(());
        }
    }

    eprintln!("Uploading files...");

    let title = if files.len() == 1 { cmd.title } else { None };

    let tag_ids: Vec<_> = tags.iter().map(|t| t.id).collect();

    for file in files {
        eprintln!("Uplading {}...", file.path.display());

        let filename = file
            .path
            .file_name()
            .map(|n| String::from_utf8_lossy(n.as_bytes()))
            .map(|s| s.to_string());

        let meta = api::FileUploadMetadata {
            filename,
            title: title.clone(),
            collection_id: collection.as_ref().map(|c| c.id),
            tag_ids: tag_ids.clone(),
        };

        let file = std::fs::File::open(&file.path)
            .with_context(|| format!("Could  not open file: {}", file.path.display()))?;
        let f = client.upload_file_std(meta, file).await?;

        // TODO: nicer formatting...
        eprintln!("{}", serde_json::to_string_pretty(&f).unwrap());
    }

    eprintln!("\nUpload complete!");

    Ok(())
}

struct FileItem {
    path: PathBuf,
    size: u64,
}

fn find_files(path: &Path, buffer: &mut Vec<FileItem>) -> Result<(), AnyError> {
    let meta = path
        .metadata()
        .with_context(|| format!("Could not read {}", path.display()))?;
    if meta.is_file() {
        buffer.push(FileItem {
            path: path.to_owned(),
            size: meta.len(),
        });
    } else if meta.is_dir() {
        let reader = std::fs::read_dir(path)
            .with_context(|| format!("Could not enumerate diretory {}", path.display()))?;

        for res in reader {
            let entry =
                res.with_context(|| format!("Could not enumerate diretory {}", path.display()))?;
            find_files(&entry.path(), buffer)?;
        }
    }

    Ok(())
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
#[derive(clap::Parser)]
struct CliArgs {
    #[clap(subcommand)]
    command: CliCommand,
}

#[derive(clap::Subcommand)]
enum CliCommand {
    Server(CommandServer),
    ImportFiles(CommandImportFiles),
    #[cfg(feature = "webkit")]
    Webkit(CommandWebkit),
    GenerateTypescript(GenerateTypescript),
    CompactDb(CommandCompactDb),
    /// Generate an archive that contains all data and blobs.
    Export {
        #[clap(flatten)]
        backend: BackendOptions,

        /// Path where the export should be written.
        /// If not given, data is written to stdout.
        path: Option<String>,
    },
    Upload(CommandUpload),
}

#[derive(clap::Parser)]
struct GenerateTypescript {}

#[derive(clap::Parser, Clone)]
struct BackendOptions {
    #[clap(long, env = "SEMANTIC_DATA_PATH")]
    data_path: Option<String>,
    #[clap(long, short, env = "SEMANTIC_KEY")]
    key: Option<String>,
    #[clap(long, env = "SEMANTIC_KEY_ITERATIONS")]
    key_iterations: Option<u32>,
    #[clap(long, env = "SEMANTIC_SALT")]
    salt: Option<String>,
    /// Binary offset in the storage file.
    /// Either a number of bytes, or a parsable pretty byte number like "300mb".
    #[clap(long)]
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

#[derive(clap::Parser)]
/// Import files into a semantic database.
struct CommandImportFiles {
    #[clap(flatten)]
    backend: BackendOptions,

    paths: Vec<std::path::PathBuf>,
}

/// Run the semantic server backend.
#[derive(clap::Parser)]
struct CommandServer {
    #[clap(flatten)]
    backend: BackendOptions,

    /// Do not initialize a backend.
    /// The backend will have to be configured via the UI.
    #[clap(long)]
    no_backend: bool,

    /// The server interface to listen on.
    /// eg: `0.0.0.0:3000`
    #[clap(long, env = "SEMANTIC_ADDRESS")]
    address: Option<String>,

    /// The key used for JWT token encryption.
    #[clap(long, env = "SEMANTIC_TOKEN_KEY")]
    token_key: Option<String>,

    #[clap(long, env = "SEMANTIC_TMP_DIR")]
    tmp_dir: Option<String>,
}

#[derive(clap::Parser)]
struct CommandCompactDb {
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

#[derive(clap::Parser)]
struct CommandUpload {
    /// Run in non-interactive mode without any prompts.
    #[clap(short = 'y', long)]
    auto_confirm: bool,

    /// The URL of the semantic server.
    #[clap(long)]
    address: Option<String>,

    /// Existing collection to upload files to.
    /// Can be the gallery title, ident or id.
    #[clap(long, short = 'c')]
    collection: Option<String>,

    /// If the speicified collection can not be found, create it.
    #[clap(long)]
    collection_create: bool,

    /// The title to give the uploaded file.
    ///
    /// NOTE: only works if a SINGLE file is uploaded.
    /// Will produce an error if multiple files are selected.
    #[clap(long)]
    title: Option<String>,

    /// Tag(s) to add to the uploaded file(s).
    ///
    /// Each specified tag can be either the tag name, ident or ID.
    #[clap(short = 't', long)]
    tag: Vec<String>,

    /// The file system paths.
    /// Each path be either a file or a directory.
    paths: Vec<PathBuf>,
}

/// Run a semantic UI inside webkit.
#[cfg(feature = "webkit")]
#[derive(clap::Parser)]
#[clap(about = "Semantic CLI")]
struct CommandWebkit {}
