//! Per-collection field-name dictionaries of compact entity payloads.
//!
//! Each collection has an append-only dictionary stored as one meta key per
//! entry ([`keys::field_dict_key`]: `0x01 "fdict" lid(collection) id`, `id`
//! a big-endian `u32`), holding the UTF-8 field name. Ids are dense and never
//! reused or deleted (clearing a collection keeps its dictionary; they are
//! tiny), so a newer dictionary is a superset of every older one and decodes
//! every row an older one decodes.
//!
//! Writes allocate ids for unknown names inside the same engine write
//! transaction that writes the rows ([`DictStaging`]): names found in the
//! store's cache are used directly (committed ids never change); the first
//! unknown name of a collection reloads its dictionary through the write
//! transaction, so new ids are allocated against the committed state. The
//! new entries are written before the transaction commits, and the cache is
//! updated only after it committed.
//!
//! Reads never load dictionaries per row. A read handle
//! ([`KvEntitySnapshot`]) loads the dictionary of a collection through its
//! own read transaction on first use and keeps it (so scans decode with the
//! dictionary state matching their rows); point reads may use the
//! store-wide cache instead. A payload referencing an id missing from the
//! dictionary used triggers one reload.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

use semantic_db_core::DbError;
use semantic_db_core::catalog::LocalCollectionId;
use semantic_db_core::embedded::StoredEntity;

use super::entity_codec::{self, EntityPayloadFormat};
use super::value_codec::FieldNames;
use super::{EntityStore, KvEngine, KvEntitySnapshot, KvReadTxn, KvScanItem, KvWriteTxn};
use crate::keys;

/// The field-name dictionary of one collection.
#[derive(Debug, Default, Clone)]
pub(crate) struct FieldDict {
    names: Vec<String>,
    ids: HashMap<String, u32>,
}

impl FieldNames for FieldDict {
    fn field_name(&self, id: u32) -> Option<&str> {
        self.names.get(id as usize).map(String::as_str)
    }
}

impl FieldDict {
    pub(crate) fn len(&self) -> usize {
        self.names.len()
    }

    pub(crate) fn id(&self, name: &str) -> Option<u32> {
        self.ids.get(name).copied()
    }

    pub(crate) fn push(&mut self, name: &str) -> Result<u32, DbError> {
        let id = u32::try_from(self.names.len())
            .map_err(|_| DbError::Storage("field dictionary is full".to_string()))?;
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        Ok(id)
    }

    /// Load the dictionary of `collection` from its dictionary entries (as
    /// returned by a scan of [`keys::field_dict_prefix`]).
    pub(crate) fn load(
        collection: LocalCollectionId,
        entries: impl IntoIterator<Item = KvScanItem>,
    ) -> Result<Self, DbError> {
        let mut dict = Self::default();
        for entry in entries {
            let (key, name) = entry?;
            let id = match keys::parse_field_dict_key(&key) {
                Some((owner, id)) if owner == collection => id,
                _ => return Err(corrupt_dictionary(collection, "malformed key")),
            };
            if id as usize != dict.len() {
                return Err(corrupt_dictionary(collection, "missing field id"));
            }
            let name = std::str::from_utf8(&name)
                .map_err(|_| corrupt_dictionary(collection, "field name is not UTF-8"))?;
            dict.push(name)?;
        }
        Ok(dict)
    }

    /// Load every dictionary among raw `entries` (any key space).
    pub(crate) fn load_all<'e>(
        entries: impl IntoIterator<Item = &'e (Vec<u8>, Vec<u8>)>,
    ) -> Result<HashMap<LocalCollectionId, Arc<FieldDict>>, DbError> {
        let mut grouped = BTreeMap::<LocalCollectionId, Vec<(Vec<u8>, Vec<u8>)>>::new();
        for (key, value) in entries {
            if let Some((collection, _)) = keys::parse_field_dict_key(key) {
                grouped
                    .entry(collection)
                    .or_default()
                    .push((key.clone(), value.clone()));
            }
        }
        grouped
            .into_iter()
            .map(|(collection, mut entries)| {
                entries.sort();
                let dict = Self::load(collection, entries.into_iter().map(Ok))?;
                Ok((collection, Arc::new(dict)))
            })
            .collect()
    }
}

fn corrupt_dictionary(collection: LocalCollectionId, message: &str) -> DbError {
    DbError::Deserialization(format!(
        "corrupt field dictionary of collection {}: {message}",
        collection.0
    ))
}

/// Committed field-name dictionaries shared by a store and its read
/// handles.
#[derive(Debug, Default)]
pub(crate) struct FieldDictCache(RwLock<HashMap<LocalCollectionId, Arc<FieldDict>>>);

impl FieldDictCache {
    pub(crate) fn get(&self, collection: LocalCollectionId) -> Option<Arc<FieldDict>> {
        self.0
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .get(&collection)
            .cloned()
    }

