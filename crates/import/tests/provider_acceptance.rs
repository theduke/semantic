//! Build the fixture first, then run with IMPORT_PROVIDER_FIXTURE set to its path.
mod support;
use futures_util::{SinkExt, StreamExt};
use semantic_data::{Object, Value, import::*};
use semantic_import::{ContentFrame, decode_stream};
use semantic_rpc::{
    interface::*,
    plugin::{self, ProviderConnection},
};
use semantic_rpc_core::interface_protocol::{InterfaceMessage, PLUGIN_SUBPROTOCOL};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use tokio_tungstenite::tungstenite::Message;

async fn http_source() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/data.txt", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut request = [0; 2048];
                socket.read(&mut request).await.unwrap();
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 262144\r\nConnection: close\r\n\r\n").await.unwrap();
                for _ in 0..64 {
                    if socket.write_all(&[b'x'; 4096]).await.is_err() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            });
        }
    });
    (url, requests, task)
}

async fn websocket_server() -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<AtomicUsize>,
    CancellationToken,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let demands = Arc::new(AtomicUsize::new(0));
    let count = demands.clone();
    let disconnect = CancellationToken::new();
    let disconnect_peer = disconnect.clone();
    let peer = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_hdr_async(tcp, |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            response.headers_mut().insert("Sec-WebSocket-Protocol", PLUGIN_SUBPROTOCOL.parse().unwrap()); Ok(response)
        }).await.unwrap();
        let (mut sink, mut source) = socket.split();
        let (outgoing, mut output) = mpsc::unbounded_channel::<InterfaceMessage>();
        let (input, incoming) = mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            while let Some(Ok(Message::Text(text))) = source.next().await {
                let message: InterfaceMessage = serde_json::from_str(&text).unwrap();
                if matches!(message, InterfaceMessage::StreamDemand { .. }) {
                    count.fetch_add(1, Ordering::SeqCst);
                }
                if input.send(Ok(message)).is_err() {
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
        let session = plugin::accept(
            incoming,
            outgoing,
            support::implementation().await,
            Some("1".into()),
            |_| async { Ok(()) },
        )
        .await
        .unwrap();
        tokio::select! { _ = session.closed() => {}, _ = disconnect_peer.cancelled() => { writer.abort(); } }
        let _ = writer.await;
        reader.abort();
    });
    (url, peer, demands, disconnect)
}

