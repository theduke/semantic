//! Explicit, offline maintenance for opt-in, exclusively managed filesystem stores.
//!
//! Stop every writer to the registered databases before opening maintenance, and
//! keep them stopped until it is dropped. The store lease excludes managed App
//! instances (including other processes); it cannot lock independently held DB
//! handles or software accessing the filesystem directly. The registry is the
//! administrator's complete inventory, including retired scopes. Never adopt a
//! shared/existing store or omit an offline database to enable collection.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use objstore::{DynObjStore, ObjStore};
use semantic_data::builtin::{ATTR_ID, ATTR_TYPE};
use semantic_data::filestore::{ATTR_FILE_FILESTORE_LOCATOR, CLEANUP_CLASS_ID};
use semantic_data::query::SelectQuery;
use semantic_data::value::{DateTime, Object, Value};
use semantic_db_core::{Batch, BatchOperation, QueryResult};

use crate::{AppError, SemanticDb};

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Registry {
    version: u32,
    owner: String,
    databases: BTreeSet<String>,
    retention_seconds: u64,
}

/// A physical store with a persistent owner/inventory and an exclusive OS lease.
/// Attach `into_store()` using the existing App builder's default-store method.
/// All scopes using the store must belong to the persisted database inventory.
#[derive(Debug)]
pub struct ManagedFileStore {
    root: PathBuf,
    registry: Registry,
    _lease: File,
    store: objstore_fs::FsObjStore,
}

impl ManagedFileStore {
    /// Initialize a new empty dedicated directory. Retention must be explicit
    /// and positive. Inventory identities should be stable database URIs/paths.
    pub fn initialize(
        root: &Path,
        owner: String,
        databases: BTreeSet<String>,
        retention_seconds: u64,
    ) -> Result<Self, AppError> {
        if owner.is_empty()
            || databases.is_empty()
            || databases.iter().any(String::is_empty)
            || retention_seconds == 0
            || retention_seconds > i64::MAX as u64
        {
            return Err(invalid(
                "owner, complete database inventory and positive retention are required",
            ));
        }
        std::fs::create_dir_all(root).map_err(io_error)?;
        let root = root.canonicalize().map_err(io_error)?;
        let lease = lock(&root)?;
        for entry in std::fs::read_dir(&root).map_err(io_error)? {
            if entry.map_err(io_error)?.file_name() != ".semantic-lease" {
                return Err(invalid(
                    "managed stores must be initialized in an empty directory",
                ));
            }
        }
        let registry = Registry {
            version: 1,
            owner,
            databases,
            retention_seconds,
        };
        let bytes = serde_json::to_vec(&registry).map_err(|e| invalid(&e.to_string()))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(".semantic-registry.pending"))
            .map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        std::fs::rename(
            root.join(".semantic-registry.pending"),
            root.join(".semantic-registry.json"),
        )
        .map_err(io_error)?;
        File::open(&root)
            .and_then(|file| file.sync_all())
            .map_err(io_error)?;
        Self::from_parts(root, registry, lease)
    }

    pub fn open(root: &Path, owner: &str) -> Result<Self, AppError> {
        let root = root.canonicalize().map_err(io_error)?;
        let lease = lock(&root)?;
        let registry: Registry = serde_json::from_slice(
            &std::fs::read(root.join(".semantic-registry.json")).map_err(io_error)?,
        )
        .map_err(|e| invalid(&e.to_string()))?;
        if registry.version != 1
            || registry.owner != owner
            || registry.databases.is_empty()
            || registry.retention_seconds == 0
            || registry.retention_seconds > i64::MAX as u64
        {
            return Err(invalid("store ownership/registry does not match"));
        }
        Self::from_parts(root, registry, lease)
    }

    fn from_parts(root: PathBuf, registry: Registry, lease: File) -> Result<Self, AppError> {
        let store =
            objstore_fs::FsObjStore::new(objstore_fs::FsObjStoreConfig::new(root.join("objects")))?;
        Ok(Self {
            root,
            registry,
            _lease: lease,
            store,
        })
    }

    pub fn database_ids(&self) -> &BTreeSet<String> {
        &self.registry.databases
    }

    /// The returned handle owns the lease, including when cloned into multiple scopes.
    pub fn into_store(self) -> DynObjStore {
        Arc::new(self)
    }
}

