use sha2::Digest;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use anyhow::Context;
use factordb::{
    data::DataMap,
    query::{self, select::Item},
    schema::{AttrMapExt, EntityContainer},
    AnyError, Db,
};
use semantic_core::{
    api::{self, BackendConfig},
    base::{AttrBlobUri, AttrDownloadUrl},
    plugin::PluginDescriptor,
};

use crate::{blobstore::DynBlobStore, server};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub backend: Option<BackendConfig>,
    pub token_key: String,
    pub server: Option<server::ServerConfig>,
}

struct AppState {
    require_auth: bool,
    config: BackendConfig,
    db: Db,
    blob: DynBlobStore,
}

#[derive(Clone)]
pub struct App {
    config: AppConfig,
    state: Arc<RwLock<Option<AppState>>>,
    rt: tokio::runtime::Handle,
    http_client: reqwest::Client,
}

impl App {
    /// Get a mutable reference to the app's config.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.http_client
    }

    pub fn needs_authentication(&self) -> bool {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.require_auth)
            .unwrap_or(true)
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
            .ok_or_else(|| anyhow::anyhow!("Blobstore not initialized"))
    }

    pub fn db(&self) -> Option<Db> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.db.clone())
    }

    pub fn require_db(&self) -> Result<Db, AnyError> {
        self.db()
            .ok_or_else(|| anyhow::anyhow!("Database not initialized"))
    }

    pub fn backend_config(&self) -> Option<BackendConfig> {
        self.state
            .read()
            .unwrap()
            .as_ref()
            .map(|state| state.config.clone())
    }

    // pub fn require_backend_config(&self) -> Result<BackendConfig, AnyError> {
    //     self.backend_config()
    //         .ok_or_else(|| anyhow::anyhow!("Database not initialized"))
    // }
    //

    fn default_data_path() -> Result<String, AnyError> {
        let path = dirs::data_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine default data directory"))?
            .join("semantic");

        path.to_str()
            .map(|x| x.to_string())
            .ok_or_else(|| anyhow::anyhow!("Non-UTF-8 data directory"))
    }

    pub async fn configure_backend(&self, config: BackendConfig) -> Result<(), AnyError> {
        let state = match &config {
            BackendConfig::Crypto(crypto) => {
                let data_path = if let Some(p) = &crypto.data_path {
                    p.clone()
                } else {
                    Self::default_data_path()?
                };

                let log = logfs::LogFs::open(data_path.clone(), crypto.key.clone())
                    .context(format!("Could not open logfs at '{:?}'", data_path))?;
                let blob = Arc::new(log.clone());
                let db = crate::db::logdb::LogDbStore::new(log).build_db().await?;

                AppState {
                    db,
                    blob,
                    config,
                    require_auth: false,
                }
            }
        };

        let base_plugin = semantic_core::base::SemanticPlugin::build_upsert_migration();
        state.db.migrate(base_plugin).await?;
        *self.state.write().unwrap() = Some(state);
        Ok(())
    }

    pub async fn close_backend(&self) -> Result<(), AnyError> {
        let mut lock = self
            .state
            .write()
            .map_err(|_| anyhow::anyhow!("Could not lock state"))?;
        let _state = lock
            .take()
            .ok_or_else(|| anyhow::anyhow!("Backend is not initialized"))?;

        // TODO: should probably have dedicated shutdown methods for
        // db/blobstore here.

        Ok(())
    }

    pub async fn build(config: AppConfig, rt: tokio::runtime::Handle) -> Result<Self, AnyError> {
        let s = Self {
            config: config.clone(),
            state: Arc::new(RwLock::new(None)),
            rt,
            http_client: reqwest::Client::new(),
        };

        if let Some(backend) = &config.backend {
            s.configure_backend(backend.clone()).await?;
        }

        Ok(s)
    }

    pub async fn load_schema(&self) -> Result<semantic_core::plugin::PluginSchema, AnyError> {
        let mut schema = semantic_core::base::SemanticPlugin::schema();
        // Fix up the schema with real IDs.

        let db = self.require_db()?;

        let reg = { db.backend().registry().read().unwrap().clone() };

        for entity in &mut schema.db.entities {
            if let Some(reg) = reg.entity_by_name(&entity.ident) {
                entity.id = reg.schema.id;
            }
        }
        for attr in &mut schema.db.attributes {
            if let Some(reg) = reg.attr_by_name(&attr.ident) {
                attr.id = reg.schema.id;
            }
        }

        Ok(schema)
    }

    pub async fn entity_mutate(&self, mutate: query::mutate::Mutate) -> Result<(), AnyError> {
        self.entity_batch(vec![mutate].into()).await
    }

    pub async fn entity_batch(&self, batch: query::mutate::BatchUpdate) -> Result<(), AnyError> {
        self.require_db()?.batch(batch).await
    }

    pub async fn create_file(
        &self,
        meta: api::FileUploadMetadata,
        data: Vec<u8>,
    ) -> Result<semantic_core::base::TypedFile, AnyError> {
        use semantic_core::base::TypedFile;

        let blob = self.require_blob()?;
        let db = self.require_db()?;

        let mime_guess = infer::get(&data);
        let size = data.len() as u64;

        // TODO: the blob store should also be computing the hash, so probably
        // just want to use that one.
        let raw_hash = sha2::Sha256::digest(&data);
        let hash = semantic_core::base::UniversalHash::new(
            semantic_core::base::UniversalHash::SHA256,
            &format!("{:x}", raw_hash),
        );

        let id = factordb::Id::random();
        let blob_uri = format!("files/{}", id);

        // FIXME: use unique create instead of put.
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
        db.create(id, map).await?;

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
                        return Err(anyhow::anyhow!(
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

    pub async fn import(&self, items: Vec<Item>, import_media: bool) -> Result<(), AnyError> {
        let db = self.require_db()?;
        let blob = self.require_blob()?;

        let entities = Self::entity_id_ident_fixup(&db, Item::flatten_list(items)).await?;
        let merges = entities
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

        if !import_media {
            return Ok(());
        }

        let client = reqwest::Client::new();

        // NOTE: if the download fails, the file still ends up in the database.
        for id in entity_ids {
            // Re-load the node in case it was already present before.
            let data = db.entity(id).await?;

            if let Some(_blob_uri) = data.get_attr::<AttrBlobUri>() {
                // TODO: check if blob exists.
                tracing::trace!(%id, "skipping download_url fetch - blob_url already present");
                continue;
            }

            if let Some(url) = data.get_attr::<AttrDownloadUrl>() {
                tracing::trace!(%id, %url, "downloading file for node");

                let data = client
                    .get(url.as_str())
                    .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.101 Safari/537.36")
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?;

                let tmp_path = std::path::PathBuf::from(url.as_str());
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

                blob.put(&path, data.to_vec()).await?;

                let mut patch = factordb::data::value::ValueMap::new();
                patch.insert_attr::<AttrBlobUri>(path);
                db.merge(id, patch).await?;

                tracing::debug!(?url, entity_id=%id, %size, "imported file for entity");
            }
        }

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
                        None => Err(anyhow::anyhow!("Blobstore not ready")),
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
                    .get_value()
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

    pub async fn run_server(self) -> Result<(), AnyError> {
        let config = self
            .config
            .server
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No server config provided"))?;
        crate::server::run_server(self, config).await
    }

    pub async fn run_query(
        &self,
        query: semantic_core::api::Query,
    ) -> Result<semantic_core::api::Reply, AnyError> {
        let res = match query {
            api::Query::ServerStatus => Ok(api::Reply::ServerStatus(api::ServerStatus {
                backend_initialized: self.db().is_some(),
            })),
            api::Query::Initialize { config } => {
                self.configure_backend(config.clone()).await?;
                Ok(api::Reply::Initialize)
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
            api::Query::Import {
                items,
                import_media,
            } => {
                let _items = self.import(items, import_media).await?;
                Ok(api::Reply::Import)
            }
            api::Query::Schema => {
                let schema = self.load_schema().await?;
                let reply = api::Reply::Schema(api::SemanticSchema { db: schema.db });
                Ok(reply)
            }
        };
        res.map_err(|err| {
            tracing::error!(?err, "api query failed");
            err
        })
    }
}
