use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use anyhow::{anyhow, bail, Context};
use factdb::{
    query, AttrMapExt, AttributeMeta, ClassContainer, DataMap, Db, Expr, Id, Item, Mutate, Patch,
    Select, Timestamp, Value, ValueMap,
};
use semantic_core::{
    api::{self, ApiError, BackendConfig, DbConfig, FileImportMetadata, JobId, SemanticSchema},
    base::{
        entity_title, AttrBlobUri, AttrBlobUriWeb, AttrDownloadUrl, AttrFileName, AttrFileSize,
        AttrHash, AttrMimeType, AttrOriginalHash, AttrPreviewImageBlobUri, SemanticBasePlugin, Tag,
        Video,
    },
    core::SemanticCorePlugin,
    plugin::{FetchUrlJob, FetchUrlOutput, ImportJob, ImportOutput, PluginDescriptor},
};
use tracing_futures::Instrument;

use crate::{
    blobstore::{blobfs::TokioSpawner, memory::MemoryBlobStore, DynBlobStore},
    jobs::JobManager,
    plugin::PluginManager,
    util::media::{self, FileInfo},
};

pub use crate::plugin::deno::DenoConfig;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub backend: Option<api::BackendConfig>,
    // TODO: move to server config?
    pub token_key: String,

    pub deno: Option<DenoConfig>,
    pub tmp_dir: Option<PathBuf>,
}

impl AppConfig {
    fn tmp_dir_videos(&self) -> Result<PathBuf, anyhow::Error> {
        self.tmp_dir
            .clone()
            .map(|p| p.join("video_conversions"))
            .ok_or_else(|| {
                anyhow!(
                "No temporary directory configured. A temp dir is required for video conversions"
            )
            })
    }
}

struct AppState {
    backend_config: api::BackendConfig,
    /// Records when the backend was opened.
    /// Required for idle backend auto-closing.
    last_activity_at: std::time::Instant,
    db: Db,
    blob: DynBlobStore,
    plugins: PluginManager,
    jobs: JobManager,
}

#[derive(Clone)]
pub struct App {
    config: AppConfig,
    state: Arc<RwLock<Option<AppState>>>,
    _rt: tokio::runtime::Handle,
    http_client: reqwest::Client,
}

impl App {
    const WORKER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

    /// Get mutable reference to the app's config.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    pub fn default_data_dir() -> Result<PathBuf, anyhow::Error> {
        let home = dirs::home_dir().context("Could not determine user home directory")?;

