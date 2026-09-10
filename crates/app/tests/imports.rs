use futures_util::StreamExt;
use semantic_app::{AppRequestContext, DbScopeId, Principal, SemanticApp, SemanticDb};
use semantic_data::{Object, schema::DbOpenMode};
use semantic_import::{ContentFrame, GenericUrlPlugin, SourceRequest};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct CancellationPlugin {
    inner: GenericUrlPlugin,
    manifest: semantic_plugin::PluginManifest,
    gate: Arc<InvocationGate>,
}
struct InvocationGate {
    method: &'static str,
    entered: tokio::sync::Semaphore,
    cancelled: tokio::sync::Semaphore,
}
struct CancellationImplementation {
    inner: Arc<dyn semantic_rpc::interface::InterfaceImplementation>,
    gate: Arc<InvocationGate>,
}
impl semantic_plugin::Plugin for CancellationPlugin {
    fn manifest(&self) -> &semantic_plugin::PluginManifest {
        &self.manifest
    }
    fn create<'a>(
        &'a self,
        context: semantic_plugin::PluginInstanceContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Arc<dyn semantic_rpc::interface::InterfaceImplementation>,
                        semantic_plugin::PluginError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let inner = semantic_plugin::Plugin::create(&self.inner, context).await?;
            Ok(Arc::new(CancellationImplementation {
                inner,
                gate: self.gate.clone(),
            })
                as Arc<
                    dyn semantic_rpc::interface::InterfaceImplementation,
                >)
        })
    }
}
impl semantic_rpc::interface::InterfaceImplementation for CancellationImplementation {
    fn descriptors(&self) -> &[semantic_rpc::interface::ImplementationDescriptor] {
        self.inner.descriptors()
    }
    fn invoke<'a>(
        &'a self,
        call: semantic_rpc::interface::ValidatedInvocation,
        context: semantic_rpc::interface::InvocationContext,
    ) -> semantic_rpc::interface::InvocationFuture<'a> {
        Box::pin(async move {
            if call.method == self.gate.method {
                self.gate.entered.add_permits(1);
                context.cancellation.cancelled().await;
                self.gate.cancelled.add_permits(1);
                return Err(semantic_rpc::interface::InvocationError::new(
                    "cancelled",
                    "observed cancellation",
                ));
            }
            self.inner.invoke(call, context).await
        })
    }
}

#[tokio::test]
async fn abandoned_application_calls_cancel_describe_probe_and_pending_fetch() {
    use semantic_rpc::interface::*;
    for method in ["describe", "probe", "fetch"] {
        for disconnect in [false, true] {
            let gate = Arc::new(InvocationGate {
                method,
                entered: tokio::sync::Semaphore::new(0),
                cancelled: tokio::sync::Semaphore::new(0),
            });
            let temp = tempfile::tempdir().unwrap();
            let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
                semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
                    .unwrap(),
            ));
            let exports = ["Source", "Fetcher", "Importer"]
                .into_iter()
                .map(|name| ImplementationDescriptor {
                    export: name.to_lowercase(),
                    interface: InterfaceRef {
                        package: "semantic.import".into(),
                        module: "v1".into(),
                        contract: None,
                        name: name.into(),
                    },
                    package_version: "1.0.0".into(),
                    fingerprint: semantic_data::schema::interface_fingerprint(
                        &semantic_data::import::package().root.interfaces[name],
                        &Default::default(),
                    )
                    .unwrap(),
                })
                .collect();
            let inner = GenericUrlPlugin::new(exports);
            let mut manifest = semantic_plugin::Plugin::manifest(&inner).clone();
            manifest.id = "test.cancellation".into();
            let app = SemanticApp::builder()
                .with_default_scope(DbScopeId::new("main"), db)
                .register_plugin(CancellationPlugin {
                    inner,
                    manifest,
                    gate: gate.clone(),
                })
                .unwrap()
                .build()
                .unwrap();
            let context = AppRequestContext {
                app: app.clone(),
                principal: Principal::system(),
                session: None,
                request_scope: None,
            };
            let (client, server, transport_disconnect) = app_session_with_disconnect(
                semantic_app::interface::implementation(context).unwrap(),
            );
            let caller = client.clone();
            let request = SourceRequest {
                url: "http://localhost/fixture.txt".into(),
                options: Object::new(),
            };
            let task = tokio::spawn(async move {
                let semantic_data::Value::Object(mut request) = request.to_value() else {
                    unreachable!()
                };
                request.insert("plugin_id", "test.cancellation".to_owned());
                request.insert(
                    "export",
                    if method == "fetch" {
                        "fetcher"
                    } else {
                        "importer"
                    }
                    .to_owned(),
                );
                caller
                    .invoke(
                        ValidatedInvocation {
                            export: "application".into(),
                            method: if method == "fetch" {
                                "fetch_source"
                            } else {
                                "list_candidates"
                            }
                            .into(),
                            arguments: vec![InvocationArgument::Value(
                                semantic_data::Value::Object(request),
                            )],
                        },
                        InvocationContext::default(),
                    )
                    .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.acquire())
                .await
                .unwrap_or_else(|_| panic!("did not enter {method}, disconnect={disconnect}"))
                .unwrap()
                .forget();
            if disconnect {
                transport_disconnect.cancel();
            } else {
                task.abort();
            }
            tokio::time::timeout(std::time::Duration::from_secs(5), gate.cancelled.acquire())
                .await
                .unwrap()
                .unwrap()
                .forget();
            let _ = client.shutdown().await;
            let _ = server.shutdown().await;
            app.shutdown().await.unwrap();
        }
    }
}