fn lock(root: &Path) -> Result<File, AppError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".semantic-lease"))
        .map_err(io_error)?;
    file.try_lock()
        .map_err(|e| invalid(&format!("exclusive store lease unavailable: {e}")))?;
    Ok(file)
}

fn invalid(message: &str) -> AppError {
    AppError::InvalidRequest(format!("file maintenance: {message}"))
}
fn io_error(error: std::io::Error) -> AppError {
    invalid(&error.to_string())
}

/// Prevent ordinary filesystem App configuration from bypassing a managed
/// directory's lifetime lease. External direct filesystem access remains an
/// administrator responsibility under the exclusive-ownership contract.
pub(crate) fn validate_store_access(store: &dyn ObjStore) -> Result<(), AppError> {
    if store.kind() != objstore_fs::FsObjStore::KIND {
        return Ok(());
    }
    if let Ok(path) = store.safe_uri().to_file_path() {
        let path = path.canonicalize().map_err(io_error)?;
        if path.join(".semantic-registry.json").exists()
            || path
                .parent()
                .is_some_and(|parent| parent.join(".semantic-registry.json").exists())
        {
            return Err(invalid(
                "managed stores must be opened through ManagedFileStore",
            ));
        }
    }
    Ok(())
}

/// Bounded work on cleanup intents and optionally orphan bytes. Reference scans
/// cover the complete inventory before any deletion, and are intentionally full.
#[derive(Clone, Copy, Debug)]
pub struct MaintenanceOptions {
    pub dry_run: bool,
    pub sweep_orphans: bool,
    pub max_entries: usize,
}