        let path = home.join(".local").join("share").join("semantic");
        Ok(path)
    }

    pub fn random_token_key() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn blob(&self) -> Option<DynBlobStore> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.blob.clone())
    }

    pub fn require_blob(&self) -> Result<DynBlobStore, anyhow::Error> {
        self.blob()
            .ok_or_else(|| anyhow!("Blobstore not initialized"))
    }

    pub fn db(&self) -> Option<Db> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.db.clone())
    }

    pub fn require_db(&self) -> Result<Db, anyhow::Error> {
        self.db().ok_or_else(|| anyhow!("Database not initialized"))
    }

    pub fn jobs(&self) -> Option<JobManager> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.jobs.clone())
    }

    pub fn require_jobs(&self) -> Result<JobManager, anyhow::Error> {
        self.jobs()
            .ok_or_else(|| anyhow!("JobManager not initialized"))
    }

    pub fn plugins(&self) -> Option<PluginManager> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.plugins.clone())
    }

    pub fn require_plugins(&self) -> Result<PluginManager, anyhow::Error> {
        self.plugins()
            .ok_or_else(|| anyhow!("PluginManager not initialized"))
    }

    pub fn backend_config(&self) -> Option<api::BackendConfig> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.backend_config.clone())
    }

    pub async fn backend_status(&self) -> Option<api::BackendStatus> {
        let db = self.db()?;
        let blob = self.blob()?;

        let db_size = db.storage_usage().await.ok().flatten();
        // TODO: fetch
        let asset_size = blob.size_storage().await.ok().flatten();
        let storage_size = db_size.unwrap_or_default() + asset_size.unwrap_or_default();

        Some(api::BackendStatus {
            db_size,
            asset_size,
            storage_size: Some(storage_size),
        })
    }

    // pub fn require_backend_config(&self) -> Result<BackendConfig, anyhow::Error> {
    //     self.backend_config()
    //         .ok_or_else(|| anyhow::anyhow!("Database not initialized"))
    // }
    //

    pub fn default_data_path() -> Result<String, anyhow::Error> {
        let path = dirs::data_dir()
            .ok_or_else(|| anyhow!("Could not determine default data directory"))?
            .join("semantic");

        if !path.is_dir() {
            std::fs::create_dir_all(&path)?;
        }

        path.to_str()
            .map(|x| x.to_string())
            .ok_or_else(|| anyhow!("Non-UTF-8 data directory"))
    }

    pub fn build_crypto_config(
        crypto: &api::BackendCryptoConfig,
    ) -> Result<logfs::CryptoConfig, anyhow::Error> {
        let conf = logfs::CryptoConfig {
            key: crypto.key.clone().into(),
            salt: crypto
                .salt
                .clone()
                .map(|x| x.into_bytes())
                .unwrap_or(b"semantic".to_vec())
                .into(),
            iterations: if let Some(iters) = crypto.key_iterations {
                NonZeroU32::new(iters).ok_or_else(|| {
                    anyhow!("Invalid number of key iterations: must be a positive number")
                })?
            } else {
                NonZeroU32::new(3_000_000).unwrap()
            },
        };
        Ok(conf)
    }

    pub fn build_logfs(crypto: &api::BackendCryptoConfig) -> Result<logfs::LogFs, anyhow::Error> {
        let data_path = if let Some(p) = &crypto.data_path {
            PathBuf::from(p.clone())
        } else {
            PathBuf::from(Self::default_data_path()?).join("db")
        };

        let mut log_config = logfs::ConfigBuilder::new(data_path.clone())
            .raw_mode()
            .allow_create()
            .offset(crypto.offset)
            .default_chunk_size(8_000_000)
            .crypto(Self::build_crypto_config(crypto)?);
        if let Some(interval) = crypto.full_index_write_interval {
            log_config = log_config.full_index_write_interval(interval);
        }
        let log_config = log_config.build();

        let log = logfs::LogFs::<logfs::Journal2>::open(log_config)
            .map_err(|err| {
                tracing::error!(?err, "Could not open logfs");
                err
            })
            .context(format!("Could not open logfs at '{:?}'", data_path))?;

        Ok(log)
    }

    pub async fn recover_database_data(
        config: api::BackendConfig,
    ) -> Result<Vec<DataMap>, anyhow::Error> {
        match config.db {
            DbConfig::Crypto(c) => {
                let logfs = Self::build_logfs(&c)?;
                let db = crate::db::logdb::LogDbStore::new(logfs.clone());
                factor_engine::backend::log::LogDb::recover_data(db).await
            }
            DbConfig::InMemory => {
                bail!("Cannot recover data from in-memory database");
            }
            DbConfig::BlobFs(_) => {
                bail!("blobfs backend does not support database recovery");
            }
        }
    }

    async fn initialize_backend(
        &self,
        config: api::BackendConfig,
        db: Db,
        blob: DynBlobStore,
    ) -> Result<(), anyhow::Error> {
        let plugins = PluginManager::new(db.clone());

        plugins.register_plugin(SemanticBasePlugin::new()).await?;
        plugins.register_plugin(SemanticCorePlugin::new()).await?;
        plugins
            .register_plugin(semantic_extra::health::HealthPlugin::new())
            .await?;
        plugins
            .register_plugin(semantic_extra::habits::HabitsPlugin::new())
            .await?;

        if let Some(c) = &self.config.deno {
            plugins.initialize_deno(c.clone()).await?;
        };

        plugins.load_db_plugins().await?;

        // Load plugins.

        let state = AppState {
            db,
            blob,
            backend_config: config,
            last_activity_at: std::time::Instant::now(),
            plugins,
            jobs: JobManager::new(),
        };

        *self.state.write().unwrap() = Some(state);

        Ok(())
    }

    pub async fn configure_backend(&self, config: api::BackendConfig) -> Result<(), anyhow::Error> {
        tracing::info!(?config, "configuring backend");
        tracing::debug!(?config, "configuring backend");
        let (db, blob) = match &config.db {
            DbConfig::Crypto(crypto) => {
                let log = Self::build_logfs(crypto)?;

                let db = crate::db::logdb::LogDbStore::new(log.clone())
                    .build_db()
                    .await
                    .map_err(|err| {
                        tracing::error!(?err, "Could not open logfs");
                        err
                    })?;
                let blob: DynBlobStore = Arc::new(log);
                (db, blob)
            }
            DbConfig::InMemory => {
                let backend = factor_engine::backend::memory::MemoryDb::new();
                let db = factor_engine::Engine::new(backend).into_client();

                let blob: DynBlobStore = Arc::new(MemoryBlobStore::new());
                (db, blob)
            }
            DbConfig::BlobFs(b) => {
                let init = blobfs::RepoInit {
                    root_path: b.path.clone().into(),
                    name: None,
                    password: b.password.clone(),
                    key_name: None,
                };
                let repo = blobfs_async::AsyncRepo::open(init, TokioSpawner {}).await?;

                let db = crate::db::blobfs::BlobfsDbStore::new(repo.clone())
                    .build_db()
                    .await
                    .map_err(|err| {
                        tracing::error!(?err, "Could not open logfs");
                        err
                    })?;
                let blob: DynBlobStore = Arc::new(repo);
                (db, blob)
            }
        };

        self.initialize_backend(config, db, blob).await
    }

    pub async fn close_backend(&self) -> Result<(), anyhow::Error> {
        let mut lock = self
            .state
            .write()
            .map_err(|_| anyhow!("Could not lock state"))?;
        let _state = lock
            .take()
            .ok_or_else(|| anyhow!("Backend is not initialized"))?;

        // TODO: should probably have dedicated shutdown methods for
        // db/blobstore here.

        Ok(())
    }

    pub async fn build(
        config: AppConfig,
        rt: tokio::runtime::Handle,
    ) -> Result<Self, anyhow::Error> {
        // Purge old video conversion data.
        if let Ok(p) = config.tmp_dir_videos() {
            tokio::fs::remove_dir_all(&p).await.ok();
        }

        let s = Self {
            config: config.clone(),
            state: Arc::new(RwLock::new(None)),
            _rt: rt,
            http_client: reqwest::Client::new(),
        };

        if let Some(backend) = &config.backend {
            s.configure_backend(backend.clone()).await?;
        }

        tokio::spawn(s.clone().run_worker());

        Ok(s)
    }

    pub async fn build_test_app(handle: tokio::runtime::Handle) -> Result<Self, anyhow::Error> {
        let tmp_dir = std::env::temp_dir().join("semantic/test-app");

        let config = AppConfig {
            backend: Some(BackendConfig {
                db: DbConfig::InMemory,
                idle_timeout: None,
            }),
            token_key: "testkey".to_string(),
            deno: Some(DenoConfig {
                data_dir: tmp_dir.join("deno"),
                plugin_dir: None,
            }),
            tmp_dir: Some(tmp_dir.join("tmp")),
        };

        Self::build(config, handle).await
    }

    /// Runs a long-running task that periodically does maintenance work.
    async fn run_worker(self) {
        tracing::trace!("Started app worker");

        loop {
            match tokio::spawn(self.clone().run_worker_tick()).await {
                Ok(_) => tracing::trace!("app worker tick completed"),
                Err(error) => {
                    tracing::error!(?error, "worker tick failed");
                }
            }

            tokio::time::sleep(Self::WORKER_INTERVAL).await;
        }
    }

    /// Run a periodic maintenance check.
    async fn run_worker_tick(self) -> Result<(), anyhow::Error> {
        let should_close_backend = {
            let state_opt = self
                .state
                .read()
                .map_err(|_| anyhow!("Could not lock state"))?;

            state_opt
                .as_ref()
                .map(|state| {
                    if let Some(timeout) = state.backend_config.idle_timeout {
                        let time_since_opened =
                            std::time::Instant::now().duration_since(state.last_activity_at);
                        let should_close =
                            time_since_opened > std::time::Duration::from_secs(timeout);
                        should_close
                    } else {
                        false
                    }
                })
                .unwrap_or_default()
        };

        if should_close_backend {
            tracing::info!("Closing backend due to IDLE TIMEOUT");
            match self.close_backend().await {
                Ok(_) => tracing::info!("Backend was closed due to idle timeout"),
                Err(error) => {
                    tracing::error!(?error, "Backend idle close failed. Closing application.");
                    // If closing the backend fails, the application is just
                    // aborted.  This is done for safety, since the idle close
                    // timeout should be guaranteed to work.
                    std::process::exit(1);
                }
            }
        }

        Ok(())
    }

    pub async fn load_schema(&self) -> Result<SemanticSchema, anyhow::Error> {
        let db = self.require_db()?.schema().await?;

        Ok(SemanticSchema { db })
    }

    pub async fn entity_mutate(&self, mutate: query::mutate::Mutate) -> Result<(), anyhow::Error> {
        self.entity_batch(vec![mutate].into()).await
    }

    pub async fn entity_batch(&self, batch: query::mutate::Batch) -> Result<(), anyhow::Error> {
        self.require_db()?.batch(batch).await
    }

    async fn find_unused_blobs(&self) -> Result<Vec<api::BlobInfo>, anyhow::Error> {
        let db = self.require_db()?;
        let blob = self.require_blob()?;

        let keys = blob.paths_offset(0, usize::MAX).await?;

        let mut unused = Vec::new();

        for key in keys {
            if !key.starts_with("files/") {
                continue;
            }

            let entities = db
                .select(
                    Select::new().with_filter(
                        Expr::or(
                            Expr::eq(AttrBlobUri::expr(), &key),
                            Expr::eq(AttrBlobUriWeb::expr(), &key),
                        )
                        .or_with(Expr::eq(AttrPreviewImageBlobUri::expr(), &key)),
                    ),
                )
                .await?;

            if entities.items.is_empty() {
                if let Some(info) = blob.get_meta(&key).await? {
                    unused.push(api::BlobInfo {
                        key,
                        size: info.size,
                    });
                }
            }
        }

        Ok(unused)
    }

    async fn delete_unused_blobs(&self) -> Result<api::UnusedBlobsDeleted, anyhow::Error> {
        let unused = self.find_unused_blobs().await?;

        let blob = self.require_blob()?;

        let mut count = 0;
        let mut size = 0;
        for item in unused {
            tracing::trace!(key=%item.key, "deleting unused blob");
            blob.remove(&item.key).await?;
            count += 1;
            size += item.size;
        }

        Ok(api::UnusedBlobsDeleted {
            count,
            reclaimed_size: size,
        })
    }

    /// File upload logic.
    ///
    /// Wrapped function used by Self::upload_file to provide a tracing span.
    async fn upload_file_inner(
        &self,
        meta: api::FileUploadMetadata,
        data: Vec<u8>,
    ) -> Result<api::FileUploadReply, anyhow::Error> {
        use semantic_core::base::TypedFile;

        tracing::trace!(?meta, size=%data.len(), "file upload started");

        let blob = self.require_blob()?;
        let db = self.require_db()?;

        let collection = if let Some(id) = meta.collection_id {
            let col: semantic_core::base::Collection = db
                .entity(id)
                .await
                .context("Could not find collection")?
                .try_into_entity()?;
            Some(col)
        } else {
            None
        };

        let tags = if meta.tag_ids.len() > 0 {
            let mut tags = Vec::new();
            for tag_id in meta.tag_ids.clone() {
                let tag = Tag::try_from_map(db.entity(tag_id).await?)?;
                tags.push(tag);
            }
            tags
        } else {
            Vec::new()
        };

        use sha2::Digest;
        let raw_hash = sha2::Sha256::digest(&data);
        let original_hash = semantic_core::base::UniversalHash::new(
            semantic_core::base::UniversalHash::SHA256,
            &format!("{:x}", raw_hash),
        );

        let mime_guess = infer::get(&data);

        // Try to optimise.
        // TODO: add setting to disable optimisations.
        let (data, optimized_hash) =
            tokio::task::spawn_blocking(move || media::optimise_file_data(data)).await?;

        // Prevent duplicates.

        let hash = optimized_hash
            .clone()
            .unwrap_or_else(|| original_hash.clone());

        // let new_parent = if let Some(parent) = &meta.parent {
        //     match db.entity(parent.clone()).await {
        //         Ok(p) => Some(p),
        //         Err(err) if err.is::<EntityNotFound>() => None,
        //         Err(err) => return Err(err.into()),
        //     }
        // } else {
        //     None
        // };

        if let Some(old_file) =
            semantic_core::base::File::find_by_hash_or_original(&db, &hash, optimized_hash.as_ref())
                .await?
        {
            let mut file = semantic_core::base::File::try_from_map(old_file.clone())?;

            if let Some(blob_path) = &file.blob_uri {
                // Make sure the blob still exists.
                if blob.get_meta(&blob_path).await?.is_some() {
                    // TODO: also check the hash is correct?

                    let mut batch = query::mutate::Batch {
                        actions: Vec::new(),
                    };

                    crate::file_import::file_upload_apply_meta(
                        &mut batch, &mut file, collection, tags, &meta,
                    )?;

                    let final_file = if !batch.actions.is_empty() {
                        tracing::debug!(?batch, "updating existing file metadata");
                        db.batch(batch).await?;

                        // Reload final file from db.
                        db.entity(file.id).await?
                    } else {
                        tracing::trace!("existing file - metadata did not change");
                        old_file
                    };

                    return Ok(api::FileUploadReply {
                        file: final_file,
                        is_new: false,
                    });
                }
            }
        }

        let size = data.len() as u64;

        let data = media::SharedBinarData::new(data);
        let media_info =
            media::analyze_file_async(mime_guess.clone(), media::DataSource::Memory(data.clone()))
                .await
                .map_err(|error| {
                    tracing::warn!(?error, "could not analyze video");
                })
                .ok()
                .flatten();

        let id = Id::random();
        let blob_uri = format!("files/{}", id);

        let data = data.try_into_owned().unwrap();

        blob.put(&blob_uri, data).await?;
        tracing::trace!(%blob_uri, "blob persisted");

        // FIXME: prevent duplicates.

        let now = Timestamp::now();

        let (hash, original_hash) = if let Some(optimized) = optimized_hash {
            (Some(optimized), Some(original_hash))
        } else {
            (Some(original_hash), None)
        };

        let mut file = semantic_core::base::File {
            id,
            ident: None,
            title: meta.title.clone().or_else(|| meta.filename.clone()),
            filename: meta.filename.clone(),
            url: None,
            download_url: None,
            preview_image_url: None,
            blob_uri: Some(blob_uri),
            blob_uri_web: None,
            size: Some(size),
            mime_type: mime_guess.map(|x| x.mime_type().to_string()),
            original_hash,
            hash,
            created_at: Some(now),
            updated_at: Some(now),
            extra: Default::default(),
            preview_image_blob_uri: None,
            imported_at: Some(Timestamp::now()),
        };

        // Build the data.

        let mut batch = query::mutate::Batch {
            actions: Vec::new(),
        };

        crate::file_import::file_upload_apply_meta(&mut batch, &mut file, collection, tags, &meta)?;

        let item = match mime_guess.map(|x| x.mime_type()).unwrap_or_default() {
            mime if mime.starts_with("image/") => {
                let mut width = None;
                let mut height = None;
                let mut visual_hash = None;

                if let Some(FileInfo::Image(imginfo)) = media_info {
                    if let Some(dim) = imginfo.dimensions {
                        width = Some(dim.width);
                        height = Some(dim.height);
                    }
                    if let Some(hash) = imginfo.visual_hash {
                        match hash {
                            media::ImageHash::ImgHashDoubleGradient16B(b) => {
                                visual_hash = Some(Vec::from(b));
                            } // _ => {
                              //     tracing::trace!(?hash, "unsupported visual hash type");
                              // }
                        }
                    }
                }

                TypedFile::Image(semantic_core::base::Image {
                    file,
                    width,
                    height,
                    visual_hash,
                })
            }
            mime if mime.starts_with("video/") => {
                let video = if let Some(media::FileInfo::Video(info)) = media_info {
                    semantic_core::base::Video {
                        file,
                        duration: Some(info.duration.as_secs()),
                        video_has_sound: Some(info.has_audio),
                        width: info.dimensions.as_ref().map(|x| x.width),
                        height: info.dimensions.as_ref().map(|x| x.height),
                    }
                } else {
                    semantic_core::base::Video {
                        file,
                        duration: None,
                        video_has_sound: None,
                        width: None,
                        height: None,
                    }
                };

                TypedFile::Video(video)
            }
            mime if mime.starts_with("audio/") => {
                let duration = if let Some(media::FileInfo::Audio(audio)) = media_info {
                    Some(audio.duration.as_secs())
                } else {
                    None
                };

                TypedFile::Audio(semantic_core::base::Audio { file, duration })
            }
            _other => TypedFile::File(file),
        };

        let map = item.clone().into_map()?;

        batch.actions.insert(0, Mutate::create(id, map));

        if !batch.actions.is_empty() {
            tracing::debug!(entity=?item, "file created");
            db.batch(batch).await?;
        } else {
            tracing::trace!("file did not change");
        }

        let final_file = db.entity(id).await?;

        Ok(api::FileUploadReply {
            file: final_file,
            is_new: true,
        })
    }

    pub async fn upload_file(
        &self,
        meta: api::FileUploadMetadata,
        data: Vec<u8>,
    ) -> Result<api::FileUploadReply, anyhow::Error> {
        self.upload_file_inner(meta, data)
            .instrument(tracing::debug_span!("file upload"))
            .await
    }

    pub async fn import_files(
        &self,
        paths: Vec<std::path::PathBuf>,
        meta: FileImportMetadata,
        on_import: crate::file_import::FileImportCallback,
    ) -> Result<(), anyhow::Error> {
        super::file_import::import_files(self, paths, meta, on_import).await
    }

    /// Find the given items in the database based on their [`Ident`], and then
    /// fix up all attributes so they match the existing ids instead of the
    /// newly specified ones.
    async fn entity_id_ident_fixup(
        db: &Db,
        mut items: Vec<DataMap>,
    ) -> Result<Vec<DataMap>, anyhow::Error> {
        let mut map = HashMap::new();
        // FIXME: use a single query.
        for item in &mut items {
            if let Some(ident) = item.get_attr::<factdb::schema::builtin::AttrIdent>() {
                if let Ok(old_entity) = db.entity(ident.clone()).await {
                    let current_type = old_entity.get_type();
                    let new_type = item.get_type();

                    if current_type != new_type {
                        return Err(anyhow!(
                                "Could not import entity '{:?}' - entity already exists with a different type (existing: {:?}, new: {:?})",
                                ident, current_type, new_type));
                    }

                    let current_id = item.get_id();
                    let old_id = old_entity.get_id().unwrap();

                    if let Some(current) = current_id {
                        map.insert(current, old_id);
                    }
                }
            }
        }

        // Now replace all ids in any attribute with the fixed up , existing id.
        for item in &mut items {
            for value in &mut item.0.values_mut() {
                if let Value::Id(id) = value {
                    if let Some(actual_id) = map.get(&id) {
                        *id = *actual_id;
                    }
                }
            }
        }

        Ok(items)
    }

    pub async fn fetch_url(&self, job: FetchUrlJob) -> Result<FetchUrlOutput, anyhow::Error> {
        self.require_plugins()?.fetch_url(job).await
    }

    pub async fn import(&self, job: ImportJob) -> Result<ImportOutput, anyhow::Error> {
        tracing::trace!("starting import");

        let output = self.require_plugins()?.import(job.clone()).await?;
        tracing::trace!(?output, "plugin import data acquired");
        let items = Item::flatten_list(output.items);

        let db = self.require_db()?;
        let entities = Self::entity_id_ident_fixup(&db, items).await?;
        let merges = entities
            .clone()
            .into_iter()
            .map(query::mutate::Merge::try_from_map)
            .collect::<Result<Vec<_>, _>>()?;

        // TODO: set semantic/imported_at.

        let entity_ids: Vec<_> = merges.iter().map(|merge| merge.id).collect();

        let actions = merges
            .iter()
            .map(|merge| query::mutate::Mutate::Merge(merge.clone()))
            .collect();
        let batch = query::mutate::Batch { actions };
        tracing::trace!(?batch, "persisting import batch");

        db.batch(batch).await?;

        if job.import_media {
            let client = reqwest::Client::new();

            // NOTE: if the download fails, the file still ends up in the database.
            for id in entity_ids {
                tokio::spawn(
                    self.clone()
                        .download_entity_blob_content(id, client.clone()),
                );
            }
        }

        tracing::trace!("import complete");

        let items = entities.into_iter().map(Item::new).collect();

        Ok(ImportOutput { items })
    }

    async fn download_entity_blob_content(
        self,
        id: Id,
        client: reqwest::Client,
    ) -> Result<(), anyhow::Error> {
        let db = self.require_db()?;
        let blob = self.require_blob()?;

        tracing::trace!(entity_id=%id, "starting entity blob content download");

        let data = db.entity(id).await?;

        if let Some(_blob_uri) = data.get_attr::<AttrBlobUri>() {
            // TODO: check if blob exists.
            tracing::trace!(%id, "skipping download_url fetch - blob_url already present");
            return Ok(());
        }

        let download_url = if let Some(url) = data.get_attr::<AttrDownloadUrl>() {
            url
        } else {
            return Ok(());
        };
        tracing::trace!(%id, %download_url, "downloading file for entity");

        // FIXME: persist large files directly without buffering in memory.
        let data = client
                .get(download_url.as_str())
                .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.101 Safari/537.36")
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;

        use sha2::Digest;
        let raw_hash = sha2::Sha256::digest(&data);
        let original_hash = semantic_core::base::UniversalHash::new(
            semantic_core::base::UniversalHash::SHA256,
            &format!("{:x}", raw_hash),
        );

        let mime_guess = infer::get(&data);
        let (data, optimized_hash) = media::optimise_file_data(data.to_vec());
        let size = data.len();

        let mut blob_uri: Option<String> = None;

        let hash = optimized_hash
            .clone()
            .unwrap_or_else(|| original_hash.clone());

        // Prevent duplicate blobs by re-using existing file blobs.
        if let Some(data) = semantic_core::base::File::find_by_hash(&db, &hash).await? {
            if let Some(path) = data.get_attr::<AttrBlobUri>() {
                // Make sure blob exists.

                if blob.get_meta(&path).await?.is_some() {
                    tracing::debug!(blob_path=%path, "re-using existing blob for import");
                    blob_uri = Some(path);
                }
            }
        }
        let blob_path = if let Some(x) = blob_uri {
            x
        } else {
            let tmp_path = std::path::PathBuf::from(download_url.as_str());
            let filename_opt = tmp_path
                .file_name()
                .and_then(|x| x.to_str())
                .map(|x| x.to_string());

            let mut path = format!("files/{}", id);
            if let Some(filename) = filename_opt {
                path.push('/');
                path.push_str(&filename);
            }

            blob.put(&path, data.to_vec()).await?;
            path
        };

        let mut patch = ValueMap::new();
        patch.insert_attr::<AttrBlobUri>(blob_path);
        patch.insert_attr::<AttrHash>(hash);
        if optimized_hash.is_some() && optimized_hash.as_ref() != Some(&original_hash) {
            patch.insert_attr::<AttrOriginalHash>(original_hash);
        }
        patch.insert_attr::<AttrFileSize>(size as u64);
        if let Some(mime) = mime_guess {
            // TODO: handle mismatch between expected and actual mime type!
            patch.insert_attr::<AttrMimeType>(mime.mime_type().to_string());
        }
        db.merge(id, patch).await?;

        tracing::debug!(%download_url, entity_id=%id, %size, "imported file for entity");

        Ok(())
    }

    fn update_last_activity_time(&self) {
        if let Some(state) = self.state.write().unwrap().as_mut() {
            state.last_activity_at = std::time::Instant::now();
        }
    }

    pub async fn convert_file(&self, _job: api::ConvertFile) -> Result<api::Job, anyhow::Error> {
        // let db = self.require_db()?;
        // let file = semantic_core::base::File::try_from_map(db.entity(job.file_id).await?)?;

        // if job.target_format != "web" {
        //     bail!("Invalid target format '{}'", job.target_format);
        // }

        todo!()
    }

    pub async fn build_export(
        &self,
        output: impl std::io::Write + Send + Sync + 'static,
        compression: Option<crate::util::Compression>,
        skip_blobs: bool,
    ) -> Result<(), anyhow::Error> {
        #[cfg(feature = "archive")]
        {
            crate::util::archive::build_archive(self, output, compression, skip_blobs).await
        }

        #[cfg(not(feature = "archive"))]
        {
            let _ = output;
            Err(anyhow::Error::msg(
                "This semantic instance was not built with archive support. Archives not possible.",
            ))
        }
    }

    async fn optimise_video(&self, video_id: Id) -> Result<api::JobId, anyhow::Error> {
        let db = self.require_db()?;
        let store = self.require_blob()?;
        let tmp_dir = self.config().tmp_dir_videos()?;
        let video_raw = db.entity(video_id).await?;
        let title = entity_title(&video_raw);
        let video = Video::try_from_map(video_raw)?;

        let jobs = self.require_jobs()?;

        let job = jobs.register_job(crate::jobs::JobInit {
            name: format!("Optimise video: {title}"),
            steps: Vec::new(),
        });
        let job_id = job.id;

        tokio::spawn(async move {
            media::optimise_video(db, store, jobs, video, tmp_dir, job)
                .await
                .ok();
        });

        Ok(job_id)
    }

    async fn file_discard_optimised(&self, file_id: Id) -> Result<(), anyhow::Error> {
        let db = self.require_db()?;
        let blob = self.require_blob()?;

        let data = db.entity(file_id).await?;
        let file = semantic_core::base::File::try_from_map(data)?;

        let original_blob_path = file.blob_uri.ok_or_else(|| {
            anyhow!(
                "Can't delet optimised file version: file does not have an original blob attached"
            )
        })?;

        // Ensure that blob still exists.
        blob.get_meta(&original_blob_path)
            .await?
            .ok_or_else(|| anyhow!("Original blob not found"))?;

        let path = file
            .blob_uri_web
            .ok_or_else(|| anyhow!("File does not have an optimized version"))?;

        db.patch(file_id, Patch::new().remove(AttrBlobUriWeb::QUALIFIED_NAME))
            .await?;

        blob.remove(&path).await?;

        Ok(())
    }

    async fn file_discard_un_optimised(&self, file_id: Id) -> Result<(), anyhow::Error> {
        let db = self.require_db()?;
        let blob = self.require_blob()?;

        let data = db.entity(file_id).await?;
        let file = semantic_core::base::File::try_from_map(data)?;

        let optimised_path = file
            .blob_uri_web
            .ok_or_else(|| anyhow!("File does not have an optimized version"))?;
        let original_path = file
            .blob_uri
            .ok_or_else(|| anyhow!("File does not have an attached blob"))?;

        // Ensure that optimized blob still exists.
        let new_meta = blob
            .get_meta(&optimised_path)
            .await?
            .ok_or_else(|| anyhow!("Optimized blob not found"))?;

        let new_extension = PathBuf::from(&optimised_path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Optimized file does not have an extension"))?;

        let new_mime = mime_guess::from_path(&optimised_path)
            .first()
            .ok_or_else(|| anyhow!("Could not determine mime type for new blob"))?
            .to_string();
        // TODO: actually check the file mime type?
        // FIXME: update hash!

        let new_filename = file.filename.and_then(|f| {
            let mut p = PathBuf::from(f);
            p.set_extension(&new_extension);
            p.to_str().map(|x| x.to_string())
        });

        let mut patch = Patch::new()
            .replace(AttrMimeType::QUALIFIED_NAME, new_mime)
            .replace(AttrFileSize::QUALIFIED_NAME, new_meta.size)
            .replace(AttrBlobUri::QUALIFIED_NAME, optimised_path);

        if let Some(name) = new_filename {
            patch = patch.replace(AttrFileName::QUALIFIED_NAME, name);
        }

        db.patch(file_id, patch).await?;

        blob.remove(&original_path).await?;

        Ok(())
    }

    async fn file_create_preview_image_blob(
        &self,
        data: api::FileCreatePreviewImageBlob,
    ) -> Result<(), anyhow::Error> {
        let db = self.require_db()?;
        let blob = self.require_blob()?;

        let file = db.entity(data.file_id).await?;

        // TODO: validate entity type?

        let file_data = base64::decode(&data.data).context("Invalid data: not base64-encoded")?;

        let mime =
            infer::get(&file_data).ok_or_else(|| anyhow!("Could not detect image mime type"))?;
        match  mime.to_string().as_str() {
            "image/jpeg" | "image/webp" => {}
            other => bail!("Invalid image mime type: expected image/jpeg, image/png or image/webp, but got {other}"),
        }
        let extension = mime.extension();

        let blob_path = format!("files/previews/{}/preview.{}", data.file_id, extension);

        let old_preview_path = file.get_attr::<AttrPreviewImageBlobUri>();

        blob.put(&blob_path, file_data).await?;

        db.patch(
            data.file_id,
            Patch::new().replace(AttrPreviewImageBlobUri::QUALIFIED_NAME, blob_path),
        )
        .await?;

        if let Some(old) = old_preview_path {
            if let Err(error) = blob.remove(&old).await {
                tracing::warn!(
                    ?error,
                    "Could not delete previous entity preview image blog"
                );
            }
        }

        Ok(())
    }

    fn start_analyze_media(&self, force: bool) -> Result<api::Job, anyhow::Error> {
        struct Reporter {
            manager: JobManager,
            step: String,
            id: JobId,
        }

        impl media::ProgressReporter for Reporter {
            fn on_progress(&self, percent: f64, message: Option<String>) {
                self.manager
                    .job_update(
                        self.id,
                        api::JobStatus::Running {
                            step: Some(self.step.clone()),
                            progress_percent: Some(percent),
                            progress_message: message,
                        },
                    )
                    .ok();
            }
        }

        let db = self.require_db()?;
        let blob = self.require_blob()?;
        let jobs = self.require_jobs()?;

        let job = jobs.register_job(crate::jobs::JobInit {
            name: "Analyze media".to_string(),
            steps: Vec::new(),
        });

        let reporter = Reporter {
            manager: jobs.clone(),
            step: "Analyze media".to_string(),
            id: job.id,
        };

        tokio::task::spawn(async move {
            match crate::util::media::analyze_files(db, blob, force, reporter).await {
                Ok(_) => {
                    tracing::info!("media analysis complete");
                    jobs.job_update(
                        job.id,
                        api::JobStatus::Finished {
                            result: Ok("Complete".to_string()),
                        },
                    )
                    .ok();
                }
                Err(error) => {
                    tracing::error!(?error, "media analysis failed");
                    jobs.job_update(
                        job.id,
                        api::JobStatus::Finished {
                            result: Err(ApiError::from_error(&error)),
                        },
                    )
                    .ok();
                }
            }
        });

        Ok(job)
    }

    pub async fn run_query(
        &self,
        query: semantic_core::api::Query,
    ) -> Result<semantic_core::api::Reply, anyhow::Error> {
        self.update_last_activity_time();

        let res = match query {
            api::Query::ServerStatus(()) => {
                let db = self.db();

                Ok(api::Reply::ServerStatus(api::ServerStatus {
                    backend_initialized: db.is_some(),
                    backend_status: self.backend_status().await,
                }))
            }
            api::Query::Initialize(options) => {
                self.configure_backend(options).await?;

                let schema = self.load_schema().await?;
                Ok(api::Reply::Initialize(schema))
            }
            api::Query::CloseBackend(()) => {
                self.close_backend().await?;
                Ok(api::Reply::CloseBackend(()))
            }
            api::Query::Select(sel) => self
                .require_db()?
                .select_map(sel)
                .await
                .map(api::Reply::Select),
            api::Query::QuerySql(select) => {
                let items = self.select_sql(select).await?;
                Ok(api::Reply::QuerySql(items))
            }
            api::Query::Mutate(update) => self
                .entity_mutate(update)
                .await
                .map(|_| api::Reply::Mutate(())),
            api::Query::Batch(batch) => self
                .entity_batch(batch)
                .await
                .map(|_| api::Reply::Batch(())),
            api::Query::HttpFetch(req) => {
                let method = req.method.parse()?;
                let mut builder = self.http_client().request(method, req.url);
                if let Some(body) = req.body {
                    builder = builder.body(body);
                }

                if !req.headers.is_empty() {
                    for (key, value) in req.headers {
                        builder = builder.header(&key, value);
                    }
                }

                let res = builder.send().await?;

                let headers = res
                    .headers()
                    .into_iter()
                    .filter_map(|(key, value)| {
                        Some((key.to_string(), value.to_str().ok().map(|x| x.to_string())?))
                    })
                    .collect();

                let status = res.status().as_u16();
                let body_bytes = res.bytes().await?;
                let body = if body_bytes.is_empty() {
                    None
                } else {
                    Some(base64::encode(body_bytes))
                };

                Ok(api::Reply::HttpFetch(
                    semantic_core::api::SimpleHttpResponse {
                        status,
                        headers,
                        body,
                    },
                ))
            }
            api::Query::Import(job) => {
                let out = self.import(job).await?;
                Ok(api::Reply::Import(out))
            }
            api::Query::Schema(()) => {
                let schema = self.load_schema().await?;
                let reply = api::Reply::Schema(schema);
                Ok(reply)
            }
            api::Query::FetchUrl(job) => {
                let output = self.fetch_url(job).await?;
                Ok(api::Reply::FetchUrl(output))
            }
            api::Query::PluginSourceCreate(source) => {
                let source = self.require_plugins()?.create_source(source).await?;
                Ok(api::Reply::PluginSourceCreate(source))
            }
            api::Query::PluginDelete(del) => {
                self.require_plugins()?.delete_plugin(del.name).await?;
                Ok(api::Reply::PluginDelete(()))
            }
            api::Query::PluginTestFetch(spec) => {
                let out = self.require_plugins()?.test_fetch(spec).await?;
                Ok(api::Reply::PluginTestFetch(out))
            }
            api::Query::PluginSourceUpdate(source) => {
                let source = self
                    .require_plugins()?
                    .plugin_source_replace(source)
                    .await?;
                Ok(api::Reply::PluginSourceUpgrade(source))
            }
            api::Query::PluginSourceValidate(source) => {
                self.require_plugins()?
                    .plugin_source_validate(source)
                    .await?;
                Ok(api::Reply::PluginSourceValidate(()))
            }
            api::Query::JobStatus(id) => {
                let job = self
                    .require_jobs()?
                    .job(id)
                    .ok_or_else(|| anyhow!("Job not found: '{id}'"))?;
                Ok(api::Reply::JobStatus(job))
            }
            api::Query::JobEvents(id) => {
                let events = self.require_jobs()?.job_take_events(id)?;
                Ok(api::Reply::JobEvents(events))
            }
            api::Query::ConvertFile(_) => {
                todo!()
            }
            api::Query::OptimiseVideo(job) => {
                let job_id = self.optimise_video(job.video_id).await?;
                Ok(api::Reply::OptimiseVideo(api::OptimiseVideoReply {
                    job_id,
                }))
            }
            api::Query::FileDiscardUnOptimized(opt) => {
                self.file_discard_un_optimised(opt.file_id).await?;
                Ok(api::Reply::FileDiscardUnOptimised(()))
            }
            api::Query::FileDiscardOptimised(opt) => {
                self.file_discard_optimised(opt.file_id).await?;
                Ok(api::Reply::FileDiscardOptimised(()))
            }
            api::Query::FindUnusedBlobs(()) => {
                let items = self.find_unused_blobs().await?;
                Ok(api::Reply::FindUnusedBlobs { items })
            }
            api::Query::DeleteUnusedBlobs(()) => {
                let out = self.delete_unused_blobs().await?;
                Ok(api::Reply::DeleteUnusedBlobs(out))
            }
            api::Query::FileCreatePreviewImageBlob(data) => {
                self.file_create_preview_image_blob(data).await?;
                Ok(api::Reply::FileCreatePreviewImageBlob(()))
            }
            api::Query::AnalyzeMedia { force } => {
                let job = self.start_analyze_media(force)?;
                Ok(api::Reply::AnalyzeMedia(job))
            }
            api::Query::TagCreate(create) => {
                let tag = self.tag_create(create).await?;
                Ok(api::Reply::TagCreate(tag))
            }
            api::Query::RecordEntityVisit(rec) => {
                self.record_entity_visit(rec).await?;
                Ok(api::Reply::RecordEntityVisit)
            }
            api::Query::TagMerge(merge) => {
                let new_tag = self.tag_merge(merge).await?;
                Ok(api::Reply::TagMerge(new_tag))
            }
            api::Query::FindSimilarImages(options) => {
                let job = crate::util::media::run_find_similar_images_job(
                    self.require_db()?,
                    self.require_jobs()?,
                    options,
                )
                .await?;
                Ok(api::Reply::FindSimilarImages(job))
            }
        };
        res.map_err(|err| {
            tracing::error!(?err, "api query failed");
            err
        })
    }

    async fn tag_create(&self, create: api::TagCreate) -> Result<DataMap, anyhow::Error> {
        let db = self.require_db()?;
        let id = Id::random();
        let tag = Tag {
            id,
            name: create.name,
            description: None,
            parent_id: None,
            extra: Default::default(),
        };
        db.create(id, tag.clone().into_map()?).await?;
        let raw = db.entity(id).await?;
        Ok(raw)
    }

    async fn tag_merge(&self, merge: api::TagMerge) -> Result<DataMap, anyhow::Error> {
        let db = self.require_db()?;

        let source_raw = db.entity(merge.source_tag).await?;
        let target_raw = db.entity(merge.target_tag).await?;

        let source = Tag::try_from_map(source_raw)?;
        let target = Tag::try_from_map(target_raw.clone())?;

        Tag::merge_tags(&db, source, target).await?;

        Ok(target_raw)
    }

    async fn record_entity_visit(
        &self,
        rec: semantic_core::base::RecordEntityVisit,
    ) -> Result<(), anyhow::Error> {
        let db = self.require_db()?;
        rec.run(&db).await
    }

    async fn select_sql(&self, query: api::QuerySql) -> Result<Vec<DataMap>, anyhow::Error> {
        let db = self.require_db()?;
        let page = db.sql(query.query).await?;
        let items = page.items.into_iter().map(|item| item.data).collect();
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use factdb::{map, Batch};
    use semantic_core::base::AttrTags;

    use super::*;

    // TODO: move test somewhere more sensible
    #[test]
    fn test_tag_merge() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        rt.block_on(async move {
            let app = App::build_test_app(handle).await.unwrap();

            let db = app.require_db().unwrap();

            let tag1 = app
                .tag_create(api::TagCreate {
                    name: "tag1".to_string(),
                })
                .await
                .unwrap()
                .try_into_entity::<Tag>()
                .unwrap();
            let tag2 = app
                .tag_create(api::TagCreate {
                    name: "tag2".to_string(),
                })
                .await
                .unwrap()
                .try_into_entity::<Tag>()
                .unwrap();

            let id1 = Id::random();
            let id2 = Id::random();
            let e1 = map! {
                "semantic/title": "e1",
            };
            db.create(id1, e1).await.unwrap();

            let e2 = map! {
                "semantic/title": "e2",
            };

            db.create(id2, e2).await.unwrap();

            db.batch(Batch {
                actions: vec![
                    Tag::mutate_add_tag(id1, tag1.id),
                    Tag::mutate_add_tag(id2, tag2.id),
                ],
            })
            .await
            .unwrap();

            let _e1 = db.entity(id1).await.unwrap();
            let _e2 = db.entity(id2).await.unwrap();
            app.tag_merge(api::TagMerge {
                target_tag: tag1.id.into(),
                source_tag: tag2.id.into(),
            })
            .await
            .unwrap();

            let e1 = db.entity(id1).await.unwrap();
            let e2 = db.entity(id2).await.unwrap();

            let x = e1.get(AttrTags::QUALIFIED_NAME).unwrap().as_id().unwrap();
            assert_eq!(x, tag1.id);

            let x = e2.get(AttrTags::QUALIFIED_NAME).unwrap().as_id().unwrap();
            assert_eq!(x, tag1.id);
        });
    }
}