struct MultiSourcePlugin {
    manifest: semantic_plugin::PluginManifest,
    implementation: GenericUrlPlugin,
}
impl semantic_plugin::Plugin for MultiSourcePlugin {
    fn manifest(&self) -> &semantic_plugin::PluginManifest {
        &self.manifest
    }

    fn create<'a>(
        &'a self,
        context: semantic_plugin::PluginInstanceContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Arc<dyn semantic_rpc::interface::InterfaceImplementation>,
                        semantic_plugin::PluginError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        semantic_plugin::Plugin::create(&self.implementation, context)
    }
}

#[tokio::test]
async fn multiple_sources_bind_each_operation_and_allow_explicit_selection() {
    use semantic_plugin::Plugin;
    let exports = [
        ("source-a", "Source"),
        ("source-b", "Source"),
        ("importer-a", "Importer"),
        ("importer-b", "Importer"),
        ("fetcher-a", "Fetcher"),
        ("fetcher-b", "Fetcher"),
    ]
    .into_iter()
    .map(
        |(export, interface)| semantic_rpc::interface::ImplementationDescriptor {
            export: export.into(),
            interface: semantic_rpc::interface::InterfaceRef {
                package: "semantic.import".into(),
                module: "v1".into(),
                contract: None,
                name: interface.into(),
            },
            package_version: "1.0.0".into(),
            fingerprint: semantic_data::schema::interface_fingerprint(
                &semantic_data::import::package().root.interfaces[interface],
                &Default::default(),
            )
            .unwrap(),
        },
    )
    .collect();
    let implementation = GenericUrlPlugin::new(exports);
    let mut manifest = implementation.manifest().clone();
    manifest.id = "test.multi-source".into();
    // Cross the names so matching by order or suffix cannot accidentally pass.
    manifest.source_bindings = [
        ("importer-a", "source-b"),
        ("importer-b", "source-a"),
        ("fetcher-a", "source-b"),
        ("fetcher-b", "source-a"),
    ]
    .into_iter()
    .map(|(operation, source)| (operation.into(), source.into()))
    .collect();
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db)
        .register_plugin(MultiSourcePlugin {
            manifest,
            implementation,
        })
        .unwrap()
        .build()
        .unwrap();
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let activation = plugins
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|activation| activation.id == "test.multi-source")
        .unwrap();
    assert_eq!(
        semantic_data::plugin::PluginActivation::from_value(&activation.to_value()).unwrap(),
        activation
    );
    let mut invalid = activation.clone();
    invalid.source_bindings.clear();
    assert!(invalid.validate().is_err());
    invalid.source_bindings = activation.source_bindings.clone();
    invalid
        .source_bindings
        .insert("importer-a".into(), "fetcher-b".into());
    assert!(invalid.validate().is_err());
    let mut legacy = activation.clone();
    legacy.exports.retain(|export| export.export != "source-b");
    legacy.source_bindings.clear();
    let semantic_data::Value::Object(mut encoded) = legacy.to_value() else {
        unreachable!()
    };
    encoded.remove("source_bindings");
    let legacy =
        semantic_data::plugin::PluginActivation::from_value(&semantic_data::Value::Object(encoded))
            .unwrap();
    for export in ["importer-a", "importer-b", "fetcher-a", "fetcher-b"] {
        assert_eq!(legacy.source_for_export(export), Some("source-a"));
    }
    let request = SourceRequest {
        url: "https://example.com/test.txt".into(),
        options: Object::new(),
    };
    for (operation, prefix) in [
        (semantic_import::Operation::ImportSource, "importer"),
        (semantic_import::Operation::Fetch, "fetcher"),
    ] {
        let candidates = context
            .import_candidates(None, &request, operation)
            .await
            .unwrap();
        let candidates: Vec<_> = candidates
            .into_iter()
            .filter(|candidate| candidate.operation.plugin_id() == "test.multi-source")
            .collect();
        assert_eq!(candidates.len(), 2);
        use semantic_rpc::interface::{
            InvocationArgument, InvocationContext, InvocationOutput, ValidatedInvocation,
        };
        let semantic_data::Value::Object(mut payload) = request.to_value() else {
            unreachable!()
        };
        payload.insert("operation", operation.as_str().to_owned());
        let api = semantic_app::interface::implementation(context.clone()).unwrap();
        let InvocationOutput::Values(values) = api
            .invoke(
                ValidatedInvocation {
                    export: "application".into(),
                    method: "list_candidates".into(),
                    arguments: vec![InvocationArgument::Value(semantic_data::Value::Object(
                        payload,
                    ))],
                },
                InvocationContext::default(),
            )
            .await
            .unwrap()
        else {
            panic!("expected candidate values")
        };
        let semantic_data::Value::List(api_candidates) = &values[0] else {
            panic!("expected candidate list")
        };
        for (suffix, source) in [("a", "source-b"), ("b", "source-a")] {
            let export = format!("{prefix}-{suffix}");
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.operation.descriptor().export == export)
                .unwrap();
            assert_eq!(candidate.source.descriptor().export, source);
            let api_candidate = api_candidates
                .iter()
                .find_map(|value| match value {
                    semantic_data::Value::Object(value)
                        if value
                            .get("plugin_id")
                            .and_then(semantic_data::Value::as_str)
                            == Some("test.multi-source")
                            && value.get("export").and_then(semantic_data::Value::as_str)
                                == Some(export.as_str()) =>
                    {
                        Some(value)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                api_candidate
                    .get("source_export")
                    .and_then(semantic_data::Value::as_str),
                Some(source)
            );
            let selected =
                semantic_import::select(&candidates, Some(("test.multi-source", &export))).unwrap();
            assert_eq!(selected.descriptor().export, export);
        }
    }
}

