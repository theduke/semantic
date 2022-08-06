use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use bytesize::ByteSize;
use factordb::{
    prelude::{AttrId, AttrIdent, AttrMapExt, AttributeDescriptor, EntityContainer, Id, Select},
    AnyError,
};
use futures::{StreamExt, TryStreamExt};
use semantic_core::{
    api::{self, ApiClientExecutor, FileUploadMetadata},
    base::{entity_title, expr_find_by_id_ident_or_title, AttrTagName, Tag, TypedFile},
};

#[derive(clap::Parser)]
pub struct CmdUpload {
    /// Run in non-interactive mode without any prompts.
    #[clap(short = 'y', long)]
    auto_confirm: bool,

    /// The URL of the semantic server.
    #[clap(long)]
    address: Option<String>,

    #[clap(long)]
    url: Option<url::Url>,

    /// Existing collection to upload files to.
    /// Can be the gallery title, ident or id.
    #[clap(long, short = 'c')]
    collection: Option<String>,

    /// The name, ident or ID of the parent entity.
    /// All uploaded files will be saved as children of the specified parent.
    #[clap(long)]
    parent: Option<String>,

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

    /// How many uploads should run in parallel.
    #[clap(long)]
    concurrency: Option<usize>,

    /// The file system paths.
    /// Each path be either a file or a directory.
    paths: Vec<PathBuf>,
}

struct FileItem {
    path: PathBuf,
    size: u64,
}

type Client = api::ApiClient<semantic::ApiClient>;

impl CmdUpload {
    pub fn run(self) {
        let rt = tokio::runtime::Runtime::new().unwrap();

        match rt.block_on(self.upload()) {
            Ok(_) => {}
            Err(err) => {
                eprintln!("Upload failed!\n{err}");
                std::process::exit(1);
            }
        }
    }

    async fn upload(self) -> Result<(), AnyError> {
        let cmd = self;

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
            let select =
                semantic_core::base::Collection::query_resolve_collection_name(&identifier);
            let items = client.select(select).await?;

            if let Some(item) = items.first() {
                if items.len() == 1 {
                    Some(semantic_core::base::Collection::try_from_map(item.clone())?)
                } else {
                    bail!("Could not resolve collection '{identifier}': found multiple matches");
                }
            } else if !cmd.auto_confirm || (cmd.auto_confirm && cmd.collection_create) {
                eprintln!("Collection not found!");

                if !cmd.auto_confirm {
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
                eprintln!("Collection '{identifier}' created!");
                Some(collection)
            } else {
                bail!("Could not resolve collection '{identifier}: not found");
            }
        } else {
            None
        };

        let parent_id: Option<Id> = if let Some(identifier) = cmd.parent {
            eprintln!("Resolving parent...");
            let identifier = identifier.trim();

            let expr = expr_find_by_id_ident_or_title(identifier);
            let select = Select::new().with_filter(expr).with_limit(10);
            let items = client.select(select).await?;

            if let Some(item) = items.first() {
                if items.len() == 1 {
                    let title = entity_title(item);
                    eprintln!("Setting parent: {title}");
                    Some(item.get_id().unwrap())
                } else {
                    bail!("Could not resolve parent '{identifier}': found multiple matches");
                }
            } else {
                bail!("Could not resolve parent '{identifier}': not found");
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
            let stdio = std::io::stderr();
            let mut lock = stdio.lock();

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

        let concurrency = cmd.concurrency.clone().unwrap_or(10);
        let url = cmd.url.clone();
        let title = if files.len() == 1 { cmd.title } else { None };
        let tag_ids: Vec<_> = tags.iter().map(|t| t.id).collect();

        let count = files.len();

        let meta = FileUploadMetadata {
            filename: None,
            title,
            url,
            ident: None,
            parent: parent_id,
            collection_id: collection.map(|c| c.id),
            tag_ids,
        };

        futures::stream::iter(files.as_slice())
            .enumerate()
            .map(Ok)
            .try_for_each_concurrent(concurrency, |(index, item)| {
                eprintln!(
                    "Uplading file {}/{}: {}...",
                    index + 1,
                    count,
                    item.path.display()
                );

                let fut = upload_file(&client, &item, &meta);

                async {
                    let f = fut.await?;
                    eprintln!("{}", serde_json::to_string_pretty(&f).unwrap());
                    Ok::<(), anyhow::Error>(())
                }
            })
            .await?;

        // for (index, file) in files.into_iter().enumerate() {
        //     let filename = file.path.file_name().unwrap().to_string_lossy();

        //     let meta = api::FileUploadMetadata {
        //         filename: Some(filename.to_string()),
        //         title: title.clone(),
        //         url: cmd.url.clone(),
        //         collection_id: collection.as_ref().map(|c| c.id),
        //         tag_ids: tag_ids.clone(),
        //     };

        //     let file = std::fs::File::open(&file.path)
        //         .with_context(|| format!("Could  not open file: {}", file.path.display()))?;
        //     let f = client.upload_file_std(meta, file).await?;

        //     // TODO: nicer formatting...
        //     eprintln!("{}", serde_json::to_string_pretty(&f).unwrap());
        // }

        eprintln!("\nUpload complete!");

        Ok(())
    }
}

async fn upload_file(
    client: &Client,
    item: &FileItem,
    meta: &FileUploadMetadata,
) -> Result<TypedFile, anyhow::Error> {
    let filename = item.path.file_name().unwrap().to_string_lossy();

    let meta = api::FileUploadMetadata {
        filename: Some(filename.to_string()),
        ..meta.clone()
    };

    let file = tokio::fs::File::open(&item.path)
        .await
        .with_context(|| format!("Could  not open file: {}", item.path.display()))?;

    let f = client.upload_file_tokio(meta, file).await?;

    Ok(f)
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
        let items = client.select(select).await?;

        if let Some(item) = items.first() {
            let tag = Tag::try_from_map(item.clone())?;

            if items.len() == 1 {
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
