use super::{session::Session, *};
use futures::lock::Mutex;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct InterfaceClient {
    endpoint: String,
    session: Arc<Mutex<Option<Session>>>,
}

impl InterfaceClient {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.into(),
            session: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn invoke(
        &self,
        call: ValidatedInvocation,
    ) -> Result<InvocationOutput, InvocationError> {
        let session = {
            let mut session = self.session.lock().await;
            if session.is_none() {
                *session = Some(connect(&self.endpoint).await?);
            }
            session.as_ref().unwrap().clone()
        };
        // A closed cached session fails. A fresh client is an explicit new connection;
        // operations are never replayed or silently reconnected.
        session.invoke(call, InvocationContext::default()).await
    }
}

fn exports() -> Result<Vec<ImplementationDescriptor>, InvocationError> {
    let package = semantic_data::import::package();
    let interface = &package.root.interfaces["Application"];
    let fingerprint = semantic_data::schema::interface_fingerprint(interface, &BTreeMap::new())
        .map_err(|error| InvocationError::new("interface_incompatible", error))?;
    Ok(vec![ImplementationDescriptor {
        export: "application".into(),
        interface: InterfaceRef {
            package: package.name,
            module: package.root.name,
            contract: None,
            name: "Application".into(),
        },
        package_version: "1.0.0".into(),
        fingerprint,
    }])
}

fn endpoint(url: &str) -> Result<String, InvocationError> {
    let (url, query) = url
        .split_once('?')
        .map_or((url, None), |(url, query)| (url, Some(query)));
    let (scheme, authority_path) = url.split_once("://").ok_or_else(|| {
        InvocationError::new(
            "invalid_argument",
            "RPC URL must be absolute for interface discovery",
        )
    })?;
    let scheme = match scheme {
        "http" => "ws",
        "https" => "wss",
        _ => {
            return Err(InvocationError::new(
                "invalid_argument",
                "expected HTTP RPC URL",
            ));
        }
    };
    let (authority, path) = authority_path
        .find('/')
        .map_or((authority_path, ""), |index| authority_path.split_at(index));
    let path = semantic_rpc_core::interface_protocol::interface_ws_path(path);
    let endpoint = format!("{scheme}://{authority}{path}");
    Ok(match query {
        Some(query) => format!("{endpoint}?{query}"),
        None => endpoint,
    })
}

/// Match browser HTTP requests, including root-, document-, and scheme-relative URLs.
#[cfg(any(target_arch = "wasm32", test))]
fn browser_endpoint(rpc_url: &str, document_url: &str) -> Result<String, InvocationError> {
    let invalid_url = |error: url::ParseError| {
        InvocationError::new("invalid_argument", format!("invalid RPC URL: {error}"))
    };
    let base = url::Url::parse(document_url).map_err(invalid_url)?;
    let mut resolved = base.join(rpc_url).map_err(invalid_url)?;
    // Fragments are never part of an HTTP request and WebSockets reject them.
    resolved.set_fragment(None);
    endpoint(resolved.as_str())
}

#[cfg(not(target_arch = "wasm32"))]
async fn connect(url: &str) -> Result<Session, InvocationError> {
    crate::plugin::websocket::connect_protocol(
        &endpoint(url)?,
        exports()?,
        None,
        Value::Null,
        "semantic.interface.v1",
    )
    .await
    .map(|connection| connection.into_session())
}

#[cfg(target_arch = "wasm32")]
async fn connect(url: &str) -> Result<Session, InvocationError> {
    use futures::{Sink, SinkExt, StreamExt};
    use gloo_net::websocket::{Message, futures::WebSocket};
    use semantic_rpc_core::interface_protocol::InterfaceMessage;
    use tokio::sync::mpsc;
    let failure = |error: String| InvocationError::new("connection_lost", error);
    let document_url = web_sys::window()
        .ok_or_else(|| failure("missing window".into()))?
        .location()
        .href()
        .map_err(|_| failure("missing document URL".into()))?;
    let endpoint = browser_endpoint(url, &document_url)?;
    let mut socket = WebSocket::open_with_protocol(&endpoint, "semantic.interface.v1")
        .map_err(|error| failure(error.to_string()))?;
    futures::future::poll_fn(|cx| Pin::new(&mut socket).poll_ready(cx))
        .await
        .map_err(|error| failure(error.to_string()))?;
    if socket.protocol() != "semantic.interface.v1" {
        return Err(InvocationError::new(
            "interface_incompatible",
            "WebSocket subprotocol was not negotiated",
        ));
    }
    let (mut sink, mut source) = socket.split();
    let (outgoing, mut output) = mpsc::unbounded_channel::<InterfaceMessage>();
    let (input, mut incoming) = mpsc::unbounded_channel();
    let writer_errors = input.clone();
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(message) = output.recv().await {
            let text = match serde_json::to_string(&message) {
                Ok(text) => text,
                Err(error) => {
                    let _ = writer_errors.send(Err(failure(error.to_string())));
                    break;
                }
            };
            if let Err(error) = sink.send(Message::Text(text)).await {
                let _ = writer_errors.send(Err(failure(error.to_string())));
                break;
            }
        }
        let _ = sink.close().await;
    });
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(message) = source.next().await {
            let result = match message {
                Ok(Message::Text(text)) => {
                    serde_json::from_str(&text).map_err(|error| failure(error.to_string()))
                }
                Ok(_) => Err(failure("unexpected binary WebSocket message".into())),
                Err(error) => Err(failure(error.to_string())),
            };
            let terminal = result.is_err();
            if input.send(result).is_err() || terminal {
                return;
            }
        }
        let _ = input.send(Err(failure("WebSocket disconnected".into())));
    });
    let exports = exports()?;
    crate::plugin::negotiate(&mut incoming, &outgoing, &exports, None, Value::Null).await?;
    Ok(Session::start(incoming, outgoing, None, exports))
}

#[cfg(test)]
mod tests {
    use super::{browser_endpoint, endpoint};

    #[test]
    fn browser_rpc_urls_resolve_against_the_document() {
        for (rpc, document, expected) in [
            (
                "/api/v1/rpc",
                "http://localhost:8080/import",
                "ws://localhost:8080/api/v1/interface/ws",
            ),
            (
                "/api/v1/rpc?scope=test",
                "https://example.org/import",
                "wss://example.org/api/v1/interface/ws?scope=test",
            ),
            (
                "../rpc",
                "https://example.org/app/import/",
                "wss://example.org/app/interface/ws",
            ),
            (
                "//api.example.org/custom?scope=test",
                "https://example.org/import",
                "wss://api.example.org/custom/interface/ws?scope=test",
            ),
            (
                "http://api.example.org:8081/custom/rpc?scope=test#ignored",
                "https://example.org/import",
                "ws://api.example.org:8081/custom/interface/ws?scope=test",
            ),
        ] {
            assert_eq!(browser_endpoint(rpc, document).unwrap(), expected);
        }
    }

    #[test]
    fn native_rpc_urls_remain_absolute() {
        assert!(endpoint("/api/v1/rpc").is_err());
        assert_eq!(
            endpoint("http://localhost:8080/custom?scope=test").unwrap(),
            "ws://localhost:8080/custom/interface/ws?scope=test"
        );
        assert_eq!(
            endpoint("https://example.org/custom/rpc").unwrap(),
            "wss://example.org/custom/interface/ws"
        );
    }

    #[test]
    fn browser_rejects_non_http_rpc_urls() {
        assert!(browser_endpoint("file:///rpc", "https://example.org").is_err());
        assert!(browser_endpoint("/rpc", "not a URL").is_err());
    }
}
