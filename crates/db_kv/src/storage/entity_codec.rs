//! Entity payload encoding.
//!
//! Every entity payload starts with a little-endian `u16` format version:
//!
//! - version 1 ([`EntityPayloadFormat::SelfContained`]): MessagePack of
//!   `{ id, collection, kind, object }` with typed values and full field
//!   names. Self-describing; still written for stores that hand payloads to
//!   code without dictionary access (see [`EntityStore::with_payload_format`])
//!   and decoded forever.
//! - version 2 ([`EntityPayloadFormat::Compact`], the default): one header
//!   byte (entity kind in bits 0-1; bit 7 set when the object's `id` field
//!   equals the entity id and is omitted), then the object encoded with the
//!   compact value encoding (`storage::value_codec`), whose object field names
//!   (top level and nested) are ids of the collection's field-name
//!   dictionary. The entity id and collection come from the key.
//!
//! Field-name dictionaries are described in the `storage::field_dict`
//! module.
//!
//! [`EntityStore::with_payload_format`]: super::EntityStore::with_payload_format

use std::collections::BTreeMap;
use std::sync::Arc;

use semantic_data::value::Object;
use semantic_data::value::serde::typed::TypedValue;
use semantic_db_core::DbError;
use semantic_db_core::catalog::LocalCollectionId;
use semantic_db_core::embedded::{StoredEntity, StoredEntityKind};
use serde::{Deserialize, Serialize};

use semantic_db_core::DEFAULT_REWRITE_BATCH_SIZE;
use semantic_db_core::embedded::{PayloadRewriteBatch, StorageCommitOutcome};

use super::field_dict::{DictStaging, FieldDict};
use super::value_codec::{self, CodecError, FieldIds, FieldNames, Reader};
use super::{EntityStore, KvEngine, write_final_values};
use crate::keys;

const ENTITY_FORMAT_VERSION_PREFIX_LEN: usize = std::mem::size_of::<u16>();
/// Self-contained MessagePack entity payloads.
pub const ENTITY_FORMAT_VERSION_V1_MSGPACK: u16 = 1;
/// Compact entity payloads with dictionary-encoded field names.
pub const ENTITY_FORMAT_VERSION_V2_COMPACT: u16 = 2;

/// The object field holding the entity id.
const ID_FIELD: &str = "id";

const HEADER_KIND_MASK: u8 = 0b11;
const HEADER_ID_ELIDED: u8 = 0x80;
const KIND_UNTYPED: u8 = 0;
const KIND_RECORD: u8 = 1;
const KIND_CLASS: u8 = 2;

/// Payload format an [`EntityStore`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityPayloadFormat {
    /// Version 2: compact values and dictionary-encoded field names.
    #[default]
    Compact,
    /// Version 1: self-describing MessagePack, decodable with
    /// [`decode_entity`] alone.
    SelfContained,
}

impl EntityPayloadFormat {
    /// Payload format version written in this format.
    pub fn version(self) -> u16 {
        match self {
            Self::Compact => ENTITY_FORMAT_VERSION_V2_COMPACT,
            Self::SelfContained => ENTITY_FORMAT_VERSION_V1_MSGPACK,
        }
    }
}

/// Decode a self-contained (version 1) entity payload.
///
/// Compact (version 2) payloads need the collection's field-name dictionary
/// and are rejected; read them through [`EntityStore`].
pub fn decode_entity(payload: &[u8]) -> Result<StoredEntity, DbError> {
    match split_payload(payload)? {
        (ENTITY_FORMAT_VERSION_V1_MSGPACK, body) => decode_v1(body),
        (ENTITY_FORMAT_VERSION_V2_COMPACT, _) => Err(DbError::Deserialization(
            "compact entity payloads need the collection's field dictionary".to_string(),
        )),
        (version, _) => Err(unsupported_version(version)),
    }
}