async fn invoke(
    connection: &ProviderConnection,
    method: &str,
    arguments: Vec<InvocationArgument>,
) -> OwnedValueStream {
    let InvocationOutput::Stream(stream) = connection
        .implementation
        .invoke(
            ValidatedInvocation {
                export: if method == "fetch" {
                    "fetcher"
                } else {
                    "importer"
                }
                .into(),
                method: method.into(),
                arguments,
            },
            InvocationContext::default(),
        )
        .await
        .unwrap()
    else {
        panic!("stream")
    };
    stream
}
async fn content(stream: OwnedValueStream) -> (Vec<u8>, usize) {
    let mut stream = decode_stream(stream);
    let mut bytes = vec![];
    let mut frames = 0;
    while let Some(frame) = stream.next().await {
        frames += 1;
        if let ContentFrame::Item(ContentEvent::FileBytes { bytes: chunk }) = frame.unwrap() {
            bytes.extend_from_slice(&chunk);
        }
    }
    (bytes, frames)
}
async fn parity(
    connection: &ProviderConnection,
    url: &str,
    requests: &AtomicUsize,
    transport: &str,
) {
    let request = SourceRequest {
        url: url.into(),
        options: Object::new(),
    };
    let start = Instant::now();
    let requests_before = requests.load(Ordering::SeqCst);
    let stream = invoke(
        connection,
        "fetch",
        vec![InvocationArgument::Value(request.to_value())],
    )
    .await;
    // Returning the stream does not fetch or buffer its content.
    assert_eq!(requests_before, requests.load(Ordering::SeqCst));
    let fetched = FetchedRequest {
        identity: semantic_import::GenericUrlPlugin::identity(&request).unwrap(),
        representation: "file".into(),
        options: Object::new(),
    };
    let stream = invoke(
        connection,
        "import_fetched",
        vec![
            InvocationArgument::Value(fetched.to_value()),
            InvocationArgument::Stream(stream),
        ],
    )
    .await;
    let (bytes, frames) = content(stream).await;
    assert!(
        frames > 4,
        "content must arrive incrementally across multiple byte frames"
    );
    assert_eq!(bytes, vec![b'x'; 262144]);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        requests_before + 1,
        "import_fetched must reuse the fetch"
    );
    let elapsed = start.elapsed();
    let (direct, _) = content(
        invoke(
            connection,
            "import_source",
            vec![InvocationArgument::Value(request.to_value())],
        )
        .await,
    )
    .await;
    assert_eq!(direct, bytes);
    assert_eq!(requests.load(Ordering::SeqCst), requests_before + 2);
    eprintln!(
        "baseline {transport}: {} bytes, {frames} frames, {} us fetch/forward/import",
        bytes.len(),
        elapsed.as_micros()
    );
    // Dropping a live output sends cancellation; the session remains usable.
    let mut abandoned = invoke(
        connection,
        "fetch",
        vec![InvocationArgument::Value(request.to_value())],
    )
    .await;
    abandoned.next().await.unwrap().unwrap();
    drop(abandoned);
    let (after_cancel, _) = content(
        invoke(
            connection,
            "fetch",
            vec![InvocationArgument::Value(request.to_value())],
        )
        .await,
    )
    .await;
    assert_eq!(after_cancel, bytes);
    pending_input_is_released_on_output_drop(connection, fetched).await;
}

