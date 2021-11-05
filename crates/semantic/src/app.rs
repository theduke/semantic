use sha2::Digest;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use url::Url;

use anyhow::{anyhow, bail, Context};
use factordb::{
    data::DataMap,
    query::{self, mutate::Mutate, select::Item},
    schema::{AttrMapExt, EntityContainer},
    AnyError, Db,
};
use semantic_core::{
    api::{self, DbConfig, SemanticSchema},
    base::{
        AttrBlobUri, AttrDownloadUrl, AttrHash, AttrMimeType, AttrOriginalHash, AttrUrl,
        SemanticBasePlugin, UniversalHash,
    },
    core::SemanticCorePlugin,
    plugin::{ImportItem, ImportOutput, PluginDescriptor},
};

use crate::{blobstore::DynBlobStore, plugin::PluginManager};

pub use crate::plugin::deno::DenoConfig;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub backend: Option<api::BackendConfig>,
    pub token_key: String,

    pub deno: Option<DenoConfig>,
}

struct AppState {
    backend_config: api::BackendConfig,
    /// Records when the backend was opened.
    /// Required for idle backend auto-closing.
    last_activity_at: std::time::Instant,
    db: Db,
    blob: DynBlobStore,
    plugins: PluginManager,
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

    /// Get a mutable reference to the app's config.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    pub fn default_data_dir() -> Result<PathBuf, AnyError> {
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

