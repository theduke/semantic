use std::{path::PathBuf, sync::Arc};

use anyhow::anyhow;
use semantics_core::{api, db::DynDb, AnyError};

use crate::blobstore::DynBlobStore;

pub struct AppConfig {
    pub data_path: PathBuf,
    pub key: String,
}

#[derive(Clone)]
pub struct App {
    db: DynDb,
    blob: DynBlobStore,
    rt: tokio::runtime::Handle,
    http_client: reqwest::Client,
}

impl App {

    pub fn blob(&self) -> &DynBlobStore {
        &self.blob
    }

    pub fn build(config: AppConfig, rt: tokio::runtime::Handle) -> Result<Self, AnyError> {
        let db = crate::db::logdb::LogDb::open(config.data_path.clone(), config.key.clone())?;
        let blob = db.log().clone();

        Ok(Self {
            db: Arc::new(db),
            blob: Arc::new(blob),
            rt,
            http_client: reqwest::Client::new(),
        })
    }

    pub async fn run_api_query(&self, query: api::Query) -> Result<api::Reply, AnyError> {
        tracing::trace!(?query, "running api query");
        match query {
            api::Query::Nodes(query) => self.db.nodes(query).await.map(api::Reply::Nodes),
            api::Query::NodeMerge(node) => self
                .db
                .node_merge(node)
                .await
                .map(|_| api::Reply::NodeUpsert),
            api::Query::NodeDelete(id) => self
                .db
                .node_delete(id)
                .await
                .map(|_| api::Reply::NodeDelete),
            api::Query::Relations(q) => self.db.relations(q).await.map(api::Reply::Relations),
            api::Query::RelationMerge(rel) => self
                .db
                .relation_merge(rel)
                .await
                .map(|_| api::Reply::RelationUpsert),
            api::Query::RelationDelete(id) => self
                .db
                .relation_delete(id)
                .await
                .map(|_| api::Reply::RelationDelete),
            api::Query::Batch(events) => self.db.batch(events).await.map(|_| api::Reply::Batch),
            api::Query::HttpFetch(req) => {
                let method = req.method.parse()?;
                let mut builder = self.http_client.request(method, req.url);
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
                    semantics_core::api::SimpleHttpResponse {
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
        }
    }

    async fn import(
        &self,
        items: Vec<semantics_core::NodeItem>,
        import_media: bool,
    ) -> Result<(), AnyError> {
        let batch = semantics_core::db::DbEvent::nodes_merge_batch(items.clone()).as_events();
        self.db.batch(batch).await?;

        if !import_media {
            return Ok(());
        }

        let client = reqwest::Client::new();

        let mut nodes = Vec::new();
        let mut relations = Vec::new();
        for item in items {
            item.flatten_into(&mut nodes, &mut relations);
        }

        for node in &mut nodes {
            // Re-load the node in case it was already present before.
            let node = self.db.node_by_uri(node.uri.clone()).await?;

            if node.data.get("semantics.io/fields/blob_url").is_some() {
                tracing::trace!(?node.id, "skipping download_url fetch - blob_url already present");
                continue;
            }

            if let Some(url_value) = node.data.get("semantics.io/fields/download_url").cloned() {
                let url = url_value.as_str().ok_or_else(|| {
                    anyhow!(
                        "Invalid type for download_url field - expected a string: {:?}",
                        url_value
                    )
                })?;

                tracing::trace!(?node.id, url, "downloading file for node");

                let data = client
                    .get(url)
                    .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.101 Safari/537.36")
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?;

                let tmp_path = std::path::PathBuf::from(url);

                let filename_opt = tmp_path
                    .file_name()
                    .and_then(|x| x.to_str())
                    .map(|x| x.to_string());

                let mut path = format!("files/{}", node.uri);
                if let Some(filename) = filename_opt {
                    path.push('/');
                    path.push_str(&filename);
                }

                self.blob.put(&path, data.to_vec()).await?;

                let patch =
                    semantics_core::Patch::new().with_set("semantics.io/fields/blob_url", path);

                let _new_node = self.db.node_patch(node.id, patch).await?;
            }
        }

        Ok(())
    }

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
                    let res = app2.blob.get(&path).await;
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
                    let res = app2.run_api_query(query.query).await;
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
}
