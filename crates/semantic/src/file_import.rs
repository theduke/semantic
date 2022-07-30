use std::{collections::HashSet, sync::Arc};

use anyhow::bail;
use factordb::{
    prelude::{AttrIdent, AttrMapExt, AttributeDescriptor, Batch, Id},
    query::mutate::EntityPatch,
    AnyError,
};
use futures::future::BoxFuture;
use semantic_core::{
    api::{self, FileImportMetadata, FileUploadMetadata},
    base::{AttrParent, AttrSecondaryUrl, AttrTags, AttrTitle, AttrUrl},
};

use crate::app::App;

pub(crate) fn file_upload_apply_meta(
    batch: &mut Batch,
    file: &mut semantic_core::base::File,
    collection: Option<semantic_core::base::Collection>,
    tags: Vec<semantic_core::base::Tag>,
    meta: &FileUploadMetadata,
) -> Result<(), AnyError> {
    if let Some(col) = collection {
        if !col.item_ids.contains(&file.id) {
            // File should be added to a collection, so add the db operation.
            batch
                .actions
                .push(semantic_core::base::Collection::mutate_add_item(
                    col.id, file.id,
                ));
        }
    }

    let mut patch = factordb::prelude::Patch::new();

    if tags.len() > 0 {
        let existing_tags: HashSet<Id> = file
            .extra
            .get(AttrTags::QUALIFIED_NAME)
            .and_then(|x| x.as_list())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|x| x.as_id())
            .collect();

        let new_tags = tags
            .into_iter()
            .filter(|t| !existing_tags.contains(&t.id))
            .collect::<Vec<_>>();

        for tag in new_tags {
            patch = patch.add(AttrTags::QUALIFIED_NAME, tag.id);
        }
    }

    if let Some(title) = &meta.title {
        if file.title.is_none() {
            patch = patch.add(AttrTitle::QUALIFIED_NAME, title.clone());
        }
    }

    if let Some(url) = &meta.url {
        if let Some(existing_url) = &file.url {
            if existing_url != url {
                patch = patch.add(AttrSecondaryUrl::QUALIFIED_NAME, url.clone().to_string());
            }
        } else {
            patch = patch.add(AttrUrl::QUALIFIED_NAME, url.clone().to_string());
        }
    }

    if let Some(ident) = &meta.ident {
        if let Some(old_ident) = &file.ident {
            if old_ident != ident {
                bail!("invalid metadata: ident has changed - old ident {old_ident} / new ident: {ident}");
            }
        } else {
            patch = patch.add(AttrIdent::QUALIFIED_NAME, ident.clone());
        }
    }

    if let Some(parent) = meta.parent.clone() {
        if let Some(old_parent) = file.extra.get_attr::<AttrParent>() {
            if old_parent != parent {
                bail!("invalid metadata: parent has changed - old parent {old_parent} / new parent: {parent}");
            }
        } else {
            patch = patch.add(AttrParent::QUALIFIED_NAME, parent);
        }
    }

    if !patch.0.is_empty() {
        batch
            .actions
            .push(factordb::prelude::Mutate::Patch(EntityPatch {
                id: file.id,
                patch,
            }));
    }

    Ok(())
}

async fn import_file(
    app: &App,
    path: &std::path::Path,
    collection_id: Option<Id>,
    tag_ids: Vec<Id>,
    title: Option<String>,
    url: Option<url::Url>,
) -> Result<api::FileUploadReply, AnyError> {
    let content = tokio::fs::read(path).await?;

    let meta = FileUploadMetadata {
        filename: path.file_name().map(|n| n.to_string_lossy().to_string()),
        title,
        collection_id,
        url,
        tag_ids,
        ident: None,
        parent: None,
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
            let f = import_file(
                &app,
                &path,
                meta.collection_id,
                meta.tag_ids,
                None,
                meta.url.clone(),
            )
            .await?;
            on_import(&path, &f);
        }

        Ok(())
    };

    Box::pin(f)
}

pub type FileImportCallback =
    Arc<dyn Fn(&std::path::Path, &api::FileUploadReply) + Send + Sync + 'static>;

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
        url: meta.url.clone(),
        tag_ids,
        ident: None,
        parent: meta.parent,
    };

    for path in paths {
        import_path_recursive(app, path, meta.clone(), on_import.clone()).await?;
    }

    Ok(())
}