    /// Record a committed dictionary state. Dictionaries only grow, so the
    /// larger of the cached and the given state wins.
    ///
    /// Returns the cached state.
    pub(crate) fn publish(
        &self,
        collection: LocalCollectionId,
        dict: Arc<FieldDict>,
    ) -> Arc<FieldDict> {
        let mut cache = self.0.write().unwrap_or_else(|err| err.into_inner());
        match cache.get(&collection) {
            Some(cached) if cached.len() >= dict.len() => cached.clone(),
            _ => {
                cache.insert(collection, dict.clone());
                dict
            }
        }
    }
}

/// Field-name dictionaries of one write transaction: committed ids plus the
/// names added by the transaction.
pub(crate) struct DictStaging<'c> {
    cache: &'c FieldDictCache,
    collections: HashMap<LocalCollectionId, StagedDict>,
}

struct StagedDict {
    dict: Arc<FieldDict>,
    /// Whether `dict` was loaded from the write transaction (and so holds
    /// every committed name), rather than taken from the cache.
    authoritative: bool,
    /// Length of the committed dictionary; later ids are new.
    committed_len: usize,
}

impl<'c> DictStaging<'c> {
    pub(crate) fn new(cache: &'c FieldDictCache) -> Self {
        Self {
            cache,
            collections: HashMap::new(),
        }
    }

    fn staged(&mut self, collection: LocalCollectionId) -> &mut StagedDict {
        self.collections.entry(collection).or_insert_with(|| {
            let dict = self.cache.get(collection).unwrap_or_default();
            StagedDict {
                committed_len: dict.len(),
                dict,
                authoritative: false,
            }
        })
    }

    fn load_authoritative(
        staged: &mut StagedDict,
        txn: &dyn KvWriteTxn,
        collection: LocalCollectionId,
    ) -> Result<(), DbError> {
        if !staged.authoritative {
            let entries = txn.scan_prefix(&keys::field_dict_prefix(collection))?;
            let dict = FieldDict::load(collection, entries.into_iter().map(Ok))?;
            staged.committed_len = dict.len();
            staged.dict = Arc::new(dict);
            staged.authoritative = true;
        }
        Ok(())
    }

    /// Id of `name` in the dictionary of `collection`, allocating one if the
    /// name is new.
    fn field_id(
        &mut self,
        txn: &dyn KvWriteTxn,
        collection: LocalCollectionId,
        name: &str,
    ) -> Result<u32, DbError> {
        let staged = self.staged(collection);
        if let Some(id) = staged.dict.id(name) {
            return Ok(id);
        }
        Self::load_authoritative(staged, txn, collection)?;
        if let Some(id) = staged.dict.id(name) {
            return Ok(id);
        }
        Arc::make_mut(&mut staged.dict).push(name)
    }

    /// The dictionary of `collection` as seen by the write transaction.
    pub(crate) fn dictionary(
        &mut self,
        txn: &dyn KvWriteTxn,
        collection: LocalCollectionId,
    ) -> Result<Arc<FieldDict>, DbError> {
        let staged = self.staged(collection);
        Self::load_authoritative(staged, txn, collection)?;
        Ok(staged.dict.clone())
    }

    /// Encode `entity` in `format`, allocating dictionary ids for new field
    /// names.
    pub(crate) fn encode(
        &mut self,
        txn: &dyn KvWriteTxn,
        entity: &StoredEntity,
        format: EntityPayloadFormat,
    ) -> Result<Vec<u8>, DbError> {
        match format {
            EntityPayloadFormat::SelfContained => entity_codec::encode_entity(entity),
            EntityPayloadFormat::Compact => {
                let collection = LocalCollectionId(entity.collection);
                entity_codec::encode_compact(entity, &mut |name: &str| {
                    self.field_id(txn, collection, name)
                })
            }
        }
    }

    /// Write the dictionary entries added by this transaction.
    pub(crate) fn write_additions(&self, txn: &mut dyn KvWriteTxn) -> Result<(), DbError> {
        for (collection, staged) in &self.collections {
            for id in staged.committed_len..staged.dict.len() {
                let id = id as u32;
                let name = staged.dict.field_name(id).unwrap_or_default();
                txn.put(&keys::field_dict_key(*collection, id), name.as_bytes())?;
            }
        }
        Ok(())
    }

    /// Record the dictionaries of a committed transaction in the cache.
    pub(crate) fn publish(self) {
        for (collection, staged) in self.collections {
            if staged.authoritative {
                self.cache.publish(collection, staged.dict);
            }
        }
    }
}

/// Decodes the entity rows of one collection scan.
#[derive(Debug)]
pub(crate) struct ScanDecoder {
    collection: LocalCollectionId,
    /// Length of the collection's entity key prefix; the id follows it.
    prefix_len: usize,
    dict: Arc<FieldDict>,
    /// Consulted when a row references an id missing from `dict` (rows
    /// committed after the scan's dictionary was resolved).
    cache: Arc<FieldDictCache>,
}