#[cfg(feature = "plugin-websocket")]
#[path = "../../import/tests/support/mod.rs"]
mod provider_support;

struct HoldJob(semantic_jobs::JobKindDescriptor);
impl semantic_jobs::JobHandler for HoldJob {
    type Input = tokio::sync::oneshot::Receiver<()>;
    type Output = ();
    fn kind(&self) -> &semantic_jobs::JobKindDescriptor {
        &self.0
    }
    fn run<'a>(
        &'a self,
        input: Self::Input,
        context: semantic_jobs::JobContext,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), semantic_jobs::JobError>> + Send + 'a>,
    > {
        Box::pin(async move {
            tokio::select! {
                _ = input => Ok(()),
                _ = context.cancellation().cancelled() => Err(semantic_jobs::JobError::new("cancelled", "cancelled")),
            }
        })
    }
}

fn app_session(
    implementation: Arc<dyn semantic_rpc::interface::InterfaceImplementation>,
) -> (
    semantic_rpc::interface::session::Session,
    semantic_rpc::interface::session::Session,
) {
    let (client, server, _) = app_session_with_disconnect(implementation);
    (client, server)
}

fn app_session_with_disconnect(
    implementation: Arc<dyn semantic_rpc::interface::InterfaceImplementation>,
) -> (
    semantic_rpc::interface::session::Session,
    semantic_rpc::interface::session::Session,
    semantic_jobs::CancellationToken,
) {
    use semantic_rpc::interface::session::Session;
    let disconnect = semantic_jobs::CancellationToken::new();
    let (a_tx, mut a_rx) = tokio::sync::mpsc::unbounded_channel();
    let (b_tx, mut b_rx) = tokio::sync::mpsc::unbounded_channel();
    let (a_in_tx, a_in) = tokio::sync::mpsc::unbounded_channel();
    let (b_in_tx, b_in) = tokio::sync::mpsc::unbounded_channel();
    let a_disconnect = disconnect.clone();
    tokio::spawn(async move {
        while let Some(message) = tokio::select! { _ = a_disconnect.cancelled() => None, message = a_rx.recv() => message }
        {
            if b_in_tx.send(Ok(message)).is_err() {
                break;
            }
        }
    });
    let b_disconnect = disconnect.clone();
    tokio::spawn(async move {
        while let Some(message) = tokio::select! { _ = b_disconnect.cancelled() => None, message = b_rx.recv() => message }
        {
            if a_in_tx.send(Ok(message)).is_err() {
                break;
            }
        }
    });
    (
        Session::start(a_in, a_tx, None, vec![]),
        Session::start(b_in, b_tx, Some(implementation), vec![]),
        disconnect,
    )
}

