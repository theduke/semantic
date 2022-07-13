//! Functionality for exporting and importing database + blob data
//! from archive files.

use std::{collections::BTreeSet, io::BufRead, path::Path};

use anyhow::{anyhow, bail};
use factordb::{
    prelude::{AttrMapExt, Batch, DataMap},
    AnyError,
};
use futures::StreamExt;
use semantic_core::base::AttrBlobUri;

use crate::{app::App, blobstore::DynBlobStore};

#[tracing::instrument(err, skip(app, output))]
pub async fn build_archive(
    app: &App,
    output: impl std::io::Write + Send + 'static,
) -> Result<(), AnyError> {
    let db = app.require_db()?;

    // TODO: don't buffer everything in memory.
    // Use some kind of pipe or a temporary file instead.

    let mut data = Vec::new();
    let mut blob_paths = BTreeSet::<String>::new();

    tracing::debug!("starting database export");
    let mut entity_count = 0;

    let mut stream = crate::db::EntitiesOrderedStream::new(db.clone(), 1_000);

    while let Some(res) = stream.next().await {
        let (_id, map) = res?;

        if let Some(path) = map.get_attr::<AttrBlobUri>() {
            blob_paths.insert(path);
        }

        let serialized = serde_json::to_vec(&map)?;
        data.extend_from_slice(&serialized);
        data.push(b'\n');
        entity_count += 1;

        if entity_count % 100 == 0 {
            tracing::debug!("exported {entity_count} entities...");
        }
    }

    tracing::debug!("loaded all entities!");
    tracing::debug!(blob_count=%blob_paths.len(), "starting blob export");

    let encoder = flate2::write::GzEncoder::new(output, flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);

    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_cksum();
    tar.append_data(&mut header, "data.json", &*data)?;

    let mut blob = app.require_blob()?;
    for path in blob_paths {
        let meta = blob
            .get_meta(&path)
            .await?
            .ok_or_else(|| anyhow!("Blob not found: '{path}'"))?;

        let full_path = format!("_blobs/{path}");
        let reader = blob.get_std_reader(&path).await?;

        let items = tokio::task::spawn_blocking(move || -> Result<(DynBlobStore, _), AnyError> {
            let mut header = tar::Header::new_gnu();
            header.set_size(meta.size);
            header.set_cksum();

            tar.append_data(&mut header, &full_path, reader)?;

            Ok((blob, tar))
        })
        .await??;
        blob = items.0;
        tar = items.1;
    }

    tar.finish()?;

    tracing::info!("Export complete");

    Ok(())
}

pub async fn import_archive(app: &App, archive_path: &Path) -> Result<(), anyhow::Error> {
    let db = app.require_db()?;
    let blob = app.require_blob()?;

    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);

    let mut batch = Batch::new();

    let mut total_blob_count: u64 = 0;

    for res in archive.entries()? {
        let entry = res?;
        let entry_path = entry.path()?;
        let path = entry_path.to_str().ok_or_else(|| {
            anyhow::anyhow!("Invalid archvie: non-utf8 path {}", entry_path.display())
        })?;

        if path.starts_with("_blobs/") {
            total_blob_count += 1;
            let real_path = path.replacen("_blobs/", "", 1);

            if blob.get_meta(&real_path).await?.is_some() {
                // TODO: check blobs for equality?
                bail!("Duplicate blob path: {real_path}");
            }
        } else if path == "data.json" {
            tracing::info!("Importing entities...");
            let reader = std::io::BufReader::new(entry);
            for line_res in reader.lines() {
                let line = line_res?;
                if line.trim().is_empty() {
                    continue;
                }

                let data: DataMap = serde_json::from_str(&line)?;
                let id = data
                    .get_id()
                    .ok_or_else(|| anyhow::anyhow!("Invalid entity without id: {data:?}"))?;

                batch = batch.and_create(factordb::query::mutate::Create { id, data });
            }
        }
    }

    db.batch(batch).await?;
    std::mem::drop(archive);

    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for (index, res) in archive.entries()?.enumerate() {
        let mut entry = res?;
        let entry_path = entry.path()?;
        let path = entry_path.to_str().ok_or_else(|| {
            anyhow::anyhow!("Invalid archvie: non-utf8 path {}", entry_path.display())
        })?;

        if path.starts_with("_blobs/") {
            let real_path = path.replacen("_blobs/", "", 1);

            let mut writer = blob.put_std_writer(&real_path).await?;
            std::io::copy(&mut entry, &mut writer)?;
            writer.flush()?;
        } else {
        }

        tracing::debug!("imported blob {index}/{total_blob_count}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use factordb::prelude::EntityContainer;
    use semantic_core::api::FileUploadMetadata;

    use super::*;

    #[test]
    fn test_archive_export_import_roundtrip() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        rt.block_on(async move {
            let (data_dir, app) =
                crate::test_app("archive-export-import-roundtrip-export", &handle).await;
            let db = app.require_db().unwrap();

            let mut files = Vec::new();

            for x in 0u8..10 {
                let data = vec![x];
                let typed = app
                    .upload_file(
                        FileUploadMetadata {
                            filename: Some(format!("x{x}.bin")),
                            title: Some(format!("x{x}.bin")),
                            url: None,
                            collection_id: None,
                            tag_ids: Vec::new(),
                        },
                        data.clone(),
                    )
                    .await
                    .unwrap();

                let id = typed.id();

                let map = db.entity(id).await.unwrap();
                files.push((map, data));
            }

            let archive_path = data_dir.join("archive.tar.gz");

            let f = std::fs::File::create(&archive_path).unwrap();
            build_archive(&app, f).await.unwrap();

            app.close_backend().await.unwrap();

            let (_, app2) =
                crate::test_app("archive-export-import-roundtrip-import", &handle).await;
            let db = app2.require_db().unwrap();
            let blob = app2.require_blob().unwrap();

            import_archive(&app2, &archive_path).await.unwrap();

            for (old_file, old_data) in files {
                let id = old_file.get_id().unwrap();
                let blob_path = old_file.get_attr::<AttrBlobUri>().unwrap();
                let new_file = db.entity(id).await.unwrap();

                assert_eq!(old_file, new_file);

                let new_data = blob.get(&blob_path).await.unwrap().unwrap();
                assert_eq!(old_data, new_data);
            }
        });
    }
}