impl ScanDecoder {
    pub(crate) fn decode(&mut self, key: &[u8], payload: &[u8]) -> Result<StoredEntity, DbError> {
        let id = key
            .get(self.prefix_len..)
            .and_then(|id| std::str::from_utf8(id).ok())
            .ok_or_else(|| DbError::Deserialization("malformed entity key".to_string()))?;
        let (collection, dict, cache) = (self.collection, &mut self.dict, &self.cache);
        entity_codec::decode_stored(collection, id, payload, &mut |fresh| {
            if fresh
                && let Some(newer) = cache.get(collection)
                && newer.len() > dict.len()
            {
                *dict = newer;
            }
            Ok(dict.clone())
        })
    }
}

impl<E: KvEngine> EntityStore<E> {
    /// Dictionary of `collection`: cached, or loaded from the engine when
    /// absent or `fresh`.
    pub(crate) fn field_dictionary(
        &self,
        collection: LocalCollectionId,
        fresh: bool,
    ) -> Result<Arc<FieldDict>, DbError> {
        if !fresh && let Some(dict) = self.dictionaries.get(collection) {
            return Ok(dict);
        }
        let scan = self
            .engine
            .scan_prefix_stream(keys::field_dict_prefix(collection))?;
        let dict = Arc::new(FieldDict::load(collection, scan)?);
        Ok(self.dictionaries.publish(collection, dict))
    }

    pub(crate) fn scan_decoder(
        &self,
        collection: LocalCollectionId,
    ) -> Result<ScanDecoder, DbError> {
        Ok(ScanDecoder {
            collection,
            prefix_len: keys::entity_prefix(collection).len(),
            dict: self.field_dictionary(collection, false)?,
            cache: self.dictionaries.clone(),
        })
    }

    pub(crate) fn decode_entity_at(
        &self,
        collection: LocalCollectionId,
        id: &str,
        payload: &[u8],
    ) -> Result<StoredEntity, DbError> {
        entity_codec::decode_stored(collection, id, payload, &mut |fresh| {
            self.field_dictionary(collection, fresh)
        })
    }
}

/// Dictionaries loaded through one read handle.
#[derive(Debug, Default)]
pub(crate) struct HandleDicts(Mutex<HashMap<LocalCollectionId, Arc<FieldDict>>>);

impl KvEntitySnapshot<'_> {
    /// Dictionary of `collection` as seen by this handle, loaded through the
    /// handle on first use (or on every call when `reload`).
    fn handle_dictionary(
        &self,
        collection: LocalCollectionId,
        reload: bool,
    ) -> Result<Arc<FieldDict>, DbError> {
        let mut loaded = self.loaded.0.lock().unwrap_or_else(|err| err.into_inner());
        if !reload && let Some(dict) = loaded.get(&collection) {
            return Ok(dict.clone());
        }
        let dict = Arc::new(load_through(self.txn.as_ref(), collection)?);
        loaded.insert(collection, dict.clone());
        drop(loaded);
        // The handle observes committed state, so its dictionary is valid
        // for every reader.
        self.dictionaries.publish(collection, dict.clone());
        Ok(dict)
    }

    /// Dictionary for point reads: the handle's, else the store-wide cache,
    /// else loaded through the handle.
    fn point_dictionary(
        &self,
        collection: LocalCollectionId,
        fresh: bool,
    ) -> Result<Arc<FieldDict>, DbError> {
        if !fresh {
            let loaded = self.loaded.0.lock().unwrap_or_else(|err| err.into_inner());
            if let Some(dict) = loaded.get(&collection) {
                return Ok(dict.clone());
            }
            drop(loaded);
            if let Some(dict) = self.dictionaries.get(collection) {
                return Ok(dict);
            }
        }
        self.handle_dictionary(collection, fresh)
    }

    pub(crate) fn scan_decoder(
        &self,
        collection: LocalCollectionId,
    ) -> Result<ScanDecoder, DbError> {
        Ok(ScanDecoder {
            collection,
            prefix_len: keys::entity_prefix(collection).len(),
            dict: self.handle_dictionary(collection, false)?,
            cache: self.dictionaries.clone(),
        })
    }

    pub(crate) fn decode_entity_at(
        &self,
        collection: LocalCollectionId,
        id: &str,
        payload: &[u8],
    ) -> Result<StoredEntity, DbError> {
        entity_codec::decode_stored(collection, id, payload, &mut |fresh| {
            self.point_dictionary(collection, fresh)
        })
    }
}

fn load_through(txn: &dyn KvReadTxn, collection: LocalCollectionId) -> Result<FieldDict, DbError> {
    FieldDict::load(
        collection,
        txn.scan_prefix_stream(keys::field_dict_prefix(collection))?,
    )
}