async fn pending_input_is_released_on_output_drop(
    connection: &ProviderConnection,
    request: FetchedRequest,
) {
    struct Released(Arc<tokio::sync::Notify>);
    impl Drop for Released {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let released = Arc::new(tokio::sync::Notify::new());
    let polled = Arc::new(tokio::sync::Notify::new());
    let guard = Released(released.clone());
    let entered = polled.clone();
    let input = OwnedValueStream::new(futures_util::stream::poll_fn(move |_| {
        let _ = &guard;
        entered.notify_one();
        std::task::Poll::Pending
    }));
    let mut output = invoke(
        connection,
        "import_fetched",
        vec![
            InvocationArgument::Value(request.to_value()),
            InvocationArgument::Stream(input),
        ],
    )
    .await;
    let consuming = tokio::spawn(async move { output.next().await });
    tokio::time::timeout(std::time::Duration::from_secs(5), polled.notified())
        .await
        .unwrap();
    consuming.abort();
    let _ = consuming.await;
    tokio::time::timeout(std::time::Duration::from_secs(5), released.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn websocket_fetch_import_parity_and_reconnect() {
    let (url, requests, http) = http_source().await;
    for _ in 0..2 {
        let (endpoint, peer, demands, disconnect) = websocket_server().await;
        let connection = plugin::websocket::connect(
            &endpoint,
            support::exports(),
            Some("1".into()),
            Value::Null,
        )
        .await
        .unwrap();
        let mut paused = decode_stream(
            invoke(
                &connection,
                "fetch",
                vec![InvocationArgument::Value(
                    SourceRequest {
                        url: url.clone(),
                        options: Object::new(),
                    }
                    .to_value(),
                )],
            )
            .await,
        );
        assert_eq!(demands.load(Ordering::SeqCst), 0);
        paused.next().await.unwrap().unwrap();
        let ContentFrame::Item(ContentEvent::FileBytes { bytes }) =
            paused.next().await.unwrap().unwrap()
        else {
            panic!("first bytes")
        };
        let before_pause = demands.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(
            demands.load(Ordering::SeqCst),
            before_pause,
            "paused consumer must not advance the remote producer"
        );
        assert_eq!(
            before_pause, 2,
            "one demand for metadata and one byte frame"
        );
        eprintln!(
            "baseline ws paused: first chunk {} bytes of 262144 total, 2 demands, 0 additional frames requested during 20 ms pause",
            bytes.len()
        );
        drop(paused);
        parity(&connection, &url, &requests, "ws").await;
        eprintln!(
            "baseline ws demand envelopes: {}",
            demands.load(Ordering::SeqCst)
        );
        disconnect.cancel();
        peer.await.unwrap();
        let output = connection
            .implementation
            .invoke(
                ValidatedInvocation {
                    export: "fetcher".into(),
                    method: "fetch".into(),
                    arguments: vec![],
                },
                InvocationContext::default(),
            )
            .await;
        assert!(output.is_err());
        connection.shutdown().await.unwrap();
    }
    http.abort();
}

#[tokio::test]
#[ignore = "build url_provider_fixture example first, then run with --ignored"]
async fn stdio_fetch_import_parity_and_child_restart() {
    let fixture = fixture_path();
    let (url, requests, http) = http_source().await;
    for _ in 0..2 {
        let connection = plugin::stdio::connect(
            plugin::stdio::StdioConfig {
                program: fixture.clone(),
                args: vec![],
                cwd: None,
                env: Default::default(),
            },
            support::exports(),
            Some("1".into()),
            Value::Null,
        )
        .await
        .unwrap();
        parity(&connection, &url, &requests, "stdio").await;
        connection.shutdown().await.unwrap();
    }
    http.abort();
}

fn fixture_path() -> String {
    let path = std::env::var("IMPORT_PROVIDER_FIXTURE").unwrap_or_else(|_| {
        format!(
            "{}/../../target/debug/examples/url_provider_fixture",
            env!("CARGO_MANIFEST_DIR")
        )
    });
    assert!(
        std::path::Path::new(&path).is_file(),
        "build fixture with cargo build -p semantic_import --example url_provider_fixture; optionally set IMPORT_PROVIDER_FIXTURE"
    );
    path
}

struct EmptyStore;
#[async_trait::async_trait]
impl semantic_jobs::JobStore for EmptyStore {
    async fn initialize(&self) -> Result<(), semantic_jobs::JobStoreError> {
        Ok(())
    }
    async fn get(
        &self,
        _: &semantic_jobs::JobId,
    ) -> Result<Option<semantic_jobs::JobRecord>, semantic_jobs::JobStoreError> {
        Ok(None)
    }
    async fn put(&self, _: &semantic_jobs::JobRecord) -> Result<(), semantic_jobs::JobStoreError> {
        Ok(())
    }
    async fn list(
        &self,
        _: semantic_jobs::JobListQuery,
    ) -> Result<semantic_jobs::JobListPage, semantic_jobs::JobStoreError> {
        Ok(semantic_jobs::JobListPage {
            records: vec![],
            next_cursor: None,
        })
    }
    async fn count(&self) -> Result<u64, semantic_jobs::JobStoreError> {
        Ok(0)
    }
    async fn delete_ids(
        &self,
        _: &[semantic_jobs::JobId],
    ) -> Result<u64, semantic_jobs::JobStoreError> {
        Ok(0)
    }
}

#[tokio::test]
#[ignore = "build url_provider_fixture example first, then run with --ignored"]
async fn child_exit_recovers_with_one_activation() {
    use semantic_plugin::{PluginActivation, PluginProvider, PluginRegistry, ScopePlugins};
    let jobs = semantic_jobs::ScopeJobs::open(
        Arc::new(EmptyStore),
        semantic_jobs::JobsBuilder::new().build(),
        Default::default(),
    )
    .await
    .unwrap();
    let runtime = ScopePlugins::new(
        "acceptance",
        PluginRegistry::new().with_host_providers(),
        jobs.clone(),
    );
    let activation = PluginActivation {
        id: "url".into(),
        revision: "1".into(),
        provider: PluginProvider::Stdio {
            program: fixture_path(),
            args: vec![],
            cwd: None,
            env: Default::default(),
        },
        enabled: true,
        generation: 1,
        configuration: Value::Null,
        configuration_schema: None,
        source_bindings: Default::default(),
        priority: None,
        exports: support::exports()
            .into_iter()
            .map(semantic_plugin::portable_export)
            .collect(),
    };
    runtime.activate(activation.clone()).await.unwrap();
    let old = runtime
        .bindings()
        .await
        .into_iter()
        .find(|b| b.descriptor().export == "fetcher")
        .unwrap();
    assert!(old.invoke("exit", vec![]).await.is_err());
    let mut replacement = activation;
    replacement.generation = 2;
    runtime.activate(replacement).await.unwrap();
    let new = runtime
        .bindings()
        .await
        .into_iter()
        .find(|b| b.descriptor().export == "fetcher")
        .unwrap();
    assert_eq!(new.generation(), 2);
    let (url, _, http) = http_source().await;
    let InvocationOutput::Stream(stream) = new
        .invoke(
            "fetch",
            vec![InvocationArgument::Value(
                SourceRequest {
                    url,
                    options: Object::new(),
                }
                .to_value(),
            )],
        )
        .await
        .unwrap()
    else {
        panic!("stream")
    };
    assert_eq!(content(stream).await.0, vec![b'x'; 262144]);
    runtime.shutdown().await.unwrap();
    jobs.shutdown().await.unwrap();
    http.abort();
}

#[tokio::test]
async fn websocket_disconnect_recovers_with_one_activation() {
    use semantic_plugin::{PluginActivation, PluginProvider, PluginRegistry, ScopePlugins};
    let jobs = semantic_jobs::ScopeJobs::open(
        Arc::new(EmptyStore),
        semantic_jobs::JobsBuilder::new().build(),
        Default::default(),
    )
    .await
    .unwrap();
    let runtime = ScopePlugins::new(
        "acceptance",
        PluginRegistry::new().with_host_providers(),
        jobs.clone(),
    );
    let (endpoint, peer, _, disconnect) = websocket_server().await;
    let activation = PluginActivation {
        id: "url".into(),
        revision: "1".into(),
        provider: PluginProvider::WebSocket { url: endpoint },
        enabled: true,
        generation: 1,
        configuration: Value::Null,
        configuration_schema: None,
        source_bindings: Default::default(),
        priority: None,
        exports: support::exports()
            .into_iter()
            .map(semantic_plugin::portable_export)
            .collect(),
    };
    runtime.activate(activation.clone()).await.unwrap();
    let old = runtime
        .bindings()
        .await
        .into_iter()
        .find(|b| b.descriptor().export == "fetcher")
        .unwrap();
    disconnect.cancel();
    peer.await.unwrap();
    assert!(old.invoke("fetch", vec![]).await.is_err());
    let (endpoint, peer, _, _) = websocket_server().await;
    let mut replacement = activation;
    replacement.generation = 2;
    replacement.provider = PluginProvider::WebSocket { url: endpoint };
    runtime.activate(replacement).await.unwrap();
    let new = runtime
        .bindings()
        .await
        .into_iter()
        .find(|b| b.descriptor().export == "fetcher")
        .unwrap();
    assert_eq!(new.generation(), 2);
    let (url, _, http) = http_source().await;
    let InvocationOutput::Stream(stream) = new
        .invoke(
            "fetch",
            vec![InvocationArgument::Value(
                SourceRequest {
                    url,
                    options: Object::new(),
                }
                .to_value(),
            )],
        )
        .await
        .unwrap()
    else {
        panic!("stream")
    };
    assert_eq!(content(stream).await.0, vec![b'x'; 262144]);
    runtime.shutdown().await.unwrap();
    peer.await.unwrap();
    jobs.shutdown().await.unwrap();
    http.abort();
}