impl Default for MaintenanceOptions {
    fn default() -> Self {
        Self {
            dry_run: true,
            sweep_orphans: false,
            max_entries: 100,
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct MaintenanceReport {
    pub eligible: usize,
    pub deleted: usize,
    pub retained: usize,
    pub failed: usize,
    pub orphan_intents: usize,
}

pub struct FileMaintenance {
    store: ManagedFileStore,
    databases: BTreeMap<String, Arc<dyn SemanticDb>>,
    running: tokio::sync::Mutex<()>,
}

impl FileMaintenance {
    /// Open only after stopping all database writers. Missing, extra, inaccessible
    /// or mismatched inventory entries fail closed; no partial-store collection.
    pub fn open(
        root: &Path,
        owner: &str,
        databases: BTreeMap<String, Arc<dyn SemanticDb>>,
    ) -> Result<Self, AppError> {
        let store = ManagedFileStore::open(root, owner)?;
        if databases.keys().cloned().collect::<BTreeSet<_>>() != store.registry.databases {
            return Err(invalid(
                "maintenance requires exactly the persisted database inventory",
            ));
        }
        Ok(Self {
            store,
            databases,
            running: tokio::sync::Mutex::new(()),
        })
    }

    pub async fn run(&self, options: MaintenanceOptions) -> Result<MaintenanceReport, AppError> {
        self.run_at(options, time::OffsetDateTime::now_utc()).await
    }

    async fn run_at(
        &self,
        options: MaintenanceOptions,
        now: time::OffsetDateTime,
    ) -> Result<MaintenanceReport, AppError> {
        let _running = self.running.lock().await;
        let mut report = MaintenanceReport::default();
        if options.max_entries == 0 {
            return Ok(report);
        }
        let mut live = BTreeSet::new();
        let mut cleanup = Vec::new();
        // Scan all collections, including internal ones and file subclasses. Any
        // stored locator protects bytes, even malformed/legacy file metadata.
        for db in self.databases.values() {
            let catalog = db.catalog().await?;
            for (_, collection) in catalog.collections() {
                let query = SelectQuery::new()
                    .with_collection(&collection.name)
                    .with_field_format(semantic_data::query::FieldFormat::Qualified);
                let QueryResult::Select(rows) = db.query_data(query.into()).await? else {
                    return Err(invalid("reference inventory returned a non-select result"));
                };
                for object in rows {
                    if let Some(locator) =
                        value(&object, ATTR_FILE_FILESTORE_LOCATOR, "filestore_locator")
                    {
                        let Some(locator) = locator.as_str() else {
                            return Err(invalid("unreadable live locator"));
                        };
                        live.insert(locator.to_owned());
                    }
                    if value(&object, ATTR_TYPE, "type").and_then(Value::as_str)
                        == Some(CLEANUP_CLASS_ID)
                    {
                        cleanup.push((db.clone(), collection.name.clone(), object));
                    }
                }
            }
        }
        let retention = time::Duration::seconds(self.store.registry.retention_seconds as i64);
        let cutoff = now
            .checked_sub(retention)
            .ok_or_else(|| invalid("retention overflow"))?;
        let mut known = BTreeSet::new();
        let mut processed = 0;
        for (db, collection, mut object) in cleanup {
            // Old scope-alias cleanup records deliberately cannot authorize deletion.
            if cleanup_string(&object, "store")? != self.store.safe_uri().as_str() {
                continue;
            }
            let locator = cleanup_string(&object, "locator")?.to_owned();
            known.insert(locator.clone());
            if processed == options.max_entries {
                continue;
            }
            let id = value(&object, ATTR_ID, "id")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("cleanup has no identity"))?
                .to_owned();
            let attempts = cleanup_value(&object, "attempts")
                .and_then(unsigned)
                .ok_or_else(|| invalid("cleanup has invalid attempts"))?;
            let Some(Value::DateTime(not_before)) = cleanup_value(&object, "not_before") else {
                return Err(invalid("cleanup has invalid retention date"));
            };
            if live.contains(&locator)
                || *not_before > DateTime::from(if attempts == 0 { cutoff } else { now })
            {
                report.retained += 1;
                continue;
            }
            // Restrict physical deletion to the immutable namespace created by
            // FileService. Never follow caller-controlled paths or symlinks.
            if !self.safe_locator(&locator)? {
                report.retained += 1;
                continue;
            }
            processed += 1;
            report.eligible += 1;
            if options.dry_run {
                continue;
            }
            let deletion = async {
                if self.store.meta(&locator).await?.is_some() {
                    self.store.delete(&locator).await?;
                }
                Ok::<(), objstore::ObjStoreError>(())
            }
            .await;
            match deletion {
                Ok(()) => {
                    // Bytes-first ordering is crash-safe: a subsequent retry sees
                    // missing bytes as success and removes the durable intent.
                    db.delete(collection, id).await?;
                    report.deleted += 1;
                }
                Err(error) => {
                    let attempts = attempts.saturating_add(1).min(u32::MAX as u64);
                    let delay = (1_i64 << attempts.min(12)).min(3600);
                    object.insert(format!("{CLEANUP_CLASS_ID}:attempts"), attempts);
                    object.insert(format!("{CLEANUP_CLASS_ID}:last_error"), error.to_string());
                    object.insert(
                        format!("{CLEANUP_CLASS_ID}:not_before"),
                        Value::DateTime(DateTime::from(now + time::Duration::seconds(delay))),
                    );
                    db.insert(collection, id, object).await?;
                    report.failed += 1;
                }
            }
        }
        if options.sweep_orphans && processed < options.max_entries {
            // Orphans enter the same durable retry queue before any deletion.
            // A separate pass deliberately gives new candidates another retention interval.
            let db = self
                .databases
                .first_key_value()
                .expect("nonempty registry")
                .1;
            for locator in
                self.orphan_candidates(cutoff, &live, &known, options.max_entries - processed)?
            {
                report.orphan_intents += 1;
                if !options.dry_run {
                    let id = format!("file-cleanup-{}", uuid::Uuid::new_v4());
                    let mut object = Object::new();
                    object.insert(ATTR_ID, id.clone());
                    object.insert(ATTR_TYPE, CLEANUP_CLASS_ID.to_owned());
                    object.insert(
                        format!("{CLEANUP_CLASS_ID}:store"),
                        self.store.safe_uri().to_string(),
                    );
                    object.insert(format!("{CLEANUP_CLASS_ID}:locator"), locator);
                    object.insert(
                        format!("{CLEANUP_CLASS_ID}:not_before"),
                        Value::DateTime(DateTime::from(now)),
                    );
                    object.insert(format!("{CLEANUP_CLASS_ID}:attempts"), 0_u64);
                    db.execute_batch(Batch::new().with_op(BatchOperation::Create {
                        collection: semantic_db_core::DEFAULT_COLLECTION.into(),
                        id,
                        object,
                    }))
                    .await?;
                }
            }
        }
        Ok(report)
    }

    fn orphan_candidates(
        &self,
        cutoff: time::OffsetDateTime,
        live: &BTreeSet<String>,
        known: &BTreeSet<String>,
        limit: usize,
    ) -> Result<Vec<String>, AppError> {
        let root = self.store.root.join("objects");
        if std::fs::symlink_metadata(&root)
            .map_err(io_error)?
            .file_type()
            .is_symlink()
        {
            return Err(invalid("managed object root is a symlink"));
        }
        let mut candidates = Vec::new();
        let mut consider = |path: PathBuf| -> Result<(), AppError> {
            let locator = path
                .strip_prefix(&root)
                .map_err(|_| invalid("orphan escaped store"))?
                .to_str()
                .ok_or_else(|| invalid("non-UTF8 locator"))?;
            if candidates.len() == limit
                || live.contains(locator)
                || known.contains(locator)
                || !self.safe_locator(locator)?
            {
                return Ok(());
            }
            let meta = std::fs::symlink_metadata(&path).map_err(io_error)?;
            if meta.is_file()
                && meta
                    .modified()
                    .ok()
                    .is_some_and(|date| time::OffsetDateTime::from(date) <= cutoff)
            {
                candidates.push(locator.to_owned());
            }
            Ok(())
        };
        // Only the two native immutable namespace shapes are enumerable. Never
        // recurse through arbitrary directories (or follow directory symlinks).
        for entry in std::fs::read_dir(&root).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let kind = entry.file_type().map_err(io_error)?;
            if kind.is_file() {
                consider(entry.path())?;
            } else if kind.is_dir()
                && entry.file_name().to_str().is_some_and(|name| {
                    name.strip_prefix("file-sha256-").is_some_and(|hash| {
                        hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                })
            {
                for child in std::fs::read_dir(entry.path()).map_err(io_error)? {
                    let child = child.map_err(io_error)?;
                    if child.file_type().map_err(io_error)?.is_file() {
                        consider(child.path())?;
                    }
                }
            }
        }
        Ok(candidates)
    }

    fn safe_locator(&self, locator: &str) -> Result<bool, AppError> {
        let parts: Vec<_> = locator.split('/').collect();
        let valid = match parts.as_slice() {
            [temporary] => temporary
                .strip_prefix("upload-tmp-")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok()),
            [hash, id] => {
                hash.strip_prefix("file-sha256-").is_some_and(|hash| {
                    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                }) && uuid::Uuid::parse_str(id).is_ok()
            }
            _ => false,
        };
        if !valid {
            return Ok(false);
        }
        let mut path = self.store.root.join("objects");
        for part in std::iter::once("").chain(parts.iter().copied()) {
            path.push(part);
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => return Ok(false),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(io_error(e)),
            }
        }
        Ok(true)
    }
}

fn value<'a>(object: &'a Object, qualified: &str, plain: &str) -> Option<&'a Value> {
    object.get(qualified).or_else(|| object.get(plain))
}
fn cleanup_value<'a>(object: &'a Object, field: &str) -> Option<&'a Value> {
    value(object, &format!("{CLEANUP_CLASS_ID}:{field}"), field)
}
fn cleanup_string<'a>(object: &'a Object, field: &str) -> Result<&'a str, AppError> {
    cleanup_value(object, field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(&format!("invalid cleanup {field}")))
}
fn unsigned(value: &Value) -> Option<u64> {
    match value {
        Value::U64(v) => Some(*v),
        Value::U32(v) => Some((*v).into()),
        Value::U16(v) => Some((*v).into()),
        Value::U8(v) => Some((*v).into()),
        _ => None,
    }
}