    pub fn require_blob(&self) -> Result<DynBlobStore, AnyError> {
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

    pub fn require_db(&self) -> Result<Db, AnyError> {
        self.db().ok_or_else(|| anyhow!("Database not initialized"))
    }

    pub fn plugins(&self) -> Option<PluginManager> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.plugins.clone())
    }

    pub fn require_plugins(&self) -> Result<PluginManager, AnyError> {
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

    // pub fn require_backend_config(&self) -> Result<BackendConfig, AnyError> {
    //     self.backend_config()
    //         .ok_or_else(|| anyhow::anyhow!("Database not initialized"))
    // }
    //

    pub fn default_data_path() -> Result<String, AnyError> {
        let path = dirs::data_dir()
            .ok_or_else(|| anyhow!("Could not determine default data directory"))?
            .join("semantic");

        path.to_str()
            .map(|x| x.to_string())
            .ok_or_else(|| anyhow!("Non-UTF-8 data directory"))
    }

    pub async fn configure_backend(&self, config: api::BackendConfig) -> Result<(), AnyError> {
        tracing::info!(?config, "configuring backend");
        tracing::debug!(?config, "configuring backend");
        let state = match &config.db {
            DbConfig::Crypto(crypto) => {
                let data_path = if let Some(p) = &crypto.data_path {
                    p.clone()
                } else {
                    Self::default_data_path()?
                };

                let log = logfs::LogFs::open(data_path.clone(), crypto.key.clone())
                    .map_err(|err| {
                        tracing::error!(?err, "Could not open logfs");
                        err
                    })
                    .context(format!("Could not open logfs at '{:?}'", data_path))?;
                let blob = Arc::new(log.clone());
                let db = crate::db::logdb::LogDbStore::new(log)
                    .build_db()
                    .await
                    .map_err(|err| {
                        tracing::error!(?err, "Could not open logfs");
                        err
                    })?;

                let plugins = PluginManager::new(db.clone());

                plugins.register_plugin(SemanticCorePlugin::new()).await?;
                plugins.register_plugin(SemanticBasePlugin::new()).await?;

                if let Some(c) = &self.config.deno {
                    plugins.initialize_deno(c.clone()).await?;
                };

                plugins.load_db_plugins().await?;

                // Load plugins.

                AppState {
                    db,
                    blob,
                    backend_config: config,
                    last_activity_at: std::time::Instant::now(),
                    plugins,
                }
            }
        };

        *self.state.write().unwrap() = Some(state);
        Ok(())
    }

    pub async fn close_backend(&self) -> Result<(), AnyError> {
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

    pub async fn build(config: AppConfig, rt: tokio::runtime::Handle) -> Result<Self, AnyError> {
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

    /// Runs a long-running task that periodically does maintenance work.
    async fn run_worker(self) {
        tracing::trace!("Started app worker");

        loop {
            match tokio::spawn(self.clone().run_worker_tick()).await {
                Ok(_) => tracing::trace!("App worker completed succesfully"),
                Err(error) => {
                    tracing::error!(?error, "worker tick failed");
                }
            }

            tokio::time::sleep(Self::WORKER_INTERVAL).await;
        }
    }

    /// Run a periodic maintenance check.
    async fn run_worker_tick(self) -> Result<(), AnyError> {
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

    pub fn load_schema(&self) -> Result<SemanticSchema, AnyError> {
        let db = self.require_db()?.schema()?;

        Ok(SemanticSchema { db })
    }

    pub async fn entity_mutate(&self, mutate: query::mutate::Mutate) -> Result<(), AnyError> {
        self.entity_batch(vec![mutate].into()).await
    }

    pub async fn entity_batch(&self, batch: query::mutate::BatchUpdate) -> Result<(), AnyError> {
        self.require_db()?.batch(batch).await
    }

    fn optimise_file_data(data: Vec<u8>) -> (Vec<u8>, UniversalHash, Option<UniversalHash>) {
        let mime_guess = infer::get(&data);
        let raw_hash = sha2::Sha256::digest(&data);
        let hash = semantic_core::base::UniversalHash::new(
            semantic_core::base::UniversalHash::SHA256,
            &format!("{:x}", raw_hash),
        );
        match mime_guess {
            Some(t) if t.mime_type().starts_with("image/") => {
                tracing::trace!("starting media optimisation");
                match crate::util::media::optimize_image_data(&data) {
                    Ok(new_data) => {
                        tracing::trace!(old_size=%data.len(), new_size=new_data.len(), "optimised image data");
                        let new_hash_raw = sha2::Sha256::digest(&new_data);
                        let new_hash = semantic_core::base::UniversalHash::new(
                            semantic_core::base::UniversalHash::SHA256,
                            &format!("{:x}", new_hash_raw),
                        );

                        (new_data, hash, Some(new_hash))
                    }
                    Err(err) => {
                        tracing::warn!(?err, "Failed to optimize image data");
                        (data, hash, None)
                    }
                }
            }
            _ => (data, hash, None),
        }
    }

    pub async fn upload_file(
        &self,
        meta: api::FileUploadMetadata,
        data: Vec<u8>,
    ) -> Result<semantic_core::base::TypedFile, AnyError> {
        use semantic_core::base::TypedFile;

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

        let mime_guess = infer::get(&data);

        // Try to optimise.
        // TODO: add setting to disable optimisations.
        let (data, hash, original_hash) = Self::optimise_file_data(data);
        let size = data.len() as u64;

        let id = factordb::Id::random();
        let blob_uri = format!("files/{}", id);

        blob.put(&blob_uri, data).await?;

        let file = semantic_core::base::File {
            id,
            ident: None,
            title: meta.title.clone().or_else(|| meta.filename.clone()),
            filename: meta.filename,
            url: None,
            download_url: None,
            preview_image_url: None,
            blob_uri: Some(blob_uri),
            size: Some(size),
            mime_type: mime_guess.map(|x| x.mime_type().to_string()),
            hash: Some(hash),
            original_hash,
            extra: Default::default(),
        };

        // Build the data.
        let item = match mime_guess.map(|x| x.mime_type()).unwrap_or_default() {
            mime if mime.starts_with("image/") => {
                TypedFile::Image(semantic_core::base::Image { file })
            }
            mime if mime.starts_with("video/") => TypedFile::Video(semantic_core::base::Video {
                file,
                duration: None,
            }),
            // mime if mime.starts_with("audio/") => {
            //     todo!()
            // }
            _other => TypedFile::File(file),
        };

        let map = item.clone().into_map()?;

        let mut batch = query::mutate::BatchUpdate::with_action(Mutate::create(id, map));

        if let Some(col) = collection {
            // File should be added to a collection, so add the db operation.
            batch
                .actions
                .push(semantic_core::base::Collection::mutate_add_item(col.id, id));
        }

        db.batch(batch).await?;

        tracing::trace!(entity=?item, "created file");

        Ok(item)
    }

    /// Find the given items in the database based on their [`Ident`], and then
    /// fix up all attributes so they match the existing ids instead of the
    /// newly specified ones.
    async fn entity_id_ident_fixup(
        db: &Db,
        mut items: Vec<DataMap>,
    ) -> Result<Vec<DataMap>, AnyError> {
        let mut map = HashMap::new();
        // FIXME: use a single query.
        for item in &mut items {
            if let Some(ident) = item.get_attr::<factordb::schema::builtin::AttrIdent>() {
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
                if let factordb::Value::Id(id) = value {
                    if let Some(actual_id) = map.get(&id) {
                        *id = *actual_id;
                    }
                }
            }
        }

        Ok(items)
    }

    pub async fn fetch_url(&self, url: Url) -> Result<ImportOutput, AnyError> {
        self.require_plugins()?.fetch_url(url.clone()).await
    }

    pub async fn import(&self, url: Url, import_media: bool) -> Result<Vec<Item>, AnyError> {
        tracing::trace!("starting import");

        let output = self.fetch_url(url).await?;

        let db = self.require_db()?;

        let flat = ImportItem::flatten(output.items);

        let mut items = flat.ready;

        if !flat.require_fetch.is_empty() {
            let plugins = self.require_plugins()?;

            for data in flat.require_fetch {
                let _id = data
                    .get_id()
                    .ok_or_else(|| anyhow!("Item to be imported does not have an ID"))?;
                let url = data.get_attr::<AttrUrl>().ok_or_else(|| {
                    anyhow!("Nested item requires separate fetch, but does not have a URL")
                })?;
                let out = plugins.fetch_url(url).await?;
                if out.items.len() != 1 {
                    bail!("Nested item fetch did not return any data");
                }

                let flat = ImportItem::flatten(out.items);
                items.extend(flat.ready);

                if !flat.require_fetch.is_empty() {
                    bail!("Nested import fetch again has nested fetches, which is not supported");
                }
            }
        }

        let entities = Self::entity_id_ident_fixup(&db, items).await?;
        let merges = entities
            .clone()
            .into_iter()
            .map(query::mutate::Merge::try_from_map)
            .collect::<Result<Vec<_>, _>>()?;

        let entity_ids: Vec<_> = merges.iter().map(|merge| merge.id).collect();

        let actions = merges
            .iter()
            .map(|merge| query::mutate::Mutate::Merge(merge.clone()))
            .collect();
        let batch = query::mutate::BatchUpdate { actions };

        db.batch(batch).await?;

        if import_media {
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

        Ok(items)
    }

    async fn download_entity_blob_content(
        self,
        id: factordb::Id,
        client: reqwest::Client,
    ) -> Result<(), AnyError> {
        let db = self.require_db()?;

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

        let data = client
                .get(download_url.as_str())
                .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.101 Safari/537.36")
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;

        let mime_guess = infer::get(&data);
        let (data, hash, original_hash) = Self::optimise_file_data(data.to_vec());

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

        let size = data.len();

        self.require_blob()?.put(&path, data.to_vec()).await?;

        let mut patch = factordb::data::value::ValueMap::new();
        patch.insert_attr::<AttrBlobUri>(path);
        patch.insert_attr::<AttrHash>(hash);
        if let Some(original) = original_hash {
            patch.insert_attr::<AttrOriginalHash>(original);
        }
        if let Some(mime) = mime_guess {
            // TODO: handle mismatch between expected and actual mime type!
            patch.insert_attr::<AttrMimeType>(mime.mime_type().to_string());
        }
        db.merge(id, patch).await?;

        tracing::debug!(%download_url, entity_id=%id, %size, "imported file for entity");

        Ok(())
    }

    #[cfg(feature = "webkit")]
    pub fn run_webview_gtk(self) -> Result<(), AnyError> {
        use gtk::{ContainerExt, WidgetExt};
        use webkit2gtk::{
            SettingsExt, URISchemeRequestExt, UserContentManagerExt, WebContextExt,
            WebInspectorExt, WebViewExt,
        };

        gtk::init().unwrap();

        let window = gtk::Window::new(gtk::WindowType::Toplevel);

        let context = webkit2gtk::WebContext::get_default().unwrap();

        let _sec = context
            .get_security_manager()
            .expect("Could not get security manager");

        // Register host:// custom URI scheme.
        {
            let app = self.clone();
            context.register_uri_scheme("host", move |request| {
                let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
                let r = request.clone();

                let mut path = match request.get_path() {
                    Some(p) => p.to_string(),
                    None => {
                        todo!()
                    }
                };

                if path.starts_with('/') {
                    path.remove(0);
                }

                let mime = mime_guess::from_path(&path).first().map(|m| m.to_string());

                let app2 = app.clone();
                app.rt.spawn(async move {
                    let res = match app2.blob() {
                        Some(blob) => blob.get(&path).await,
                        None => Err(anyhow!("Blobstore not ready")),
                    };
                    sender.send(res).expect("Could not send file load result");
                });

                receiver.attach(None, move |res| {
                    match res {
                        Ok(Some(data)) => {
                            let stream =
                                gio::MemoryInputStream::from_bytes(&glib::Bytes::from(&data));
                            r.finish(
                                &stream,
                                data.len() as i64,
                                mime.as_ref().map(|x| x.as_str()),
                            );
                        }
                        Ok(None) => {
                            tracing::trace!("host:// serving failed - file not found");
                            r.finish_error(&mut glib::error::Error::new(
                                glib::FileError::Noent,
                                "not found",
                            ));
                        }
                        Err(err) => {
                            r.finish_error(&mut glib::error::Error::new(
                                glib::FileError::Failed,
                                &err.to_string(),
                            ));
                        }
                    }
                    glib::Continue(true)
                });

                // r.finish(stream, stream_length, content_type)
            });
        }

        let manager = webkit2gtk::UserContentManager::new();

        let settings = webkit2gtk::Settings::default();
        settings.set_enable_developer_extras(true);
        settings.set_enable_write_console_messages_to_stdout(true);
        settings.set_allow_file_access_from_file_urls(true);
        settings.set_allow_universal_access_from_file_urls(true);
        settings.set_default_font_family("DejaVu Serif");
        settings.set_enable_media(true);
        settings.set_enable_media_stream(true);
        settings.set_enable_plugins(true);

        let webview = webkit2gtk::WebViewBuilder::new()
            .web_context(&context)
            .user_content_manager(&manager)
            .is_ephemeral(true)
            .settings(&settings)
            .build();

        // Register message handler.
        {
            let app = self.clone();
            let webview = webview.clone();
            manager.connect_script_message_received(move |_manager, js_result| {
                eprintln!("script_message_received");

                let (sender, receiver) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);

                // FIXME: no unwraps!
                let ctx = js_result
                    .get_global_context()
                    .expect("Could not get context");

                let value = js_result
                    .js_value()
                    .and_then(|v| v.to_string(&ctx))
                    .expect("Expected a string");
                let query: api::QueryWithId = serde_json::from_str(&value).unwrap();

                let query_id = query.id;

                let app2 = app.clone();
                app.rt.spawn(async move {
                    let res = app2.run_query(query.query).await;
                    tracing::trace!(?res, "callback response");
                    sender.send(res).expect("Could not send api query result");
                });

                let webview = webview.clone();
                receiver.attach(None, move |res| {
                    let code = match res {
                        Ok(reply) => {
                            let reply_json = serde_json::to_string(&reply).unwrap();
                            format!("CALLBACKS[{}].resolve({})", query_id, reply_json)
                        }
                        Err(err) => {
                            format!(
                                "CALLBACKS[{}].reject(\"{}\")",
                                query_id,
                                err.to_string().replace("\"", "\\\"")
                            )
                        }
                    };

                    // TODO: check _result for errors.
                    webview.run_javascript(&code, None::<&gio::Cancellable>, |_result| {});
                    glib::Continue(true)
                });
            });

            let flag = manager.register_script_message_handler("semantics");
            assert!(flag, "could not register script message handler");
        }

        // FIXME: only for debug!
        // let path = std::env::current_dir()
        //     .ok()
        //     .and_then(|x| x.to_str().map(|x| x.to_string()))
        //     .context("Could not get current path")?;
        // let full_path = format!("file://{}/ui/index.html", path);
        webview.load_uri("http://localhost:8000/index.html");
        window.add(&webview);

        window.show_all();

        let inspector = webview.get_inspector().unwrap();
        inspector.show();

        // let js = std::fs::read_to_string("ui/build/app.js").context("Could not load JS")?;
        // webview.run_javascript(&js, None::<&gio::Cancellable>, |_result| {});

        // let cancellable = gio::Cancellable::new();
        // webview.run_javascript("42", Some(&cancellable), |result| match result {
        //     Ok(result) => {
        //         let context = result.get_global_context().unwrap();
        //         let value = result.get_value().unwrap();
        //         println!("is_boolean: {}", value.is_boolean(&context));
        //         println!("is_number: {}", value.is_number(&context));
        //         println!("{:?}", value.to_number(&context));
        //         println!("{:?}", value.to_boolean(&context));
        //     }
        //     Err(error) => println!("{}", error),
        // });

        window.connect_delete_event(|_, _| {
            gtk::main_quit();
            gtk::Inhibit(false)
        });

        gtk::main();

        Ok(())
    }

    fn update_last_activity_time(&self) {
        if let Some(state) = self.state.write().unwrap().as_mut() {
            state.last_activity_at = std::time::Instant::now();
        }
    }

    #[tracing::instrument(level = "trace", skip(self), err)]
    pub async fn run_query(
        &self,
        query: semantic_core::api::Query,
    ) -> Result<semantic_core::api::Reply, AnyError> {
        self.update_last_activity_time();

        let res = match query {
            api::Query::ServerStatus => Ok(api::Reply::ServerStatus(api::ServerStatus {
                backend_initialized: self.db().is_some(),
            })),
            api::Query::Initialize(options) => {
                self.configure_backend(options).await?;

                let schema = self.load_schema()?;
                Ok(api::Reply::Initialize(schema))
            }
            api::Query::CloseBackend => {
                self.close_backend().await?;
                Ok(api::Reply::CloseBackend)
            }
            api::Query::Select(sel) => self.require_db()?.select(sel).await.map(api::Reply::Select),
            api::Query::Mutate(update) => {
                self.entity_mutate(update).await.map(|_| api::Reply::Mutate)
            }
            api::Query::Batch(batch) => self.entity_batch(batch).await.map(|_| api::Reply::Batch),
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
            api::Query::Import { url, import_media } => {
                let items = self.import(url, import_media).await?;
                Ok(api::Reply::Import { items })
            }
            api::Query::Schema => {
                let schema = self.load_schema()?;
                let reply = api::Reply::Schema(schema);
                Ok(reply)
            }
            api::Query::FetchUrl { url } => {
                let output = self.fetch_url(url).await?;
                Ok(api::Reply::FetchUrl(output))
            }
            api::Query::PluginSourceCreate(source) => {
                let source = self.require_plugins()?.create_source(source).await?;
                Ok(api::Reply::PluginSourceCreate(source))
            }
            api::Query::PluginDelete { name } => {
                self.require_plugins()?.delete_plugin(name).await?;
                Ok(api::Reply::PluginDelete)
            }
            api::Query::PluginTestFetch(spec) => {
                let out = self.require_plugins()?.test_fetch(spec).await?;
                Ok(api::Reply::PluginTestFetch(out))
            }
        };
        res.map_err(|err| {
            tracing::error!(?err, "api query failed");
            err
        })
    }
}
