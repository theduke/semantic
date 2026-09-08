use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{IsTerminal as _, Read as _};
use std::path::{Path, PathBuf};

use dialoguer::{FuzzySelect, theme::ColorfulTheme};
use futures::StreamExt as _;
use semantic_base::directory_query::{
    ATTR_RELATION_RELATION, ATTR_RELATION_TO, DirectoryChildFilter, DirectoryQueryPage,
    DirectorySort, directories_query, directory_by_id_query, directory_children_query,
    directory_links_query, root_directories_named_query,
};
use semantic_data::attr::ATTR_TITLE;
use semantic_data::builtin::{ATTR_ID, ATTR_TYPE, DEFAULT_COLLECTION};
use semantic_data::bundles::directory::{
    ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, DIRECTORY_CLASS_ID,
    DIRECTORY_NODE_CLASS_ID, DIRECTORY_NODE_RELATION_ID,
};
use semantic_data::filestore::{ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILENAME, FILE_CLASS_ID};
use semantic_data::value::{Object, Value};
use semantic_rpc::file::{FileUploadContent, FileUploadRequest};
use semantic_rpc::{RpcClient, RpcClientError};
use sha2::{Digest as _, Sha256};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, OutputArgs};

const DIRECTORY_QUERY_PAGE_SIZE: usize = 1_000;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Files to upload, or one directory when --tree is set.
    ///
    /// --recursive accepts multiple paths and flattens all discovered files.
    /// --tree requires exactly one directory and preserves its hierarchy.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Recursively upload regular files without preserving directories.
    #[arg(long, short = 'r', conflicts_with = "tree")]
    pub recursive: bool,

    /// Upload one directory as a recursively merged remote directory tree.
    ///
    /// Without --target-directory, an interactive terminal prompts for a remote
    /// root from a filterable nested list. Choosing the top-level option creates
    /// or reuses the local directory name as a remote root. Non-interactive runs
    /// use that top-level behavior automatically. With --target-directory, the
    /// local root's contents are merged directly into the specified directory.
    #[arg(long, conflicts_with = "recursive")]
    pub tree: bool,

    /// Existing remote directory ID into which the local root contents are merged.
    ///
    /// Supplying this option bypasses interactive directory selection.
    #[arg(
        long,
        visible_alias = "target-dir",
        value_name = "DIRECTORY_ID",
        requires = "tree"
    )]
    pub target_directory: Option<String>,

    /// Disable remote-directory selection prompts.
    ///
    /// Without an explicit --target-directory, non-interactive mode creates or
    /// reuses a top-level remote directory named after the local root.
    #[arg(long, requires = "tree")]
    pub non_interactive: bool,

    /// Replace same-named remote files whose SHA-256 hash differs.
    ///
    /// Same-named files with an unchanged hash are always skipped. Changed files
    /// are also skipped unless this flag is supplied. Replacement reuses the
    /// existing entity ID and directory link; blob and entity updates are not atomic.
    #[arg(long, requires = "tree")]
    pub replace: bool,

    /// JSON object merged into every uploaded file entity.
    ///
    /// PATH must point to a file containing a JSON object, not an array or scalar.
    #[arg(long, value_name = "PATH")]
    pub entity: Option<PathBuf>,

    /// Explicit record ID (valid only when exactly one non-tree file is uploaded).
    #[arg(long, conflicts_with = "tree")]
    pub id: Option<String>,

    /// MIME type applied to every uploaded file.
    ///
    /// By default the server detects each file's MIME type independently.
    #[arg(long)]
    pub mime_type: Option<String>,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
    if args.tree {
        run_tree(args).await
    } else {
        run_flat(args).await
    }
}

async fn run_flat(args: Args) -> std::result::Result<(), CliError> {
    let files = collect_files(&args.paths, args.recursive)?;
    if files.is_empty() {
        return Err(CliError::InvalidInput(
            "no regular files were found to upload".to_string(),
        ));
    }
    if args.id.is_some() && files.len() != 1 {
        return Err(CliError::InvalidInput(
            "--id can only be used when exactly one file is uploaded".to_string(),
        ));
    }
    let entity = read_entity(args.entity.as_deref())?;
    let client = args.client.rpc_client();
    let mut responses = Vec::with_capacity(files.len());
    for path in &files {
        let filename = utf8_file_name(path)?;
        let response = upload_file(
            &client,
            args.client.scope.clone(),
            path,
            filename,
            args.id.clone(),
            args.mime_type.clone(),
            entity.clone(),
        )
        .await?;
        responses.push(response.into_value());
    }

    let output = if responses.len() == 1 {
        responses.pop().expect("one upload response")
    } else {
        Value::List(responses)
    };
    args.output.print(&output)
}