// Keep the lifetime lease in the object-store handle used by FileService/imports.
// Default trait helpers retain this handle throughout uploads and publication.
#[async_trait::async_trait]
impl ObjStore for ManagedFileStore {
    fn kind(&self) -> &str {
        "semantic.managed_fs"
    }
    fn safe_uri(&self) -> &url::Url {
        self.store.safe_uri()
    }
    async fn healthcheck(&self) -> Result<(), objstore::ObjStoreError> {
        self.store.healthcheck().await
    }
    async fn meta(
        &self,
        key: &str,
    ) -> Result<Option<objstore::ObjectMeta>, objstore::ObjStoreError> {
        self.store.meta(key).await
    }
    async fn get(&self, key: &str) -> Result<Option<bytes::Bytes>, objstore::ObjStoreError> {
        self.store.get(key).await
    }
    async fn get_stream(
        &self,
        key: &str,
    ) -> Result<Option<objstore::ValueStream>, objstore::ObjStoreError> {
        self.store.get_stream(key).await
    }
    async fn get_range_stream(
        &self,
        key: &str,
        range: objstore::ByteRange,
    ) -> Result<Option<objstore::ValueStream>, objstore::ObjStoreError> {
        self.store.get_range_stream(key, range).await
    }
    async fn get_with_meta(
        &self,
        key: &str,
    ) -> Result<Option<(bytes::Bytes, objstore::ObjectMeta)>, objstore::ObjStoreError> {
        self.store.get_with_meta(key).await
    }
    async fn get_stream_with_meta(
        &self,
        key: &str,
    ) -> Result<Option<(objstore::ObjectMeta, objstore::ValueStream)>, objstore::ObjStoreError>
    {
        self.store.get_stream_with_meta(key).await
    }
    async fn generate_download_url(
        &self,
        args: objstore::DownloadUrlArgs,
    ) -> Result<Option<url::Url>, objstore::ObjStoreError> {
        self.store.generate_download_url(args).await
    }
    async fn generate_upload_url(
        &self,
        args: objstore::UploadUrlArgs,
    ) -> Result<Option<url::Url>, objstore::ObjStoreError> {
        self.store.generate_upload_url(args).await
    }
    async fn send_put(
        &self,
        put: objstore::Put,
    ) -> Result<objstore::ObjectMeta, objstore::ObjStoreError> {
        self.store.send_put(put).await
    }
    async fn send_copy(
        &self,
        copy: objstore::Copy,
    ) -> Result<objstore::ObjectMeta, objstore::ObjStoreError> {
        self.store.send_copy(copy).await
    }
    async fn delete(&self, key: &str) -> Result<(), objstore::ObjStoreError> {
        self.store.delete(key).await
    }
    async fn delete_prefix(&self, prefix: &str) -> Result<(), objstore::ObjStoreError> {
        self.store.delete_prefix(prefix).await
    }
    async fn list_keys(
        &self,
        args: objstore::ListArgs,
    ) -> Result<objstore::KeyPage, objstore::ObjStoreError> {
        self.store.list_keys(args).await
    }
    async fn list(
        &self,
        args: objstore::ListArgs,
    ) -> Result<objstore::ObjectMetaPage, objstore::ObjStoreError> {
        self.store.list(args).await
    }
}

#[cfg(test)]
mod tests;
