use std::sync::{Arc, Mutex, mpsc as std_mpsc};

use bytes::Bytes;
use futures::StreamExt as _;
use futures::channel::mpsc;
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    bundles::directory::{
        ATTR_DIRECTORY_NODE_FROM, ATTR_DIRECTORY_NODE_ORDER, DIRECTORY_NODE_CLASS_ID,
        DIRECTORY_NODE_RELATION_ID,
    },
    value::{Object, Value},
};
use semantic_rpc::{
    RpcClient, RpcClientError,
    file::{FileDownloadByteStream, FileUploadContent, FileUploadRequest, FileUploadResponse},
};
use tokio::runtime::Handle;

use crate::layout::MountConfig;

const ATTR_RELATION_RELATION: &str = "semantic:relation:relation";
const ATTR_RELATION_TO: &str = "semantic:relation:to";

pub(crate) enum Event {
    UploadFinished {
        inode: u64,
        result: std::result::Result<FileUploadResponse, String>,
    },
}

#[derive(Clone)]
pub(crate) struct RuntimeBridge {
    client: RpcClient,
    runtime: Handle,
    event_tx: std_mpsc::Sender<Event>,
    events: Arc<Mutex<std_mpsc::Receiver<Event>>>,
}

pub(crate) struct UploadSession {
    pub sender: Option<mpsc::Sender<std::result::Result<Bytes, RpcClientError>>>,
    pub completion: std_mpsc::Receiver<std::result::Result<FileUploadResponse, String>>,
}

impl RuntimeBridge {
    pub fn new(client: RpcClient, runtime: Handle) -> Self {
        let (event_tx, event_rx) = std_mpsc::channel();
        Self {
            client,
            runtime,
            event_tx,
            events: Arc::new(Mutex::new(event_rx)),
        }
    }

    pub fn invoke(
        &self,
        command: impl Into<String>,
        payload: Value,
    ) -> std::result::Result<Value, String> {
        self.runtime
            .block_on(self.client.invoke_value(command, payload))
            .map_err(|error| error.to_string())
    }

    pub fn read(
        &self,
        id: String,
        scope_id: Option<String>,
        offset: u64,
        size: u32,
    ) -> std::result::Result<Bytes, String> {
        self.runtime
            .block_on(self.client.read_file_range(id, scope_id, offset, size))
            .map_err(|error| error.to_string())
    }

    pub fn stream_file_from(
        &self,
        id: String,
        scope_id: Option<String>,
        offset: u64,
    ) -> std::result::Result<FileDownloadByteStream, String> {
        self.runtime
            .block_on(self.client.stream_file_from(id, scope_id, offset))
            .map_err(|error| error.to_string())
    }

    pub fn next_download_chunk(
        &self,
        stream: &mut FileDownloadByteStream,
    ) -> std::result::Result<Option<Bytes>, String> {
        self.runtime
            .block_on(stream.next())
            .transpose()
            .map_err(|error| error.to_string())
    }

    pub fn begin_upload(
        &self,
        inode: u64,
        filename: String,
        parent: Option<String>,
        size: Option<u64>,
        scope_id: Option<String>,
    ) -> UploadSession {
        let (sender, receiver) = mpsc::channel(8);
        let (completion_tx, completion_rx) = std_mpsc::channel();
        let client = self.client.clone();
        let event_tx = self.event_tx.clone();
        self.runtime.spawn(async move {
            let request = FileUploadRequest {
                scope_id: scope_id.clone(),
                id: None,
                filename: Some(filename),
                mime_type: None,
                entity: Object::new(),
                content: FileUploadContent::Stream {
                    stream: Box::pin(receiver),
                    size,
                },
            };
            let result = client
                .upload_file(request, None)
                .await
                .map_err(|error| error.to_string());
            let result = match result {
                Ok(response) => {
                    if let Some(parent) = parent {
                        match link_entity(&client, scope_id, &parent, &response.id).await {
                            Ok(()) => Ok(response),
                            Err(error) => Err(format!(
                                "file uploaded as '{}' but linking it into directory '{}' failed: {error}",
                                response.id, parent
                            )),
                        }
                    } else {
                        Ok(response)
                    }
                }
                Err(error) => Err(error),
            };
            let _ = completion_tx.send(result.clone());
            let _ = event_tx.send(Event::UploadFinished { inode, result });
        });
        UploadSession {
            sender: Some(sender),
            completion: completion_rx,
        }
    }

    pub fn drain_events(&self) -> Vec<Event> {
        let receiver = self.events.lock().expect("FUSE event receiver poisoned");
        std::iter::from_fn(|| receiver.try_recv().ok()).collect()
    }
}

pub(crate) fn entity_payload(config: &MountConfig, id: &str, object: Object) -> Value {
    let mut payload = Object::new();
    if let Some(scope_id) = &config.scope_id {
        payload.insert("scope_id", Value::String(scope_id.clone()));
    }
    payload.insert("collection", Value::String(config.collection.clone()));
    payload.insert("id", Value::String(id.to_string()));
    payload.insert("object", Value::Object(object));
    Value::Object(payload)
}

pub(crate) fn batch_payload(config: &MountConfig, operations: Vec<Value>) -> Value {
    let mut payload = Object::new();
    if let Some(scope_id) = &config.scope_id {
        payload.insert("scope_id", Value::String(scope_id.clone()));
    }
    payload.insert("operations", Value::List(operations));
    Value::Object(payload)
}

pub(crate) fn link_operation(parent: &str, child: &str) -> Value {
    let id = directory_node_id(parent, child);
    let mut object = Object::new();
    object.insert("id", Value::String(id.clone()));
    object.insert("type", Value::String(DIRECTORY_NODE_CLASS_ID.to_string()));
    object.insert(
        ATTR_RELATION_RELATION,
        Value::String(DIRECTORY_NODE_RELATION_ID.to_string()),
    );
    object.insert(ATTR_DIRECTORY_NODE_FROM, Value::String(parent.to_string()));
    object.insert(ATTR_RELATION_TO, Value::String(child.to_string()));
    object.insert(ATTR_DIRECTORY_NODE_ORDER, Value::U64(0));
    let mut operation = Object::new();
    operation.insert("kind", Value::String("upsert".to_string()));
    operation.insert("collection", Value::String(DEFAULT_COLLECTION.to_string()));
    operation.insert("id", Value::String(id));
    operation.insert("object", Value::Object(object));
    Value::Object(operation)
}

pub(crate) fn unlink_operation(parent: &str, child: &str) -> Value {
    let mut operation = Object::new();
    operation.insert("kind", Value::String("delete_by_id".to_string()));
    operation.insert("collection", Value::String(DEFAULT_COLLECTION.to_string()));
    operation.insert("id", Value::String(directory_node_id(parent, child)));
    Value::Object(operation)
}

async fn link_entity(
    client: &RpcClient,
    scope_id: Option<String>,
    parent: &str,
    child: &str,
) -> std::result::Result<(), String> {
    let config = MountConfig {
        scope_id,
        ..MountConfig::default()
    };
    client
        .invoke_value(
            "semantic.db.batch",
            batch_payload(&config, vec![link_operation(parent, child)]),
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn directory_node_id(parent: &str, child: &str) -> String {
    format!(
        "semantic:directory_node:{}:{}",
        hex::encode(parent.as_bytes()),
        hex::encode(child.as_bytes())
    )
}