#[tokio::test]
async fn upstream_change_cancels_queued_distinct_importer_through_application_session() {
    queued_upstream_cancellation(false).await;
    queued_upstream_cancellation(true).await;
}

async fn queued_upstream_cancellation(consume_preview: bool) {
    use semantic_data::Value;
    use semantic_jobs::{JobId, JobStatus};
    use semantic_rpc::interface::{
        InterfaceImplementation, InvocationArgument, InvocationContext, InvocationOutput,
        ValidatedInvocation,
    };
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let mut registry = semantic_jobs::JobsBuilder::new();
    let hold = registry
        .register(HoldJob(semantic_jobs::JobKindDescriptor {
            id: semantic_jobs::JobKindId("test.hold".into()),
            title: "Hold queue".into(),
            description: None,
        }))
        .unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .with_default_file_store_uri(
            DbScopeId::new("main"),
            format!("fs://{}", temp.path().join("blobs").display()),
        )
        .with_jobs(
            registry.build(),
            semantic_jobs::JobsConfig {
                max_concurrent_jobs: std::num::NonZeroUsize::new(1).unwrap(),
                ..Default::default()
            },
        )
        .build()
        .unwrap();
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let mut upstream = plugins.list().await.unwrap().remove(0);
    let mut downstream = upstream.clone();
    downstream.id = "test.downstream".into();
    plugins.configure(downstream).await.unwrap();
    let jobs = context.jobs(None).await.unwrap();
    let (release, blocked) = tokio::sync::oneshot::channel();
    let blocker = jobs
        .submit(&hold, blocked, Default::default())
        .await
        .unwrap();
    let request = SourceRequest {
        url: if consume_preview {
            http(vec![b"queued after preview"]).await
        } else {
            "http://127.0.0.1:1/unpolled.txt".into()
        },
        options: Object::new(),
    };
    let identity = GenericUrlPlugin::identity(&request).unwrap();
    let (client, server) = app_session(semantic_app::interface::implementation(context).unwrap());
    let Value::Object(mut fetch_request) = request.to_value() else {
        unreachable!()
    };
    fetch_request.insert("plugin_id", upstream.id.clone());
    fetch_request.insert("export", "fetcher".to_owned());
    let InvocationOutput::Stream(mut stream) = client
        .invoke(
            ValidatedInvocation {
                export: "application".into(),
                method: "fetch_source".into(),
                arguments: vec![InvocationArgument::Value(Value::Object(fetch_request))],
            },
            InvocationContext::default(),
        )
        .await
        .unwrap()
    else {
        panic!("expected fetch stream")
    };
    if consume_preview {
        // Simulate a browser reading preview metadata before transferring the
        // remaining stream; authoritative origin groups must survive both paths.
        assert!(stream.next().await.unwrap().is_ok());
    }
    let fetched_request = semantic_import::FetchedRequest {
        identity: identity.clone(),
        representation: "file".into(),
        options: Object::new(),
    };
    let Value::Object(mut import_request) = fetched_request.to_value() else {
        unreachable!()
    };
    import_request.insert("plugin_id", "test.downstream".to_owned());
    import_request.insert("export", "importer".to_owned());
    let InvocationOutput::Values(values) = client
        .invoke(
            ValidatedInvocation {
                export: "application".into(),
                method: "start_import_fetched".into(),
                arguments: vec![
                    InvocationArgument::Value(Value::Object(import_request)),
                    InvocationArgument::Stream(stream),
                ],
            },
            InvocationContext::default(),
        )
        .await
        .unwrap()
    else {
        panic!("expected job id")
    };
    let Value::Object(result) = &values[0] else {
        panic!("expected object")
    };
    let Some(Value::String(id)) = result.get("id") else {
        panic!("expected id")
    };
    let id = JobId(id.clone());
    assert_eq!(
        jobs.get(id.clone()).await.unwrap().unwrap().status,
        JobStatus::Queued
    );
    upstream.enabled = false;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        plugins.configure(upstream),
    )
    .await
    .unwrap()
    .unwrap();
    let record = jobs.get(id).await.unwrap().unwrap();
    assert_eq!(record.status, JobStatus::Cancelled);
    assert_eq!(record.error.unwrap().code, "plugin_changed");
    assert!(
        plugins
            .runtime
            .bindings()
            .await
            .iter()
            .any(|b| b.plugin_id() == "test.downstream")
    );
    let entity_id = uuid::Uuid::from(semantic_import::stable_entity_id(
        &identity.namespace,
        &identity.source,
        "file",
    ))
    .to_string();
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION.into(), entity_id)
            .await
            .unwrap()
            .is_none()
    );
    release.send(()).unwrap();
    blocker.wait().await.unwrap();
    client.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
    app.shutdown().await.unwrap();
}

