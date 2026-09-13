use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use futures_util::{StreamExt as _, TryStreamExt as _};
use objstore::{
    DataSource, DynObjStore, ObjStore as _, ObjStoreError, Operation, Put, SizedValueStream,
};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt as _};

use super::blob_index::{BlobIndex, valid_hash};
use super::{ImportOptions, TransferError, TransferStats, jsonl};

enum TarCommand {
    Blob {
        hash: String,
        path: PathBuf,
        done: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    Finish {
        done: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

pub(crate) async fn export_tar<W: AsyncWrite + Unpin>(
    db: &dyn crate::SemanticDb,
    store: DynObjStore,
    writer: &mut W,
    temp_dir: Option<&Path>,
) -> Result<TransferStats, TransferError> {
    let temp = super::make_temp_dir(temp_dir)?;
    let jsonl_path = temp.path().join("entities.jsonl");
    let index_path = temp.path().join("blob-index.redb");
    let archive_path = temp.path().join("export.tar");
    let stage_path = temp.path().join("blob-stage");
    let index = BlobIndex::create(&index_path)?;

    let mut jsonl_file = tokio::fs::File::create(&jsonl_path).await?;
    let mut stats = jsonl::export_jsonl(db, &mut jsonl_file, Some(temp.path()), |record| {
        index.add_entity(record)
    })
    .await?;
    jsonl_file.flush().await?;
    drop(jsonl_file);

    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let tar_path = archive_path.clone();
    let entities_path = jsonl_path.clone();
    let tar_task =
        tokio::task::spawn_blocking(move || write_tar_stream(&tar_path, &entities_path, receiver));

    let result = async {
        let mut after = None::<String>;
        while let Some((hash, candidate)) = index.next_hash(after.as_deref())? {
            let bytes = stage_source_blob(&store, &index, &hash, &candidate, &stage_path).await?;
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();
            sender
                .send(TarCommand::Blob {
                    hash: hash.clone(),
                    path: stage_path.clone(),
                    done: done_tx,
                })
                .await
                .map_err(|_| TransferError::Archive("tar writer stopped unexpectedly".into()))?;
            done_rx
                .await
                .map_err(|_| TransferError::Archive("tar writer stopped unexpectedly".into()))?
                .map_err(TransferError::Archive)?;
            stats.blobs += 1;
            stats.blob_bytes += bytes;
            after = Some(hash);
        }
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        sender
            .send(TarCommand::Finish { done: done_tx })
            .await
            .map_err(|_| TransferError::Archive("tar writer stopped unexpectedly".into()))?;
        done_rx
            .await
            .map_err(|_| TransferError::Archive("tar writer stopped unexpectedly".into()))?
            .map_err(TransferError::Archive)
    }
    .await;
    drop(sender);
    let tar_result = tar_task
        .await
        .map_err(|error| TransferError::Archive(format!("tar writer task failed: {error}")))?;
    result?;
    tar_result?;

    let mut archive = tokio::fs::File::open(&archive_path).await?;
    tokio::io::copy(&mut archive, writer).await?;
    writer.flush().await?;
    Ok(stats)
}

pub(crate) async fn import_tar<R: AsyncBufRead + Unpin>(
    db: &dyn crate::SemanticDb,
    store: DynObjStore,
    reader: &mut R,
    source_name: &str,
    options: &ImportOptions,
) -> Result<TransferStats, TransferError> {
    options.validate()?;
    let temp = super::make_temp_dir(options.temp_dir.as_deref())?;
    let archive_path = temp.path().join("input.tar");
    let mut archive_file = tokio::fs::File::create(&archive_path).await?;
    tokio::io::copy(reader, &mut archive_file).await?;
    archive_file.flush().await?;
    drop(archive_file);

    let root = temp.path().to_path_buf();
    let source = source_name.to_string();
    let max_record_bytes = options.max_record_bytes;
    let staged =
        tokio::task::spawn_blocking(move || stage_archive(&root, &source, max_record_bytes))
            .await
            .map_err(|error| {
                TransferError::Archive(format!("tar reader task failed: {error}"))
            })??;

    let index = BlobIndex::open(&staged.index_path)?;
    publish_blobs(&store, &index, &staged.blob_dir).await?;

    let file = tokio::fs::File::open(&staged.jsonl_path).await?;
    let mut reader = tokio::io::BufReader::new(file);
    let mut stats = jsonl::import_jsonl(db, &mut reader, source_name, options).await?;
    stats.blobs = staged.blobs;
    stats.blob_bytes = staged.blob_bytes;
    Ok(stats)
}

struct StagedArchive {
    jsonl_path: PathBuf,
    index_path: PathBuf,
    blob_dir: PathBuf,
    blobs: u64,
    blob_bytes: u64,
}

fn stage_archive(
    root: &Path,
    source_name: &str,
    max_record_bytes: usize,
) -> Result<StagedArchive, TransferError> {
    let archive_path = root.join("input.tar");
    let jsonl_path = root.join("entities.jsonl");
    let blob_dir = root.join("blobs");
    let index_path = root.join("blob-index.redb");
    std::fs::create_dir(&blob_dir)?;
    let index = BlobIndex::create(&index_path)?;
    let mut entities_seen = false;
    let mut blobs = 0u64;
    let mut blob_bytes = 0u64;
    let file = std::fs::File::open(&archive_path)?;
    let mut archive = tar::Archive::new(file);
    for entry in archive.entries().map_err(archive_error)? {
        let mut entry = entry.map_err(archive_error)?;
        let path = entry.path().map_err(archive_error)?;
        let path = path
            .to_str()
            .ok_or_else(|| TransferError::Archive("archive entry path is not valid UTF-8".into()))?
            .to_string();
        let kind = entry.header().entry_type();
        if kind.is_dir() && (path == "blobs" || path == "blobs/") {
            continue;
        }
        if !kind.is_file() {
            return Err(TransferError::Archive(format!(
                "archive entry '{path}' is not a regular file"
            )));
        }
        if path == "entities.jsonl" {
            if entities_seen {
                return Err(TransferError::Archive(
                    "archive contains duplicate entities.jsonl entries".into(),
                ));
            }
            entities_seen = true;
            let mut output = std::fs::File::create(&jsonl_path)?;
            std::io::copy(&mut entry, &mut output)
                .map_err(|error| TransferError::Archive(format!("read entities.jsonl: {error}")))?;
            output.sync_all()?;
            continue;
        }
        let Some(hash) = path.strip_prefix("blobs/") else {
            return Err(TransferError::Archive(format!(
                "unknown archive entry '{path}'"
            )));
        };
        if hash.contains('/') || !valid_hash(hash) {
            return Err(TransferError::Archive(format!(
                "invalid blob archive entry '{path}'"
            )));
        }
        if !index.mark_received(hash)? {
            return Err(TransferError::Archive(format!(
                "archive contains duplicate blob '{hash}'"
            )));
        }
        let output_path = blob_dir.join(hash);
        let mut output = std::fs::File::create(&output_path)?;
        let (size, digest) = copy_and_hash(&mut entry, &mut output)?;
        output.sync_all()?;
        if digest != hash {
            return Err(TransferError::Blob(format!(
                "archive blob '{hash}' has SHA-256 digest '{digest}'"
            )));
        }
        blobs += 1;
        blob_bytes += size;
    }
    if !entities_seen {
        return Err(TransferError::Archive(
            "archive does not contain entities.jsonl".into(),
        ));
    }

    jsonl::scan_jsonl_file(&jsonl_path, source_name, max_record_bytes, |record| {
        index.add_entity(record)
    })?;
    let mut after = None::<String>;
    while let Some((hash, _)) = index.next_hash(after.as_deref())? {
        if !index.is_received(&hash)? {
            return Err(TransferError::Blob(format!(
                "archive is missing referenced blob '{hash}'"
            )));
        }
        after = Some(hash);
    }
    let mut after = None::<String>;
    while let Some((hash, _)) = index.next_received(after.as_deref())? {
        if !index.contains_hash(&hash)? {
            return Err(TransferError::Blob(format!(
                "archive contains unreferenced blob '{hash}'"
            )));
        }
        after = Some(hash);
    }
    drop(index);
    Ok(StagedArchive {
        jsonl_path,
        index_path,
        blob_dir,
        blobs,
        blob_bytes,
    })
}

fn write_tar_stream(
    archive_path: &Path,
    entities_path: &Path,
    mut receiver: tokio::sync::mpsc::Receiver<TarCommand>,
) -> Result<(), TransferError> {
    let file = std::fs::File::create(archive_path)?;
    let mut builder = tar::Builder::new(file);
    append_file(&mut builder, "entities.jsonl", entities_path)?;
    while let Some(command) = receiver.blocking_recv() {
        match command {
            TarCommand::Blob { hash, path, done } => {
                let result = append_file(&mut builder, &format!("blobs/{hash}"), &path)
                    .map_err(|error| error.to_string());
                let failed = result.is_err();
                let _ = done.send(result);
                if failed {
                    return Err(TransferError::Archive(format!(
                        "failed to append blob '{hash}'"
                    )));
                }
            }
            TarCommand::Finish { done } => {
                let result = builder.finish().map_err(|error| error.to_string());
                let failed = result.is_err();
                let _ = done.send(result);
                if failed {
                    return Err(TransferError::Archive(
                        "failed to finish tar archive".into(),
                    ));
                }
                let file = builder.into_inner().map_err(archive_error)?;
                file.sync_all()?;
                return Ok(());
            }
        }
    }
    Ok(())
}

fn append_file(
    builder: &mut tar::Builder<std::fs::File>,
    archive_name: &str,
    path: &Path,
) -> Result<(), TransferError> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(size);
    header.set_cksum();
    builder
        .append_data(&mut header, archive_name, &mut file)
        .map_err(archive_error)
}

async fn stage_source_blob(
    store: &DynObjStore,
    index: &BlobIndex,
    hash: &str,
    candidate: &str,
    path: &Path,
) -> Result<u64, TransferError> {
    let mut locator = Some(candidate.to_string());
    let mut after = None::<String>;
    loop {
        if let Some(current) = locator.take()
            && let Some(mut stream) = store.get_stream(&current).await?
        {
            let mut file = tokio::fs::File::create(path).await?;
            let mut hasher = Sha256::new();
            let mut bytes = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                hasher.update(&chunk);
                bytes = bytes.saturating_add(chunk.len() as u64);
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            let actual = hex::encode(hasher.finalize());
            if actual != hash {
                return Err(TransferError::Blob(format!(
                    "blob at locator '{current}' has SHA-256 digest '{actual}', expected '{hash}'"
                )));
            }
            return Ok(bytes);
        }
        let Some((key, assoc_hash, assoc_locator, _)) = index.next_association(after.as_deref())?
        else {
            break;
        };
        after = Some(key.clone());
        if assoc_hash == hash && assoc_locator != candidate {
            locator = Some(assoc_locator);
        } else if assoc_hash.as_str() > hash {
            break;
        }
    }
    Err(TransferError::Blob(format!(
        "no referenced locator contains blob '{hash}'"
    )))
}

async fn publish_blobs(
    store: &DynObjStore,
    index: &BlobIndex,
    blob_dir: &Path,
) -> Result<(), TransferError> {
    let mut after = None::<String>;
    while let Some((association, hash, locator, mime)) = index.next_association(after.as_deref())? {
        if let Some(stream) = store.get_stream(&locator).await? {
            let actual = hash_stream(stream).await?;
            if actual != hash {
                return Err(TransferError::Blob(format!(
                    "destination locator '{locator}' already contains different content"
                )));
            }
        } else {
            let path = blob_dir.join(&hash);
            let file = tokio::fs::File::open(&path).await?;
            let size = file.metadata().await?.len();
            let stream = tokio_util::io::ReaderStream::new(file).map(|result| {
                result.map_err(|error| ObjStoreError::Io {
                    operation: Operation::Put,
                    source: Some(Box::new(error)),
                })
            });
            let stream = SizedValueStream::new(stream.boxed(), size);
            let mut put = Put::new(&locator, DataSource::Stream(stream));
            put.mime_type = mime;
            store.send_put(put).await?;
        }
        after = Some(association);
    }
    Ok(())
}

async fn hash_stream(mut stream: objstore::ValueStream) -> Result<String, TransferError> {
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.try_next().await? {
        hasher.update(&chunk);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn copy_and_hash(
    input: &mut impl Read,
    output: &mut impl Write,
) -> Result<(u64, String), TransferError> {
    let mut buffer = [0u8; 64 * 1024];
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    Ok((total, hex::encode(hasher.finalize())))
}

fn archive_error(error: impl std::fmt::Display) -> TransferError {
    TransferError::Archive(error.to_string())
}

#[cfg(all(test, feature = "storage-redb"))]
mod tests {
    use bytes::Bytes;
    use objstore::{DataSource, ObjStore as _, Put};
    use semantic_data::filestore::{ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILESTORE_LOCATOR};
    use semantic_data::schema::DbOpenMode;
    use semantic_data::value::{Object, Value};
    use semantic_db_core::{Batch, BatchOperation};
    use sha2::Digest as _;

    use crate::transfer::{ExportOptions, ImportOptions, TransferFormat};
    use crate::{AppRequestContext, Principal};

    async fn context(root: &std::path::Path) -> AppRequestContext {
        let config = crate::AppConfig::new().with_data_dir(root);
        let storage = crate::storage::resolve_storage(
            &config,
            crate::storage::StorageConfig {
                mode: DbOpenMode::AutoCreate,
                ..Default::default()
            },
        )
        .unwrap();
        let app = crate::storage::open_app(config, storage).await.unwrap();
        AppRequestContext {
            app,
            principal: Principal::system(),
            session: None,
            request_scope: None,
        }
    }

    #[tokio::test]
    async fn both_export_formats_place_relations_after_regular_entities() {
        use semantic_db_core::catalog::{ATTR_RELATION_FROM, ATTR_RELATION_TO, RELATION_CLASS_ID};

        let temp = tempfile::tempdir().unwrap();
        let source = context(&temp.path().join("source")).await;
        let db = source.resolve_db(None).await.unwrap();
        let mut batch = Batch::new();
        for id in ["a-relation", "b-regular", "c-relation", "d-regular"] {
            let mut object = Object::new();
            object.insert("id", Value::String(id.into()));
            if id.ends_with("relation") {
                object.insert("type", Value::String(RELATION_CLASS_ID.into()));
                object.insert(
                    "semantic:relation:relation",
                    Value::String("test:link".into()),
                );
                object.insert(ATTR_RELATION_FROM, Value::String("b-regular".into()));
                object.insert(ATTR_RELATION_TO, Value::String("d-regular".into()));
            }
            batch = batch.with_op(BatchOperation::Upsert {
                collection: "entities".into(),
                id: id.into(),
                object,
            });
        }
        db.execute_batch(batch).await.unwrap();
        for format in [TransferFormat::Jsonl, TransferFormat::Tar] {
            let mut output = Vec::new();
            let stats = crate::transfer::export(
                db.as_ref(),
                Some(source.default_file_store(None).await.unwrap()),
                &mut output,
                &ExportOptions {
                    format,
                    temp_dir: Some(temp.path().join("staging")),
                },
            )
            .await
            .unwrap();
            assert_eq!(stats.entities, 4);
            let jsonl = if format == TransferFormat::Tar {
                let mut archive = tar::Archive::new(output.as_slice());
                let mut entries = archive.entries().unwrap();
                let mut entry = entries.next().unwrap().unwrap();
                assert_eq!(
                    entry.path().unwrap().as_ref(),
                    std::path::Path::new("entities.jsonl")
                );
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
                bytes
            } else {
                output
            };
            let records = jsonl
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .enumerate()
                .map(|(line, bytes)| {
                    super::jsonl::decode_line("test", line as u64 + 1, bytes).unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                records
                    .iter()
                    .map(|record| record.id.as_str())
                    .collect::<Vec<_>>(),
                ["b-regular", "d-regular", "a-relation", "c-relation"]
            );
            assert_eq!(
                std::fs::read_dir(temp.path().join("staging"))
                    .unwrap()
                    .count(),
                0
            );
        }
        source.app.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn tar_round_trip_restores_only_referenced_blob_at_original_locator() {
        let temp = tempfile::tempdir().unwrap();
        let source = context(&temp.path().join("source")).await;
        let source_db = source.resolve_db(None).await.unwrap();
        let source_store = source.default_file_store(None).await.unwrap();
        let content = Bytes::from_static(b"streamed blob contents");
        let hash = hex::encode(sha2::Sha256::digest(&content));
        let locator = format!("file-sha256-{hash}/original");
        source_store
            .send_put(Put::new(&locator, DataSource::Data(content.clone())))
            .await
            .unwrap();
        source_store
            .send_put(Put::new(
                "unreferenced/orphan",
                DataSource::Data(Bytes::from_static(b"orphan")),
            ))
            .await
            .unwrap();
        let mut object = Object::new();
        object.insert("id", Value::String("file-1".into()));
        object.insert(ATTR_FILE_CONTENT_HASH_SHA256, Value::String(hash.clone()));
        object.insert(ATTR_FILE_FILESTORE_LOCATOR, Value::String(locator.clone()));
        source_db
            .execute_batch(Batch::new().with_op(BatchOperation::Upsert {
                collection: "entities".into(),
                id: "file-1".into(),
                object: object.clone(),
            }))
            .await
            .unwrap();

        let archive_path = temp.path().join("backup.tar");
        let mut archive = tokio::fs::File::create(&archive_path).await.unwrap();
        let export_stats = crate::transfer::export(
            source_db.as_ref(),
            Some(source_store),
            &mut archive,
            &ExportOptions {
                format: TransferFormat::Tar,
                temp_dir: Some(temp.path().join("tmp")),
            },
        )
        .await
        .unwrap();
        assert_eq!(export_stats.entities, 1);
        assert_eq!(export_stats.blobs, 1);
        drop(archive);
        source.app.shutdown().await.unwrap();

        let destination = context(&temp.path().join("destination")).await;
        let destination_db = destination.resolve_db(None).await.unwrap();
        let destination_store = destination.default_file_store(None).await.unwrap();
        let mut archive = tokio::fs::File::open(&archive_path).await.unwrap();
        let import_stats = crate::transfer::import(
            destination_db.as_ref(),
            Some(destination_store.clone()),
            &mut archive,
            "backup.tar",
            &ImportOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(import_stats.entities, 1);
        assert_eq!(import_stats.blobs, 1);
        let restored = destination_db
            .get("entities".into(), "file-1".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.object, object);
        assert_eq!(
            destination_store.get(&locator).await.unwrap(),
            Some(content)
        );
        assert!(
            destination_store
                .get("unreferenced/orphan")
                .await
                .unwrap()
                .is_none()
        );
        destination.app.shutdown().await.unwrap();
    }
}
