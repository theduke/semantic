use rmp_serde::{from_slice, to_vec_named};
use semantic_db_core::DbError;
use semantic_db_kv::KvWriteOp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"SEMWAL01";
const CHECKSUM_LEN: usize = 32;
const VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(u64);

impl EventId {
    pub const FIRST: Self = Self(1);

    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn next(self) -> std::result::Result<Self, DbError> {
        self.0
            .checked_add(1)
            .and_then(Self::new)
            .ok_or_else(|| DbError::Storage("WAL event id overflow".to_string()))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WalEvent {
    version: u16,
    id: u64,
    operations: Vec<KvWriteOp>,
}

pub(crate) fn encode(
    id: EventId,
    operations: &[KvWriteOp],
) -> std::result::Result<Vec<u8>, DbError> {
    let payload = to_vec_named(&WalEvent {
        version: VERSION,
        id: id.get(),
        operations: operations.to_vec(),
    })
    .map_err(|err| DbError::Serialization(format!("encode WAL event {}: {err}", id.get())))?;
    let checksum = Sha256::digest(&payload);
    let mut encoded = Vec::with_capacity(MAGIC.len() + CHECKSUM_LEN + payload.len());
    encoded.extend_from_slice(MAGIC);
    encoded.extend_from_slice(&checksum);
    encoded.extend_from_slice(&payload);
    Ok(encoded)
}

pub(crate) fn decode(
    expected_id: EventId,
    encoded: &[u8],
) -> std::result::Result<Vec<KvWriteOp>, DbError> {
    let header_len = MAGIC.len() + CHECKSUM_LEN;
    if encoded.len() < header_len || &encoded[..MAGIC.len()] != MAGIC {
        return Err(DbError::Storage(format!(
            "WAL event {} has an invalid format header",
            expected_id.get()
        )));
    }
    let checksum = &encoded[MAGIC.len()..header_len];
    let payload = &encoded[header_len..];
    if Sha256::digest(payload).as_slice() != checksum {
        return Err(DbError::Storage(format!(
            "WAL event {} checksum mismatch",
            expected_id.get()
        )));
    }
    let event: WalEvent = from_slice(payload).map_err(|err| {
        DbError::Deserialization(format!("decode WAL event {}: {err}", expected_id.get()))
    })?;
    if event.version != VERSION {
        return Err(DbError::Storage(format!(
            "unsupported WAL event version {} in event {}",
            event.version,
            expected_id.get()
        )));
    }
    if event.id != expected_id.get() {
        return Err(DbError::Storage(format!(
            "WAL key/event id mismatch: key {}, payload {}",
            expected_id.get(),
            event.id
        )));
    }
    if event.operations.is_empty() {
        return Err(DbError::Storage(format!(
            "WAL event {} contains an empty batch",
            expected_id.get()
        )));
    }
    Ok(event.operations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_event(event: &WalEvent) -> Vec<u8> {
        let payload = to_vec_named(event).unwrap();
        let checksum = Sha256::digest(&payload);
        [MAGIC.as_slice(), checksum.as_slice(), payload.as_slice()].concat()
    }

    fn event(version: u16, id: u64) -> WalEvent {
        WalEvent {
            version,
            id,
            operations: vec![KvWriteOp::Delete {
                key: b"key".to_vec(),
            }],
        }
    }

    #[test]
    fn decode_rejects_checksum_corruption() {
        let mut encoded = encode(EventId::FIRST, &event(VERSION, 1).operations).unwrap();
        *encoded.last_mut().unwrap() ^= 1;
        assert!(decode(EventId::FIRST, &encoded).is_err());
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let encoded = encode_event(&event(VERSION + 1, 1));
        assert!(decode(EventId::FIRST, &encoded).is_err());
    }

    #[test]
    fn decode_rejects_mismatched_event_id() {
        let encoded = encode_event(&event(VERSION, 2));
        assert!(decode(EventId::FIRST, &encoded).is_err());
    }
}
