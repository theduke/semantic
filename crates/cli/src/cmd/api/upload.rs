use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use futures::StreamExt as _;
use semantic_data::value::{Object, Value};
use semantic_rpc::RpcClientError;
use semantic_rpc::file::{FileUploadContent, FileUploadRequest};

use crate::CliError;
use crate::cmd::shared::{ApiClientArgs, OutputArgs};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Files to upload, or directories when --recursive is set.
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// Recursively upload regular files found below directory arguments.
    #[arg(long, short = 'r')]
    pub recursive: bool,

    /// JSON object merged into every uploaded file entity.
    #[arg(long, value_name = "PATH")]
    pub entity: Option<PathBuf>,

    /// Explicit record ID (valid only when exactly one file is uploaded).
    #[arg(long)]
    pub id: Option<String>,

    /// Explicit MIME type. By default the server detects it from the file.
    #[arg(long)]
    pub mime_type: Option<String>,

    #[command(flatten)]
    pub client: ApiClientArgs,

    #[command(flatten)]
    pub output: OutputArgs,
}

pub async fn run(args: Args) -> std::result::Result<(), CliError> {
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
        let content = upload_content(path).await?;
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                CliError::InvalidInput(format!(
                    "upload path has no valid UTF-8 filename: {}",
                    path.display()
                ))
            })?;
        let response = client
            .upload_file(
                FileUploadRequest {
                    scope_id: args.client.scope.clone(),
                    id: args.id.clone(),
                    filename: Some(filename.to_string()),
                    mime_type: args.mime_type.clone(),
                    entity: entity.clone(),
                    content,
                },
                None,
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
            "{} is a directory; pass --recursive to upload it",
            path.display()
        )));
    }

    let entries = std::fs::read_dir(path).map_err(|source| CliError::Io {
        action: "read directory",
        path: path.display().to_string(),
        source,
    })?;
    let mut children = entries
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
    children.sort();
    for child in children {
        collect_path(&child, true, files)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::collect_files;

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
        assert!(error.to_string().contains("--recursive"));
    }
}