#[derive(Debug)]
struct LocalTree {
    root: PathBuf,
    entries: BTreeMap<PathBuf, Vec<LocalEntry>>,
}

#[derive(Debug)]
struct LocalEntry {
    name: String,
    path: PathBuf,
    kind: LocalEntryKind,
}

#[derive(Debug)]
enum LocalEntryKind {
    Directory,
    File { hash: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RemoteKind {
    Directory,
    File { hash: Option<String> },
    Other,
}

#[derive(Clone, Debug)]
struct RemoteEntry {
    id: String,
    name: String,
    kind: RemoteKind,
    object: Object,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryChoice {
    id: Option<String>,
    label: String,
}

impl fmt::Display for DirectoryChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileMergeAction {
    Upload,
    SkipUnchanged,
    SkipChanged,
    Replace,
}

#[derive(Default)]
struct TreeStats {
    directories_created: u64,
    files_uploaded: u64,
    files_replaced: u64,
    files_skipped_unchanged: u64,
    files_skipped_changed: u64,
}

impl TreeStats {
    fn into_value(self, root_id: String) -> Value {
        let mut object = Object::new();
        object.insert("root_directory_id", Value::String(root_id));
        object.insert("directories_created", Value::U64(self.directories_created));
        object.insert("files_uploaded", Value::U64(self.files_uploaded));
        object.insert("files_replaced", Value::U64(self.files_replaced));
        object.insert(
            "files_skipped_unchanged",
            Value::U64(self.files_skipped_unchanged),
        );
        object.insert(
            "files_skipped_changed",
            Value::U64(self.files_skipped_changed),
        );
        Value::Object(object)
    }
}

async fn run_tree(args: Args) -> std::result::Result<(), CliError> {
    if args.paths.len() != 1 {
        return Err(CliError::InvalidInput(
            "--tree requires exactly one local root directory".to_string(),
        ));
    }
    let tree = build_local_tree(&args.paths[0])?;
    let shared_entity = read_entity(args.entity.as_deref())?;
    let client = args.client.rpc_client();
    let scope = args.client.scope.clone();
    let mut stats = TreeStats::default();
    let root_name = utf8_file_name(&tree.root)?;
    let selected_target = if let Some(target_id) = args.target_directory.clone() {
        Some(target_id)
    } else if should_prompt_for_directory(
        args.non_interactive,
        std::io::stdin().is_terminal(),
        std::io::stderr().is_terminal(),
    ) {
        prompt_for_directory(&client, scope.as_ref(), root_name).await?
    } else {
        None
    };
    let remote_root_id = match selected_target.as_deref() {
        Some(target_id) => {
            require_directory(&client, scope.as_ref(), target_id).await?;
            target_id.to_string()
        }
        None => match find_root_directory(&client, scope.as_ref(), root_name).await? {
            Some(id) => id,
            None => {
                stats.directories_created += 1;
                create_directory(&client, scope.as_ref(), root_name, None).await?
            }
        },
    };

    let mut remote_directories = BTreeMap::from([(PathBuf::new(), remote_root_id.clone())]);
    for (relative_directory, entries) in &tree.entries {
        let remote_directory_id = remote_directories
            .get(relative_directory)
            .cloned()
            .ok_or_else(|| {
                CliError::InvalidInput(format!(
                    "missing planned remote directory for {}",
                    relative_directory.display()
                ))
            })?;
        let remote_entries = list_directory(&client, scope.as_ref(), &remote_directory_id).await?;

        for entry in entries {
            if !matches!(entry.kind, LocalEntryKind::Directory) {
                continue;
            }
            let existing = unique_named_entry(&remote_entries, &entry.name, &remote_directory_id)?;
            let child_id = match existing {
                Some(existing) if existing.kind == RemoteKind::Directory => existing.id.clone(),
                Some(existing) => {
                    return Err(name_type_conflict(
                        &remote_directory_id,
                        &entry.name,
                        "directory",
                        &existing.kind,
                    ));
                }
                None => {
                    stats.directories_created += 1;
                    create_directory(
                        &client,
                        scope.as_ref(),
                        &entry.name,
                        Some(&remote_directory_id),
                    )
                    .await?
                }
            };
            let relative = entry
                .path
                .strip_prefix(&tree.root)
                .expect("tree entry is below root")
                .to_path_buf();
            remote_directories.insert(relative, child_id);
        }

        for entry in entries {
            let LocalEntryKind::File { hash } = &entry.kind else {
                continue;
            };
            let existing = unique_named_entry(&remote_entries, &entry.name, &remote_directory_id)?;
            if let Some(existing) = existing
                && !matches!(existing.kind, RemoteKind::File { .. })
            {
                return Err(name_type_conflict(
                    &remote_directory_id,
                    &entry.name,
                    "file",
                    &existing.kind,
                ));
            }
            let action = file_merge_action(existing, hash, args.replace);
            match action {
                FileMergeAction::SkipUnchanged => stats.files_skipped_unchanged += 1,
                FileMergeAction::SkipChanged => stats.files_skipped_changed += 1,
                FileMergeAction::Upload | FileMergeAction::Replace => {
                    let id = existing
                        .map(|entry| entry.id.clone())
                        .unwrap_or_else(|| format!("file-{}", uuid::Uuid::new_v4()));
                    let entity = replacement_entity(existing, &shared_entity);
                    let response = upload_file(
                        &client,
                        scope.clone(),
                        &entry.path,
                        &entry.name,
                        Some(id),
                        args.mime_type.clone(),
                        entity,
                    )
                    .await?;
                    if action == FileMergeAction::Upload {
                        if let Err(error) =
                            link_entity(&client, scope.as_ref(), &remote_directory_id, &response.id)
                                .await
                        {
                            return Err(CliError::InvalidInput(format!(
                                "file '{}' uploaded as '{}' but linking it into directory '{}' failed: {error}",
                                entry.path.display(),
                                response.id,
                                remote_directory_id
                            )));
                        }
                        stats.files_uploaded += 1;
                    } else {
                        stats.files_replaced += 1;
                    }
                }
            }
        }
    }

    args.output.print(&stats.into_value(remote_root_id))
}

fn should_prompt_for_directory(
    non_interactive: bool,
    stdin_is_terminal: bool,
    stderr_is_terminal: bool,
) -> bool {
    !non_interactive && stdin_is_terminal && stderr_is_terminal
}

async fn prompt_for_directory(
    client: &RpcClient,
    scope: Option<&String>,
    local_root_name: &str,
) -> std::result::Result<Option<String>, CliError> {
    let mut choices = vec![DirectoryChoice {
        id: None,
        label: format!("[top level] create or reuse '{local_root_name}'"),
    }];
    choices.extend(load_directory_choices(client, scope).await?);

    let selected = FuzzySelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select the remote root directory")
        .items(&choices)
        .default(0)
        .interact_opt()
        .map_err(|error| {
            CliError::InvalidInput(format!("failed to select a remote directory: {error}"))
        })?
        .ok_or_else(|| {
            CliError::InvalidInput("remote directory selection cancelled".to_string())
        })?;
    Ok(choices[selected].id.clone())
}

async fn load_directory_choices(
    client: &RpcClient,
    scope: Option<&String>,
) -> std::result::Result<Vec<DirectoryChoice>, CliError> {
    let directories = query_all_pages(client, scope, |offset| {
        directories_query(DirectoryQueryPage::new(DIRECTORY_QUERY_PAGE_SIZE, offset))
    })
    .await?;
    let links = query_all_pages(client, scope, |offset| {
        directory_links_query(DirectoryQueryPage::new(DIRECTORY_QUERY_PAGE_SIZE, offset))
    })
    .await?;
    directory_choices(&directories, &links)
}

fn directory_choices(
    directories: &[Object],
    links: &[Object],
) -> std::result::Result<Vec<DirectoryChoice>, CliError> {
    let mut titles = BTreeMap::new();
    for directory in directories {
        let id = object_string(directory, &[ATTR_ID]).ok_or_else(|| {
            CliError::InvalidInput("directory query returned an item without an ID".to_string())
        })?;
        let title = object_string(directory, &[ATTR_TITLE, "title"])
            .ok_or_else(|| CliError::InvalidInput(format!("directory '{id}' has no title")))?;
        titles.insert(id, title);
    }

    let mut children = BTreeMap::<String, Vec<String>>::new();
    let mut linked_children = BTreeSet::new();
    for link in links {
        let Some(parent_id) = object_string(
            link,
            &["directory_from", "parent_id", ATTR_DIRECTORY_NODE_FROM],
        ) else {
            continue;
        };
        let Some(child_id) = object_string(link, &["directory_to", "child_id", ATTR_RELATION_TO])
        else {
            continue;
        };
        if titles.contains_key(&parent_id) && titles.contains_key(&child_id) {
            children
                .entry(parent_id)
                .or_default()
                .push(child_id.clone());
            linked_children.insert(child_id);
        }
    }
    for child_ids in children.values_mut() {
        child_ids.sort_by(|left, right| {
            titles
                .get(left)
                .cmp(&titles.get(right))
                .then_with(|| left.cmp(right))
        });
        child_ids.dedup();
    }

    let mut roots = titles
        .keys()
        .filter(|id| !linked_children.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    sort_directory_ids(&mut roots, &titles);

    let mut choices = Vec::with_capacity(titles.len());
    let mut visited = BTreeSet::new();
    for root in roots {
        append_directory_choices(
            &root,
            Vec::new(),
            &titles,
            &children,
            &mut visited,
            &mut choices,
        );
    }
    let mut remaining = titles
        .keys()
        .filter(|id| !visited.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    sort_directory_ids(&mut remaining, &titles);
    for id in remaining {
        append_directory_choices(
            &id,
            Vec::new(),
            &titles,
            &children,
            &mut visited,
            &mut choices,
        );
    }
    Ok(choices)
}

fn sort_directory_ids(ids: &mut [String], titles: &BTreeMap<String, String>) {
    ids.sort_by(|left, right| {
        titles
            .get(left)
            .cmp(&titles.get(right))
            .then_with(|| left.cmp(right))
    });
}

fn append_directory_choices(
    id: &str,
    mut path: Vec<String>,
    titles: &BTreeMap<String, String>,
    children: &BTreeMap<String, Vec<String>>,
    visited: &mut BTreeSet<String>,
    choices: &mut Vec<DirectoryChoice>,
) {
    if !visited.insert(id.to_string()) {
        return;
    }
    let Some(title) = titles.get(id) else {
        return;
    };
    path.push(title.clone());
    choices.push(DirectoryChoice {
        id: Some(id.to_string()),
        label: format!("{}  [{id}]", path.join(" / ")),
    });
    if let Some(child_ids) = children.get(id) {
        for child_id in child_ids {
            append_directory_choices(child_id, path.clone(), titles, children, visited, choices);
        }
    }
}

fn replacement_entity(existing: Option<&RemoteEntry>, shared: &Object) -> Object {
    let mut entity = existing
        .map(|entry| entry.object.clone())
        .unwrap_or_default();
    entity.retain(|key, _| {
        !matches!(
            key.as_str(),
            "id" | "type"
                | "filestore_locator"
                | "filename"
                | "byte_size"
                | "mime_type"
                | "filekind"
                | "content_hash_sha256"
        ) && !key.starts_with("semantic:filestore:file:")
    });
    for (key, value) in shared {
        entity.insert(key.clone(), value.clone());
    }
    entity
}

fn file_merge_action(
    existing: Option<&RemoteEntry>,
    local_hash: &str,
    replace: bool,
) -> FileMergeAction {
    let Some(existing) = existing else {
        return FileMergeAction::Upload;
    };
    match &existing.kind {
        RemoteKind::File { hash: Some(hash) } if hash.eq_ignore_ascii_case(local_hash) => {
            FileMergeAction::SkipUnchanged
        }
        RemoteKind::File { .. } if replace => FileMergeAction::Replace,
        RemoteKind::File { .. } => FileMergeAction::SkipChanged,
        _ => FileMergeAction::SkipChanged,
    }
}

fn build_local_tree(root: &Path) -> std::result::Result<LocalTree, CliError> {
    let metadata = std::fs::symlink_metadata(root).map_err(|source| CliError::Io {
        action: "inspect",
        path: root.display().to_string(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::InvalidInput(format!(
            "--tree root must be a directory and not a symbolic link: {}",
            root.display()
        )));
    }
    let root = std::fs::canonicalize(root).map_err(|source| CliError::Io {
        action: "resolve",
        path: root.display().to_string(),
        source,
    })?;
    utf8_file_name(&root)?;
    let mut entries = BTreeMap::new();
    collect_tree_directory(&root, &root, &mut entries)?;
    Ok(LocalTree { root, entries })
}

fn collect_tree_directory(
    root: &Path,
    directory: &Path,
    tree: &mut BTreeMap<PathBuf, Vec<LocalEntry>>,
) -> std::result::Result<(), CliError> {
    let relative = directory
        .strip_prefix(root)
        .expect("tree directory is below root")
        .to_path_buf();
    let paths = read_sorted_directory(directory)?;
    let mut entries = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = std::fs::symlink_metadata(&path).map_err(|source| CliError::Io {
            action: "inspect",
            path: path.display().to_string(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(CliError::InvalidInput(format!(
                "symbolic links are not uploaded: {}",
                path.display()
            )));
        }
        let name = utf8_file_name(&path)?.to_string();
        let kind = if metadata.is_dir() {
            LocalEntryKind::Directory
        } else if metadata.is_file() {
            LocalEntryKind::File {
                hash: sha256_file(&path)?,
            }
        } else {
            return Err(CliError::InvalidInput(format!(
                "tree entry is not a regular file or directory: {}",
                path.display()
            )));
        };
        entries.push(LocalEntry {
            name,
            path: path.clone(),
            kind,
        });
        if metadata.is_dir() {
            collect_tree_directory(root, &path, tree)?;
        }
    }
    tree.insert(relative, entries);
    Ok(())
}

fn read_sorted_directory(path: &Path) -> std::result::Result<Vec<PathBuf>, CliError> {
    let entries = std::fs::read_dir(path).map_err(|source| CliError::Io {
        action: "read directory",
        path: path.display().to_string(),
        source,
    })?;
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|source| CliError::Io {
                    action: "read directory entry in",
                    path: path.display().to_string(),
                    source,
                })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    paths.sort();
    Ok(paths)
}

fn sha256_file(path: &Path) -> std::result::Result<String, CliError> {
    let mut file = std::fs::File::open(path).map_err(|source| CliError::Io {
        action: "open",
        path: path.display().to_string(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| CliError::Io {
            action: "read",
            path: path.display().to_string(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

async fn require_directory(
    client: &RpcClient,
    scope: Option<&String>,
    id: &str,
) -> std::result::Result<(), CliError> {
    let rows = query_rows(client, scope, directory_by_id_query(id)).await?;
    match rows.as_slice() {
        [_] => Ok(()),
        [] => Err(CliError::InvalidInput(format!(
            "target directory '{id}' was not found or is not a directory"
        ))),
        _ => Err(CliError::InvalidInput(format!(
            "target directory lookup for '{id}' returned multiple records"
        ))),
    }
}

async fn find_root_directory(
    client: &RpcClient,
    scope: Option<&String>,
    name: &str,
) -> std::result::Result<Option<String>, CliError> {
    let matches = query_rows(client, scope, root_directories_named_query(name, 2))
        .await?
        .iter()
        .filter_map(|row| object_string(row, &[ATTR_ID]))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(id.clone())),
        _ => Err(CliError::InvalidInput(format!(
            "multiple remote root directories are named '{name}'; use --target-directory"
        ))),
    }
}

async fn list_directory(
    client: &RpcClient,
    scope: Option<&String>,
    parent_id: &str,
) -> std::result::Result<Vec<RemoteEntry>, CliError> {
    query_all_pages(client, scope, |offset| {
        directory_children_query(
            parent_id,
            DirectoryChildFilter::All,
            DirectorySort::Order,
            DirectoryQueryPage::new(DIRECTORY_QUERY_PAGE_SIZE, offset),
        )
    })
    .await?
    .into_iter()
    .map(remote_entry_from_object)
    .collect()
}

fn remote_entry_from_object(object: Object) -> std::result::Result<RemoteEntry, CliError> {
    let id = object_string(&object, &[ATTR_ID]).ok_or_else(|| {
        CliError::InvalidInput("directory query returned an item without an ID".to_string())
    })?;
    let entity_type = object_string(&object, &[ATTR_TYPE]);
    let (name, kind) = match entity_type.as_deref() {
        Some(DIRECTORY_CLASS_ID) => (
            object_string(&object, &[ATTR_TITLE, "title"]),
            RemoteKind::Directory,
        ),
        Some(FILE_CLASS_ID) => (
            object_string(
                &object,
                &[ATTR_FILE_FILENAME, "filename", ATTR_TITLE, "title"],
            ),
            RemoteKind::File {
                hash: object_string(
                    &object,
                    &[ATTR_FILE_CONTENT_HASH_SHA256, "content_hash_sha256"],
                ),
            },
        ),
        _ => (
            object_string(
                &object,
                &[ATTR_TITLE, "title", ATTR_FILE_FILENAME, "filename"],
            ),
            RemoteKind::Other,
        ),
    };
    let name = name.ok_or_else(|| {
        CliError::InvalidInput(format!(
            "directory item '{id}' has no title or filename and cannot be merged safely"
        ))
    })?;
    Ok(RemoteEntry {
        id,
        name,
        kind,
        object,
    })
}

fn unique_named_entry<'a>(
    entries: &'a [RemoteEntry],
    name: &str,
    parent_id: &str,
) -> std::result::Result<Option<&'a RemoteEntry>, CliError> {
    let mut matches = entries.iter().filter(|entry| entry.name == name);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(CliError::InvalidInput(format!(
            "remote directory '{parent_id}' contains duplicate entries named '{name}'"
        )));
    }
    Ok(first)
}

fn name_type_conflict(
    parent_id: &str,
    name: &str,
    local_kind: &str,
    remote_kind: &RemoteKind,
) -> CliError {
    CliError::InvalidInput(format!(
        "cannot merge local {local_kind} '{name}' into directory '{parent_id}': a remote {} has that name",
        remote_kind_name(remote_kind)
    ))
}

fn remote_kind_name(kind: &RemoteKind) -> &'static str {
    match kind {
        RemoteKind::Directory => "directory",
        RemoteKind::File { .. } => "file",
        RemoteKind::Other => "entity",
    }
}

async fn create_directory(
    client: &RpcClient,
    scope: Option<&String>,
    name: &str,
    parent_id: Option<&str>,
) -> std::result::Result<String, CliError> {
    let id = format!("directory-{}", uuid::Uuid::new_v4());
    let mut directory = Object::new();
    directory.insert(ATTR_ID, Value::String(id.clone()));
    directory.insert(ATTR_TYPE, Value::String(DIRECTORY_CLASS_ID.to_string()));
    directory.insert(ATTR_TITLE, Value::String(name.to_string()));
    let mut operations = vec![upsert_operation(id.clone(), directory)];
    if let Some(parent_id) = parent_id {
        operations.push(link_operation(parent_id, &id));
    }
    invoke_batch(client, scope, operations).await?;
    Ok(id)
}

async fn link_entity(
    client: &RpcClient,
    scope: Option<&String>,
    parent_id: &str,
    child_id: &str,
) -> std::result::Result<(), CliError> {
    invoke_batch(client, scope, vec![link_operation(parent_id, child_id)])
        .await
        .map(|_| ())
}

fn upsert_operation(id: String, object: Object) -> Value {
    let mut operation = Object::new();
    operation.insert("kind", Value::String("upsert".to_string()));
    operation.insert("collection", Value::String(DEFAULT_COLLECTION.to_string()));
    operation.insert("id", Value::String(id));
    operation.insert("object", Value::Object(object));
    Value::Object(operation)
}

fn link_operation(parent_id: &str, child_id: &str) -> Value {
    let id = directory_node_id(parent_id, child_id);
    let mut object = Object::new();
    object.insert(ATTR_ID, Value::String(id.clone()));
    object.insert(
        ATTR_TYPE,
        Value::String(DIRECTORY_NODE_CLASS_ID.to_string()),
    );
    object.insert(
        ATTR_RELATION_RELATION,
        Value::String(DIRECTORY_NODE_RELATION_ID.to_string()),
    );
    object.insert(
        ATTR_DIRECTORY_NODE_FROM,
        Value::String(parent_id.to_string()),
    );
    object.insert(ATTR_RELATION_TO, Value::String(child_id.to_string()));
    object.insert(ATTR_DIRECTORY_NODE_ORDER, Value::U64(0));
    upsert_operation(id, object)
}

fn directory_node_id(parent_id: &str, child_id: &str) -> String {
    format!(
        "semantic:directory_node:{}:{}",
        hex::encode(parent_id.as_bytes()),
        hex::encode(child_id.as_bytes())
    )
}

async fn invoke_batch(
    client: &RpcClient,
    scope: Option<&String>,
    operations: Vec<Value>,
) -> std::result::Result<Value, CliError> {
    let mut payload = Object::new();
    if let Some(scope) = scope {
        payload.insert("scope_id", Value::String(scope.clone()));
    }
    payload.insert("operations", Value::List(operations));
    Ok(client
        .invoke_value("semantic.db.batch", Value::Object(payload))
        .await?)
}

async fn query_all_pages(
    client: &RpcClient,
    scope: Option<&String>,
    build_query: impl Fn(usize) -> String,
) -> std::result::Result<Vec<Object>, CliError> {
    let mut rows = Vec::new();
    let mut offset = 0;
    loop {
        let page = query_rows(client, scope, build_query(offset)).await?;
        let is_last_page = page.len() < DIRECTORY_QUERY_PAGE_SIZE;
        rows.extend(page);
        if is_last_page {
            return Ok(rows);
        }
        offset = offset.saturating_add(DIRECTORY_QUERY_PAGE_SIZE);
    }
}

async fn query_rows(
    client: &RpcClient,
    scope: Option<&String>,
    query: String,
) -> std::result::Result<Vec<Object>, CliError> {
    let mut payload = Object::new();
    payload.insert("query", Value::String(query));
    payload.insert("format", Value::String("sql".to_string()));
    if let Some(scope) = scope {
        payload.insert("scope_id", Value::String(scope.clone()));
    }
    let response = client
        .invoke_value("semantic.db.query", Value::Object(payload))
        .await?;
    let Value::Object(response) = response else {
        return Err(CliError::InvalidInput(
            "database query returned a non-object response".to_string(),
        ));
    };
    let Some(Value::List(rows)) = response.get("rows") else {
        return Err(CliError::InvalidInput(
            "database query response is missing select rows".to_string(),
        ));
    };
    rows.iter()
        .map(|row| match row {
            Value::Object(object) => Ok(object.clone()),
            _ => Err(CliError::InvalidInput(
                "database query returned a non-object row".to_string(),
            )),
        })
        .collect()
}

fn object_string(object: &Object, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

async fn upload_file(
    client: &RpcClient,
    scope_id: Option<String>,
    path: &Path,
    filename: &str,
    id: Option<String>,
    mime_type: Option<String>,
    entity: Object,
) -> std::result::Result<semantic_rpc::file::FileUploadResponse, CliError> {
    let content = upload_content(path).await?;
    Ok(client
        .upload_file(
            FileUploadRequest {
                scope_id,
                id,
                filename: Some(filename.to_string()),
                mime_type,
                entity,
                content,
            },
            None,
        )
        .await?)
}

async fn upload_content(path: &Path) -> std::result::Result<FileUploadContent, CliError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|source| CliError::Io {
            action: "open",
            path: path.display().to_string(),
            source,
        })?;
    let size = file
        .metadata()
        .await
        .map_err(|source| CliError::Io {
            action: "inspect",
            path: path.display().to_string(),
            source,
        })?
        .len();
    let stream = tokio_util::io::ReaderStream::new(file).map(|result| {
        result.map_err(|error| RpcClientError::Transport(format!("failed to read upload: {error}")))
    });
    Ok(FileUploadContent::Stream {
        stream: Box::pin(stream),
        size: Some(size),
    })
}

fn read_entity(path: Option<&Path>) -> std::result::Result<Object, CliError> {
    let Some(path) = path else {
        return Ok(Object::new());
    };
    let input = std::fs::read_to_string(path).map_err(|source| CliError::Io {
        action: "read",
        path: path.display().to_string(),
        source,
    })?;
    match serde_json::from_str::<Value>(&input).map_err(|source| CliError::Json {
        source_name: path.display().to_string(),
        source,
    })? {
        Value::Object(object) => Ok(object),
        _ => Err(CliError::InvalidInput(format!(
            "{} must contain a JSON object",
            path.display()
        ))),
    }
}

fn collect_files(
    paths: &[PathBuf],
    recursive: bool,
) -> std::result::Result<Vec<PathBuf>, CliError> {
    let mut files = BTreeSet::new();
    for path in paths {
        collect_path(path, recursive, &mut files)?;
    }
    Ok(files.into_iter().collect())
}

fn collect_path(
    path: &Path,
    recursive: bool,
    files: &mut BTreeSet<PathBuf>,
) -> std::result::Result<(), CliError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| CliError::Io {
        action: "inspect",
        path: path.display().to_string(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(CliError::InvalidInput(format!(
            "symbolic links are not uploaded: {}",
            path.display()
        )));
    }
    if metadata.is_file() {
        files.insert(path.to_path_buf());
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(CliError::InvalidInput(format!(
            "upload path is not a regular file or directory: {}",
            path.display()
        )));
    }
    if !recursive {
        return Err(CliError::InvalidInput(format!(
            "{} is a directory; pass --recursive or --tree to upload it",
            path.display()
        )));
    }

    for child in read_sorted_directory(path)? {
        collect_path(&child, true, files)?;
    }
    Ok(())
}

fn utf8_file_name(path: &Path) -> std::result::Result<&str, CliError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            CliError::InvalidInput(format!(
                "upload path has no valid UTF-8 filename: {}",
                path.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_file(hash: Option<&str>) -> RemoteEntry {
        RemoteEntry {
            id: "file-1".to_string(),
            name: "file.txt".to_string(),
            kind: RemoteKind::File {
                hash: hash.map(ToOwned::to_owned),
            },
            object: Object::new(),
        }
    }

    #[test]
    fn recursive_collection_is_sorted_and_deduplicated() {
        let root =
            std::env::temp_dir().join(format!("semantic-cli-upload-test-{}", std::process::id()));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).expect("create test directory");
        std::fs::write(root.join("b.txt"), b"b").expect("write b");
        std::fs::write(nested.join("a.txt"), b"a").expect("write a");

        let files =
            collect_files(&[root.clone(), root.join("b.txt")], true).expect("collect files");
        let mut expected = vec![root.join("b.txt"), nested.join("a.txt")];
        expected.sort();
        assert_eq!(files, expected);

        std::fs::remove_dir_all(&root).expect("remove test directory");
    }

    #[test]
    fn directory_requires_recursive_flag() {
        let error = collect_files(&[PathBuf::from(".")], false)
            .expect_err("directory should require recursive flag");
        assert!(error.to_string().contains("--recursive or --tree"));
    }

    #[test]
    fn tree_planning_is_sorted_and_hashes_files() {
        let root = std::env::temp_dir().join(format!(
            "semantic-cli-tree-plan-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("nested")).expect("create tree");
        std::fs::write(root.join("z.txt"), b"hello").expect("write file");
        std::fs::write(root.join("a.txt"), b"world").expect("write file");

        let tree = build_local_tree(&root).expect("build local tree");
        let root_entries = tree.entries.get(Path::new("")).expect("root entries");
        assert_eq!(
            root_entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt", "nested", "z.txt"]
        );
        let LocalEntryKind::File { hash } = &root_entries[2].kind else {
            panic!("z.txt should be a file");
        };
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        std::fs::remove_dir_all(&root).expect("remove test directory");
    }

    #[test]
    fn merge_decision_uses_hash_and_replace_flag() {
        let same = remote_file(Some("ABC"));
        assert_eq!(
            file_merge_action(Some(&same), "abc", false),
            FileMergeAction::SkipUnchanged
        );
        let changed = remote_file(Some("old"));
        assert_eq!(
            file_merge_action(Some(&changed), "new", false),
            FileMergeAction::SkipChanged
        );
        assert_eq!(
            file_merge_action(Some(&changed), "new", true),
            FileMergeAction::Replace
        );
        assert_eq!(
            file_merge_action(None, "new", false),
            FileMergeAction::Upload
        );
    }

    #[test]
    fn directory_link_identity_is_stable() {
        assert_eq!(
            directory_node_id("parent", "child"),
            "semantic:directory_node:706172656e74:6368696c64"
        );
    }

    #[test]
    fn directory_picker_choices_show_nested_paths() {
        let directory = |id: &str, title: &str| {
            [
                (ATTR_ID.to_string(), Value::String(id.to_string())),
                (ATTR_TITLE.to_string(), Value::String(title.to_string())),
            ]
            .into_iter()
            .collect::<Object>()
        };
        let link = |parent: &str, child: &str| {
            [
                ("parent_id".to_string(), Value::String(parent.to_string())),
                ("child_id".to_string(), Value::String(child.to_string())),
            ]
            .into_iter()
            .collect::<Object>()
        };
        let choices = directory_choices(
            &[
                directory("root-b", "Work"),
                directory("child", "Reports"),
                directory("root-a", "Archive"),
            ],
            &[link("root-b", "child")],
        )
        .expect("build directory choices");

        assert_eq!(
            choices,
            vec![
                DirectoryChoice {
                    id: Some("root-a".to_string()),
                    label: "Archive  [root-a]".to_string(),
                },
                DirectoryChoice {
                    id: Some("root-b".to_string()),
                    label: "Work  [root-b]".to_string(),
                },
                DirectoryChoice {
                    id: Some("child".to_string()),
                    label: "Work / Reports  [child]".to_string(),
                },
            ]
        );
    }

    #[test]
    fn directory_prompt_requires_both_terminals_and_interactive_mode() {
        assert!(should_prompt_for_directory(false, true, true));
        assert!(!should_prompt_for_directory(true, true, true));
        assert!(!should_prompt_for_directory(false, false, true));
        assert!(!should_prompt_for_directory(false, true, false));
    }

    #[cfg(unix)]
    #[test]
    fn tree_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "semantic-cli-tree-symlink-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create test directory");
        std::fs::write(root.join("target"), b"data").expect("write target");
        symlink(root.join("target"), root.join("link")).expect("create symlink");
        let error = build_local_tree(&root).expect_err("symlink should be rejected");
        assert!(error.to_string().contains("symbolic links"));
        std::fs::remove_dir_all(&root).expect("remove test directory");
    }
}
