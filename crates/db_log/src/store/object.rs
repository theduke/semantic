use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use bytes::Bytes;
use objstore::{Conditions, ObjStore, Put};
use semantic_db_core::DbError;

use super::{DEFAULT_PREFIX, LogStore, event_key, normalize_prefix, parse_event_key};
use crate::EventId;

enum Command {
    List {
        prefix: String,
        response: mpsc::Sender<std::result::Result<Vec<String>, String>>,
    },
    Read {
        key: String,
        response: mpsc::Sender<std::result::Result<Option<Vec<u8>>, String>>,
    },
    Append {
        key: String,
        bytes: Vec<u8>,
        response: mpsc::Sender<std::result::Result<(), String>>,
    },
    Shutdown,
}

/// ObjStore-backed WAL storage.
///
/// The store must provide strongly consistent list/read behavior and atomic
/// conditional object creation. A dedicated Tokio worker bridges its async API
/// to the synchronous embedded database engine without nesting runtimes.
/// `objstore_fs` 0.1.0-alpha.3, used by this crate's tests, does not provide
/// those durability and conditional-write guarantees and is not suitable for
/// a production WAL.
pub struct ObjStoreLogStore {
    commands: mpsc::SyncSender<Command>,
    worker: Option<JoinHandle<()>>,
    prefix: String,
}

impl std::fmt::Debug for ObjStoreLogStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjStoreLogStore")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl ObjStoreLogStore {
    pub fn new(store: Arc<dyn ObjStore>) -> std::result::Result<Self, DbError> {
        Self::with_prefix(store, DEFAULT_PREFIX)
    }

    pub fn with_prefix(
        store: Arc<dyn ObjStore>,
        prefix: impl Into<String>,
    ) -> std::result::Result<Self, DbError> {
        let prefix = normalize_prefix(prefix)?;
        let (commands, receiver) = mpsc::sync_channel(64);
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("semantic-objstore-wal".to_string())
            .spawn(move || run_worker(store, receiver, startup_sender))
            .map_err(|err| DbError::Storage(format!("start ObjStore WAL worker: {err}")))?;
        match startup_receiver.recv() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let _ = worker.join();
                return Err(DbError::Storage(err));
            }
            Err(_) => {
                let _ = worker.join();
                return Err(DbError::Storage(
                    "ObjStore WAL worker stopped during startup".to_string(),
                ));
            }
        }
        Ok(Self {
            commands,
            worker: Some(worker),
            prefix,
        })
    }

    fn request<T>(
        &self,
        build: impl FnOnce(mpsc::Sender<std::result::Result<T, String>>) -> Command,
    ) -> std::result::Result<T, DbError> {
        let (sender, receiver) = mpsc::channel();
        self.commands
            .send(build(sender))
            .map_err(|_| DbError::Storage("ObjStore WAL worker stopped".to_string()))?;
        receiver
            .recv()
            .map_err(|_| DbError::Storage("ObjStore WAL worker dropped response".to_string()))?
            .map_err(DbError::Storage)
    }
}

impl Drop for ObjStoreLogStore {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl LogStore for ObjStoreLogStore {
    fn event_ids(&self, from: EventId) -> std::result::Result<Vec<EventId>, DbError> {
        let keys = self.request(|response| Command::List {
            prefix: self.prefix.clone(),
            response,
        })?;
        let mut ids = keys
            .iter()
            .map(|key| parse_event_key(&self.prefix, key))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.retain(|id| *id >= from);
        ids.sort_unstable();
        Ok(ids)
    }

    fn read_event(&self, id: EventId) -> std::result::Result<Option<Vec<u8>>, DbError> {
        let key = event_key(&self.prefix, id);
        self.request(|response| Command::Read { key, response })
    }

    fn append_event(&mut self, id: EventId, bytes: Vec<u8>) -> std::result::Result<(), DbError> {
        let key = event_key(&self.prefix, id);
        self.request(|response| Command::Append {
            key,
            bytes,
            response,
        })
    }
}

fn run_worker(
    store: Arc<dyn ObjStore>,
    receiver: mpsc::Receiver<Command>,
    startup: mpsc::SyncSender<std::result::Result<(), String>>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = startup.send(Err(format!("start ObjStore WAL runtime: {err}")));
            return;
        }
    };
    if startup.send(Ok(())).is_err() {
        return;
    }
    while let Ok(command) = receiver.recv() {
        match command {
            Command::List { prefix, response } => {
                let result = runtime
                    .block_on(store.list_all_keys(&prefix))
                    .map_err(|err| format!("list ObjStore WAL prefix '{prefix}': {err}"));
                let _ = response.send(result);
            }
            Command::Read { key, response } => {
                let result = runtime
                    .block_on(store.get(&key))
                    .map(|value| value.map(|bytes| bytes.to_vec()))
                    .map_err(|err| format!("read ObjStore WAL key '{key}': {err}"));
                let _ = response.send(result);
            }
            Command::Append {
                key,
                bytes,
                response,
            } => {
                let result = runtime.block_on(async {
                    if store
                        .get(&key)
                        .await
                        .map_err(|err| format!("check ObjStore WAL key '{key}': {err}"))?
                        .is_some()
                    {
                        return Err(format!("refusing to overwrite ObjStore WAL key '{key}'"));
                    }
                    let mut put = Put::new(key.clone(), Bytes::from(bytes));
                    put.conditions = Conditions::new().if_not_exists();
                    store
                        .send_put(put)
                        .await
                        .map(|_| ())
                        .map_err(|err| format!("append ObjStore WAL key '{key}': {err}"))
                });
                let _ = response.send(result);
            }
            Command::Shutdown => break,
        }
    }
}