async fn http(bodies: Vec<&'static [u8]>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        for body in bodies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        }
    });
    format!("http://{address}/file.txt")
}

#[tokio::test]
async fn native_fetch_import_replace_and_history_clear_preserve_files() {
    fetch_import_replace_and_history_clear_preserve_files(None).await;
}

#[cfg(feature = "plugin-stdio")]
#[tokio::test]
#[ignore = "build semantic_import --example url_provider_fixture first, then run this test with --ignored"]
async fn stdio_fetch_import_replace_and_history_clear_preserve_files() {
    let program = std::env::var("IMPORT_PROVIDER_FIXTURE").unwrap_or_else(|_| {
        format!(
            "{}/../../target/debug/examples/url_provider_fixture",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    assert!(
        std::path::Path::new(&program).is_file(),
        "build semantic_import --example url_provider_fixture first"
    );
    fetch_import_replace_and_history_clear_preserve_files(Some(
        semantic_data::plugin::PluginProvider::Stdio {
            program,
            args: Vec::new(),
            cwd: None,
            env: Default::default(),
        },
    ))
    .await;
}

#[cfg(feature = "plugin-websocket")]
#[tokio::test]
async fn websocket_fetch_import_replace_and_history_clear_preserve_files() {
    use futures_util::SinkExt;
    use semantic_rpc_core::interface_protocol::{InterfaceMessage, PLUGIN_SUBPROTOCOL};
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_hdr_async(socket, |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            response.headers_mut().insert("Sec-WebSocket-Protocol", PLUGIN_SUBPROTOCOL.parse().unwrap());
            Ok(response)
        }).await.unwrap();
        let (mut sink, mut source) = socket.split();
        let (outgoing, mut output) = tokio::sync::mpsc::unbounded_channel::<InterfaceMessage>();
        let (input, incoming) = tokio::sync::mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            while let Some(Ok(Message::Text(text))) = source.next().await {
                if input
                    .send(Ok(serde_json::from_str::<InterfaceMessage>(&text).unwrap()))
                    .is_err()
                {
                    break;
                }
            }
        });
        let writer = tokio::spawn(async move {
            while let Some(message) = output.recv().await {
                if sink
                    .send(Message::Text(
                        serde_json::to_string(&message).unwrap().into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let session = semantic_rpc::plugin::accept(
            incoming,
            outgoing,
            provider_support::implementation().await,
            Some("1".into()),
            |_| async { Ok(()) },
        )
        .await
        .unwrap();
        session.closed().await;
        writer.abort();
        reader.abort();
    });
    fetch_import_replace_and_history_clear_preserve_files(Some(
        semantic_data::plugin::PluginProvider::WebSocket { url },
    ))
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), peer)
        .await
        .unwrap()
        .unwrap();
}

#[cfg(feature = "plugin-websocket")]
#[tokio::test]
async fn idle_websocket_disconnect_marks_plugin_unavailable_and_cancels_binding() {
    use futures_util::SinkExt;
    use semantic_rpc_core::interface_protocol::{InterfaceMessage, PLUGIN_SUBPROTOCOL};
    use tokio_tungstenite::tungstenite::Message;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (disconnect, disconnected) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_hdr_async(socket, |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            response.headers_mut().insert("Sec-WebSocket-Protocol", PLUGIN_SUBPROTOCOL.parse().unwrap());
            Ok(response)
        }).await.unwrap();
        let (mut sink, mut source) = socket.split();
        let (outgoing, mut output) = tokio::sync::mpsc::unbounded_channel::<InterfaceMessage>();
        let (input, incoming) = tokio::sync::mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            while let Some(Ok(Message::Text(text))) = source.next().await {
                if input
                    .send(Ok(serde_json::from_str::<InterfaceMessage>(&text).unwrap()))
                    .is_err()
                {
                    break;
                }
            }
        });
        let writer = tokio::spawn(async move {
            while let Some(message) = output.recv().await {
                if sink
                    .send(Message::Text(
                        serde_json::to_string(&message).unwrap().into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let session = semantic_rpc::plugin::accept(
            incoming,
            outgoing,
            provider_support::implementation().await,
            Some("1".into()),
            |_| async { Ok(()) },
        )
        .await
        .unwrap();
        disconnected.await.unwrap();
        reader.abort();
        writer.abort();
        let _ = reader.await;
        let _ = writer.await;
        session.shutdown().await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let mut registry = semantic_jobs::JobsBuilder::new();
    let hold = registry
        .register(HoldJob(semantic_jobs::JobKindDescriptor {
            id: semantic_jobs::JobKindId("test.idle-disconnect".into()),
            title: "Wait for disconnect".into(),
            description: None,
        }))
        .unwrap();
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db)
        .with_jobs(registry.build(), Default::default())
        .build()
        .unwrap();
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let mut activation = plugins.list().await.unwrap().remove(0);
    activation.provider = semantic_data::plugin::PluginProvider::WebSocket { url };
    let plugin_id = activation.id.clone();
    plugins.configure(activation).await.unwrap();
    let binding = plugins
        .runtime
        .bindings()
        .await
        .into_iter()
        .find(|binding| binding.plugin_id() == plugin_id)
        .unwrap();
    assert!(!binding.cancellation().is_cancelled());
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    let (_release, blocked) = tokio::sync::oneshot::channel();
    let job = context
        .jobs(None)
        .await
        .unwrap()
        .submit(
            &hold,
            blocked,
            semantic_jobs::SubmitOptions {
                groups: vec![binding.group()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // No invocation occurs: the transport monitor must notice the idle disconnect.
    disconnect.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if plugins
                .runtime
                .states()
                .await
                .into_iter()
                .any(|(id, state, _)| {
                    id == plugin_id && state == semantic_plugin::PluginState::Unavailable
                })
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        binding.cancellation().cancelled().await;
        assert!(matches!(
            job.wait().await,
            Err(semantic_jobs::JobCompletionError::Cancelled(_))
        ));
    })
    .await
    .unwrap();
    assert!(plugins.runtime.bindings().await.is_empty());
    peer.await.unwrap();
    app.shutdown().await.unwrap();
}

async fn fetch_import_replace_and_history_clear_preserve_files(
    provider: Option<semantic_data::plugin::PluginProvider>,
) {
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .with_default_file_store_uri(
            DbScopeId::new("main"),
            format!("fs://{}", temp.path().join("blobs").display()),
        )
        .build()
        .unwrap();
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    if let Some(provider) = provider {
        let plugins = app
            .plugins(&Principal::system(), DbScopeId::new("main"))
            .await
            .unwrap();
        let mut activation = plugins.list().await.unwrap().remove(0);
        activation.provider = provider;
        plugins.configure(activation).await.unwrap();
    }
    let request = SourceRequest {
        url: http(vec![b"preview", b"first", b"second"]).await,
        options: Object::new(),
    };
    let identity = GenericUrlPlugin::identity(&request).unwrap();
    let id = uuid::Uuid::from(semantic_import::stable_entity_id(
        &identity.namespace,
        &identity.source,
        "file",
    ))
    .to_string();
    let fetched = context
        .fetch_source(None, request.clone(), None)
        .await
        .unwrap();
    let mut stream = semantic_import::decode_stream(fetched.stream);
    let mut bytes = Vec::new();
    while let Some(frame) = stream.next().await {
        if let ContentFrame::Item(semantic_import::ContentEvent::FileBytes { bytes: chunk }) =
            frame.unwrap()
        {
            bytes.extend_from_slice(&chunk);
        }
    }
    assert_eq!(bytes, b"preview");
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION.into(), id.clone())
            .await
            .unwrap()
            .is_none()
    );
    let first = context
        .start_import_source(None, request.clone(), None)
        .await
        .unwrap();
    assert_eq!(first.wait().await.unwrap().items, 1);
    let original = db
        .get(semantic_db_core::DEFAULT_COLLECTION.into(), id.clone())
        .await
        .unwrap()
        .unwrap();
    let second = context
        .start_import_source(None, request, None)
        .await
        .unwrap();
    second.wait().await.unwrap();
    let replacement = db
        .get(semantic_db_core::DEFAULT_COLLECTION.into(), id.clone())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(original.object, replacement.object);
    context
        .jobs(None)
        .await
        .unwrap()
        .clear_completed()
        .await
        .unwrap();
    let file = app.files().read(&context, None, id).await.unwrap();
    let mut stream = file.stream;
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(bytes, b"second");
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn generation_change_cancels_idle_fetch_and_rejects_stale_binding() {
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db)
        .build()
        .unwrap();
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    let request = SourceRequest {
        url: "http://127.0.0.1:1/no-request.txt".into(),
        options: Object::new(),
    };
    let candidates = context
        .import_candidates(None, &request, semantic_import::Operation::Fetch)
        .await
        .unwrap();
    let stale = semantic_import::select(&candidates, None).unwrap();
    let mut fetched = context.fetch_source(None, request, None).await.unwrap();
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let mut activation = plugins.list().await.unwrap().remove(0);
    activation.enabled = false;
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        plugins.configure(activation),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        fetched.stream.next().await.unwrap().unwrap_err().code,
        "plugin_changed"
    );
    assert_eq!(
        stale.invoke("fetch", Vec::new()).await.err().unwrap().code,
        "plugin_changed"
    );
    assert!(plugins.runtime.bindings().await.is_empty());
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_file_replacement_preserves_published_bytes() {
    use semantic_import::ImportWriter;
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let store: objstore::DynObjStore = Arc::new(
        objstore_fs::FsObjStore::new(objstore_fs::FsObjStoreConfig::new(
            temp.path().join("blobs"),
        ))
        .unwrap(),
    );
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .with_default_file_store(DbScopeId::new("main"), store.clone())
        .build()
        .unwrap();
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    context.resolve_db(None).await.unwrap();
    let writer = semantic_app::imports::AppImportWriter::new(db, store, app.files().clone());
    let identity = semantic_import::SourceIdentity {
        namespace: "test".into(),
        source: "source".into(),
    };
    let mut metadata = semantic_import::FileMetadata {
        key: "file".into(),
        filename: None,
        mime_type: "text/plain".into(),
        expected_size: None,
        attributes: Object::new(),
    };
    let mut file = writer.begin_file(&identity, &metadata).await.unwrap();
    file.write_chunk(bytes::Bytes::from_static(b"old"))
        .await
        .unwrap();
    file.finish().await.unwrap();
    metadata
        .attributes
        .insert("filename", semantic_data::Value::U64(123));
    let mut file = writer.begin_file(&identity, &metadata).await.unwrap();
    file.write_chunk(bytes::Bytes::from_static(b"new"))
        .await
        .unwrap();
    assert!(file.finish().await.is_err());
    let id =
        uuid::Uuid::from(semantic_import::stable_entity_id("test", "source", "file")).to_string();
    let mut file = app.files().read(&context, None, id).await.unwrap().stream;
    assert_eq!(
        file.next().await.unwrap().unwrap(),
        bytes::Bytes::from_static(b"old")
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn fetched_stream_transfers_to_a_job_without_refetch() {
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db)
        .with_default_file_store_uri(
            DbScopeId::new("main"),
            format!("fs://{}", temp.path().join("blobs").display()),
        )
        .build()
        .unwrap();
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    let request = SourceRequest {
        url: http(vec![b"transferred"]).await,
        options: Object::new(),
    };
    let fetched = context
        .fetch_source(None, request.clone(), None)
        .await
        .unwrap();
    let candidates = context
        .import_candidates(None, &request, semantic_import::Operation::ImportFetched)
        .await
        .unwrap();
    let binding = semantic_import::select(&candidates, None).unwrap();
    let fetched_request = semantic_import::FetchedRequest {
        identity: fetched.identity.clone(),
        representation: "file".into(),
        options: Object::new(),
    };
    let ticket = context
        .start_import_fetched(None, binding, fetched_request, fetched)
        .await
        .unwrap();
    assert_eq!(ticket.wait().await.unwrap().bytes, 11);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn disabling_plugin_cancels_running_import_before_file_publication() {
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .with_default_file_store_uri(
            DbScopeId::new("main"),
            format!("fs://{}", temp.path().join("blobs").display()),
        )
        .build()
        .unwrap();
    let context = AppRequestContext {
        app: app.clone(),
        principal: Principal::system(),
        session: None,
        request_scope: None,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (started, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        let _ = socket.read(&mut request).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 100\r\n\r\npartial").await.unwrap();
        let _ = started.send(());
        let _ = socket.read(&mut request).await;
    });
    let request = SourceRequest {
        url: format!("http://{address}/file.txt"),
        options: Object::new(),
    };
    let identity = GenericUrlPlugin::identity(&request).unwrap();
    let id = uuid::Uuid::from(semantic_import::stable_entity_id(
        &identity.namespace,
        &identity.source,
        "file",
    ))
    .to_string();
    let ticket = context
        .start_import_source(None, request, None)
        .await
        .unwrap();
    received.await.unwrap();
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    let mut activation = plugins.list().await.unwrap().remove(0);
    activation.enabled = false;
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        plugins.configure(activation),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(
        ticket.wait().await,
        Err(semantic_jobs::JobCompletionError::Cancelled(_))
    ));
    assert!(
        db.get(semantic_db_core::DEFAULT_COLLECTION.into(), id)
            .await
            .unwrap()
            .is_none()
    );
    server.await.unwrap();
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn uninstall_of_registered_default_survives_runtime_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn SemanticDb> = Arc::new(semantic_db_core::Db::new(
        semantic_db_redb::open_backend(temp.path().join("db.redb"), DbOpenMode::AutoCreate)
            .unwrap(),
    ));
    let app = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db.clone())
        .build()
        .unwrap();
    let plugins = app
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    assert_eq!(plugins.list().await.unwrap().len(), 1);
    plugins.uninstall("semantic.generic-url").await.unwrap();
    assert!(plugins.list().await.unwrap().is_empty());
    app.shutdown().await.unwrap();
    let reopened = SemanticApp::builder()
        .with_default_scope(DbScopeId::new("main"), db)
        .build()
        .unwrap();
    let plugins = reopened
        .plugins(&Principal::system(), DbScopeId::new("main"))
        .await
        .unwrap();
    assert!(plugins.list().await.unwrap().is_empty());
    assert!(plugins.runtime.bindings().await.is_empty());
    reopened.shutdown().await.unwrap();
}
