//! Functionality for exporting and importing database + blob data
//! from archive files.

use std::collections::BTreeSet;

use anyhow::anyhow;
use factordb::{
    query::{
        expr::{self, Expr},
        select::{Order, Select},
    },
    schema::{builtin::AttrId, AttrMapExt, AttributeDescriptor},
    AnyError, Id,
};
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

    let mut last_id = Id::nil();
    tracing::debug!("starting database export");
    let mut entity_count = 0;
    loop {
        let filter = Expr::binary(AttrId::expr(), expr::BinaryOp::Gt, last_id);
        let page = db
            .select(
                Select::new()
                    .with_filter(filter)
                    .with_sort(AttrId::expr(), Order::Asc),
            )
            .await?;
        if let Some(id) = page.items.last().and_then(|item| item.data.get_id()) {
            last_id = id;
        } else {
            break;
        }

        for item in page.items {
            if let Some(path) = item.data.get_attr::<AttrBlobUri>() {
                blob_paths.insert(path);
            }

            let serialized = serde_json::to_vec(&item.data)?;
            data.extend_from_slice(&serialized);
            data.push(b'\n');
            entity_count += 1;
        }

        tracing::trace!(%entity_count, "exported entity page");
    }

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