/// Encode a self-contained (version 1) entity payload.
pub fn encode_entity(entity: &StoredEntity) -> Result<Vec<u8>, DbError> {
    let wire = StoredEntityWire {
        id: entity.id.clone(),
        collection: entity.collection,
        kind: (&entity.kind).into(),
        object: entity
            .object
            .iter()
            .map(|(k, v)| (k.clone(), TypedValue(v.clone())))
            .collect(),
    };
    let body = rmp_serde::to_vec(&wire).map_err(|err| DbError::Serialization(err.to_string()))?;
    let mut out = Vec::with_capacity(ENTITY_FORMAT_VERSION_PREFIX_LEN + body.len());
    out.extend_from_slice(&ENTITY_FORMAT_VERSION_V1_MSGPACK.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Format version of a stored payload.
pub(crate) fn payload_version(payload: &[u8]) -> Result<u16, DbError> {
    split_payload(payload).map(|(version, _)| version)
}

/// Encode a compact (version 2) payload, mapping field names through
/// `fields`.
pub(crate) fn encode_compact(
    entity: &StoredEntity,
    fields: &mut FieldIds<'_>,
) -> Result<Vec<u8>, DbError> {
    let id_elided = matches!(
        entity.object.get(ID_FIELD),
        Some(semantic_data::value::Value::String(id)) if *id == entity.id
    );
    let mut header = match entity.kind {
        StoredEntityKind::Untyped => KIND_UNTYPED,
        StoredEntityKind::Record => KIND_RECORD,
        StoredEntityKind::Class => KIND_CLASS,
    };
    if id_elided {
        header |= HEADER_ID_ELIDED;
    }
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&ENTITY_FORMAT_VERSION_V2_COMPACT.to_le_bytes());
    out.push(header);
    let len = entity.object.len() - usize::from(id_elided);
    let entries = entity
        .object
        .iter()
        .filter(|(name, _)| !(id_elided && name.as_str() == ID_FIELD));
    value_codec::encode_object_body(entries, len, &mut out, fields)?;
    Ok(out)
}

/// Decode a stored payload of any version for the entity key
/// `(collection, id)`.
///
/// `dictionary(fresh)` supplies the collection's field-name dictionary; it
/// is called at most twice, the second time with `fresh = true` when the
/// first dictionary lacks an id used by the payload.
pub(crate) fn decode_stored(
    collection: LocalCollectionId,
    id: &str,
    payload: &[u8],
    dictionary: &mut dyn FnMut(bool) -> Result<Arc<FieldDict>, DbError>,
) -> Result<StoredEntity, DbError> {
    match split_payload(payload)? {
        (ENTITY_FORMAT_VERSION_V1_MSGPACK, body) => decode_v1(body),
        (ENTITY_FORMAT_VERSION_V2_COMPACT, body) => {
            match decode_compact(collection, id, body, dictionary(false)?.as_ref()) {
                Err(CodecError::UnknownField(_)) => Ok(decode_compact(
                    collection,
                    id,
                    body,
                    dictionary(true)?.as_ref(),
                )?),
                result => Ok(result?),
            }
        }
        (version, _) => Err(unsupported_version(version)),
    }
}

fn decode_compact(
    collection: LocalCollectionId,
    id: &str,
    body: &[u8],
    names: &dyn FieldNames,
) -> Result<StoredEntity, CodecError> {
    let mut reader = Reader::new(body);
    let header = reader.byte()?;
    if header & !(HEADER_KIND_MASK | HEADER_ID_ELIDED) != 0 {
        return Err(CodecError::Malformed(format!(
            "unknown entity header bits {header:#04x}"
        )));
    }
    let kind = match header & HEADER_KIND_MASK {
        KIND_UNTYPED => StoredEntityKind::Untyped,
        KIND_RECORD => StoredEntityKind::Record,
        KIND_CLASS => StoredEntityKind::Class,
        other => {
            return Err(CodecError::Malformed(format!(
                "unknown entity kind {other}"
            )));
        }
    };
    let mut object = BTreeMap::new();
    reader.object_body(names, &mut object)?;
    if !reader.is_empty() {
        return Err(CodecError::Malformed(
            "trailing bytes after entity object".to_string(),
        ));
    }
    if header & HEADER_ID_ELIDED != 0 {
        object.insert(
            ID_FIELD.to_string(),
            semantic_data::value::Value::String(id.to_string()),
        );
    }
    Ok(StoredEntity {
        id: id.to_string(),
        collection: collection.0,
        kind,
        object: Object::from(object),
    })
}

fn split_payload(payload: &[u8]) -> Result<(u16, &[u8]), DbError> {
    let Some((prefix, body)) = payload.split_first_chunk::<ENTITY_FORMAT_VERSION_PREFIX_LEN>()
    else {
        return Err(DbError::Deserialization(
            "entity payload missing format version prefix".to_string(),
        ));
    };
    Ok((u16::from_le_bytes(*prefix), body))
}

fn unsupported_version(version: u16) -> DbError {
    DbError::Deserialization(format!(
        "unsupported entity payload format version: {version}"
    ))
}

#[derive(Serialize, Deserialize)]
struct StoredEntityWire {
    id: String,
    collection: usize,
    kind: StoredEntityKindWire,
    object: BTreeMap<String, TypedValue>,
}

#[derive(Serialize, Deserialize)]
enum StoredEntityKindWire {
    Untyped,
    Record,
    Class,
}

impl From<&StoredEntityKind> for StoredEntityKindWire {
    fn from(value: &StoredEntityKind) -> Self {
        match value {
            StoredEntityKind::Untyped => Self::Untyped,
            StoredEntityKind::Record => Self::Record,
            StoredEntityKind::Class => Self::Class,
        }
    }
}

impl From<StoredEntityKindWire> for StoredEntityKind {
    fn from(value: StoredEntityKindWire) -> Self {
        match value {
            StoredEntityKindWire::Untyped => Self::Untyped,
            StoredEntityKindWire::Record => Self::Record,
            StoredEntityKindWire::Class => Self::Class,
        }
    }
}

fn decode_v1(body: &[u8]) -> Result<StoredEntity, DbError> {
    let wire: StoredEntityWire =
        rmp_serde::from_slice(body).map_err(|err| DbError::Deserialization(err.to_string()))?;
    let object = wire
        .object
        .into_iter()
        .map(|(field, value)| (field, value.0))
        .collect::<BTreeMap<_, _>>();
    Ok(StoredEntity {
        id: wire.id,
        collection: wire.collection,
        kind: wire.kind.into(),
        object: Object::from(object),
    })
}

impl<E: KvEngine> EntityStore<E> {
    /// Rewrite every entity payload that is not in the store's payload
    /// format (for example version 1 rows of databases written before the
    /// compact format) in the store's format.
    ///
    /// Rows are otherwise upgraded lazily when they are next written; this
    /// explicit maintenance step upgrades all of them. It runs in batches of
    /// [`DEFAULT_REWRITE_BATCH_SIZE`] rewritten rows per write transaction
    /// (see [`Self::rewrite_entities_batch`]), so it never holds more than
    /// one batch of payloads. Entity keys (and so the maintained row counts)
    /// are unchanged. Returns the number of rewritten entities.
    pub fn rewrite_all_entities_to_current_format(&mut self) -> Result<usize, DbError> {
        let mut rewritten = 0;
        let mut resume_after = None;
        loop {
            let batch =
                self.rewrite_entities_batch(resume_after.as_deref(), DEFAULT_REWRITE_BATCH_SIZE)?;
            rewritten += batch.rewritten as usize;
            resume_after = batch.resume_after;
            if resume_after.is_none() {
                return Ok(rewritten);
            }
        }
    }

    /// Rewrite up to `limit` outdated entity payloads whose keys follow
    /// `resume_after` (all keys when `None`) in one write transaction.
    ///
    /// Candidates are found through a read handle, keeping only their keys;
    /// the write transaction re-reads each of them, so rows a concurrent
    /// write already upgraded or deleted are skipped. The returned resume
    /// position is the last examined key, or `None` once all entities were
    /// examined.
    pub fn rewrite_entities_batch(
        &mut self,
        resume_after: Option<&[u8]>,
        limit: usize,
    ) -> Result<PayloadRewriteBatch, DbError> {
        let format = self.payload_format;
        let start = match resume_after {
            // The smallest key after `resume_after`.
            Some(key) => [key, &[0]].concat(),
            None => vec![keys::TAG_ENTITY],
        };
        let end = super::prefix_range_end(&[keys::TAG_ENTITY]);
        let mut scanned = 0;
        let mut candidates = Vec::new();
        let mut last = None;
        let mut exhausted = true;
        {
            let txn = self.engine.begin_read()?;
            for entry in txn.scan_range_stream(start, end)? {
                if candidates.len() >= limit.max(1) {
                    exhausted = false;
                    break;
                }
                let (key, payload) = entry?;
                scanned += 1;
                if payload_version(&payload)? != format.version() {
                    candidates.push(key.clone());
                }
                last = Some(key);
            }
        }

        let mut staging = DictStaging::new(&self.dictionaries);
        let mut rewritten = 0;
        let outcome = self.engine.write_with(None, |txn| {
            let mut values = Vec::new();
            for key in &candidates {
                let Some(payload) = txn.get(key)? else {
                    continue;
                };
                if payload_version(&payload)? == format.version() {
                    continue;
                }
                let (collection, id) = keys::parse_entity_key(key)
                    .ok_or_else(|| DbError::Deserialization("malformed entity key".to_string()))?;
                let entity = decode_stored(collection, id, &payload, &mut |_| {
                    staging.dictionary(&*txn, collection)
                })?;
                let payload = staging.encode(&*txn, &entity, format)?;
                values.push((key.clone(), Some(payload)));
            }
            rewritten = values.len() as u64;
            staging.write_additions(txn)?;
            write_final_values(txn, values)
        })?;
        if matches!(outcome, StorageCommitOutcome::Committed { .. }) {
            staging.publish();
        }
        Ok(PayloadRewriteBatch {
            scanned,
            rewritten,
            resume_after: if exhausted { None } else { last },
        })
    }
}

#[cfg(test)]
#[path = "entity_codec_tests.rs"]
mod tests;
