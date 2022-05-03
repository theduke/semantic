use std::sync::Arc;

use factordb::{
    prelude::{Batch, Id},
    AnyError,
};
use futures::future::BoxFuture;
use semantic_core::api::{FileImportMetadata, FileUploadMetadata};

use crate::app::App;

pub(crate) fn file_upload_apply_meta(
    batch: &mut Batch,
    file: &mut semantic_core::base::File,
    collection: Option<semantic_core::base::Collection>,
    tags: Vec<semantic_core::base::Tag>,
) -> Result<(), AnyError> {
    if let Some(col) = collection {
        // File should be added to a collection, so add the db operation.
        batch
            .actions
            .push(semantic_core::base::Collection::mutate_add_item(
                col.id, file.id,
            ));
    }

    if tags.len() > 0 {
        for tag in tags {
            let mutation = semantic_core::base::Tag::mutate_add_tag(file.id, tag.id);
            batch.actions.push(mutation.into());
        }
    }

    Ok(())
}

async fn import_file(
    app: &App,
    path: &std::path::Path,
    collection_id: Option<Id>,
    tag_ids: Vec<Id>,
) -> Result<semantic_core::base::TypedFile, AnyError> {
    let content = tokio::fs::read(path).await?;

    let meta = FileUploadMetadata {
        filename: path.file_name().map(|n| n.to_string_lossy().to_string()),
        title: None,
        collection_id,
        tag_ids,
    };
    let file = app.upload_file(meta, content).await?;

    Ok(file)
}

fn import_path_recursive(
    app: &App,
    path: std::path::PathBuf,
    meta: FileUploadMetadata,
    on_import: FileImportCallback,
) -> BoxFuture<'static, Result<(), AnyError>> {
    let app = app.clone();
    let f = async move {
        let fs_meta = tokio::fs::metadata(&path).await?;

        if fs_meta.is_dir() {
            let mut stream = tokio::fs::read_dir(&path).await?;
            while let Some(entry) = stream.next_entry().await? {
                import_path_recursive(&app, entry.path(), meta.clone(), on_import.clone()).await?;
            }
        } else if fs_meta.is_file() {
            let f = import_file(&app, &path, meta.collection_id, meta.tag_ids).await?;
            on_import(&path, &f);
        }

        Ok(())
    };

    Box::pin(f)
}

pub type FileImportCallback =
    Arc<dyn Fn(&std::path::Path, &semantic_core::base::TypedFile) + Send + Sync + 'static>;

pub async fn import_files(
    app: &App,
    paths: Vec<std::path::PathBuf>,
    meta: FileImportMetadata,
    on_import: FileImportCallback,
) -> Result<(), AnyError> {
    let mut tag_ids = Vec::new();
    {
        let db = app.require_db()?;
        for ident in &meta.tags {
            let tag = semantic_core::base::Tag::search_by_ident(&db, &ident).await?;
            tag_ids.push(tag.id);
        }
    }

    let meta = FileUploadMetadata {
        filename: None,
        title: None,
        collection_id: meta.collection_id,
        tag_ids,
    };

    for path in paths {
        import_path_recursive(app, path, meta.clone(), on_import.clone()).await?;
    }

    Ok(())
}
