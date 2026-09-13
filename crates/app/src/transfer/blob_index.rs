use std::ops::Bound;
use std::path::Path;

use redb::{ReadableTable as _, TableDefinition};
use semantic_data::filestore::{
    ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILESTORE_LOCATOR, ATTR_FILE_MIME_TYPE,
};
use semantic_data::value::{Object, Value};
use semantic_db_core::EntityRecord;

use super::TransferError;

const HASHES: TableDefinition<&str, &str> = TableDefinition::new("hashes");
const LOCATORS: TableDefinition<&str, &str> = TableDefinition::new("locators");
const ASSOCIATIONS: TableDefinition<&str, &str> = TableDefinition::new("associations");
const RECEIVED: TableDefinition<&str, &str> = TableDefinition::new("received");

pub(crate) struct BlobIndex {
    db: redb::Database,
}

impl BlobIndex {
    pub(crate) fn create(path: &Path) -> Result<Self, TransferError> {
        let db = redb::Database::create(path).map_err(index_error)?;
        let write = db.begin_write().map_err(index_error)?;
        {
            write.open_table(HASHES).map_err(index_error)?;
            write.open_table(LOCATORS).map_err(index_error)?;
            write.open_table(ASSOCIATIONS).map_err(index_error)?;
            write.open_table(RECEIVED).map_err(index_error)?;
        }
        write.commit().map_err(index_error)?;
        Ok(Self { db })
    }

    pub(crate) fn open(path: &Path) -> Result<Self, TransferError> {
        let db = redb::Database::open(path).map_err(index_error)?;
        Ok(Self { db })
    }

    pub(crate) fn add_entity(&self, record: &EntityRecord) -> Result<(), TransferError> {
        let Some(hash) =
            aliased_string(record, ATTR_FILE_CONTENT_HASH_SHA256, "content_hash_sha256")?
        else {
            return Ok(());
        };
        if !valid_hash(&hash) {
            return Err(TransferError::Blob(format!(
                "entity '{}:{}' has invalid SHA-256 hash '{hash}'",
                record.collection, record.id
            )));
        }
        let locator = aliased_string(record, ATTR_FILE_FILESTORE_LOCATOR, "filestore_locator")?
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                TransferError::Blob(format!(
                    "entity '{}:{}' has a content hash but no filestore locator",
                    record.collection, record.id
                ))
            })?;
        if locator.contains('\0') {
            return Err(TransferError::Blob(format!(
                "entity '{}:{}' has a locator containing a NUL byte",
                record.collection, record.id
            )));
        }
        let mime = aliased_string(record, ATTR_FILE_MIME_TYPE, "mime_type")?.unwrap_or_default();

        let write = self.db.begin_write().map_err(index_error)?;
        {
            let mut locators = write.open_table(LOCATORS).map_err(index_error)?;
            if let Some(existing) = locators.get(locator.as_str()).map_err(index_error)?
                && existing.value() != hash
            {
                return Err(TransferError::Blob(format!(
                    "locator '{locator}' is referenced with conflicting hashes '{}' and '{hash}'",
                    existing.value()
                )));
            }
            locators
                .insert(locator.as_str(), hash.as_str())
                .map_err(index_error)?;

            let mut hashes = write.open_table(HASHES).map_err(index_error)?;
            if hashes.get(hash.as_str()).map_err(index_error)?.is_none() {
                hashes
                    .insert(hash.as_str(), locator.as_str())
                    .map_err(index_error)?;
            }

            let key = association_key(&hash, &locator);
            write
                .open_table(ASSOCIATIONS)
                .map_err(index_error)?
                .insert(key.as_str(), mime.as_str())
                .map_err(index_error)?;
        }
        write.commit().map_err(index_error)?;
        Ok(())
    }

    pub(crate) fn mark_received(&self, hash: &str) -> Result<bool, TransferError> {
        let write = self.db.begin_write().map_err(index_error)?;
        let fresh = {
            let mut table = write.open_table(RECEIVED).map_err(index_error)?;
            if table.get(hash).map_err(index_error)?.is_some() {
                false
            } else {
                table.insert(hash, "1").map_err(index_error)?;
                true
            }
        };
        write.commit().map_err(index_error)?;
        Ok(fresh)
    }

    pub(crate) fn contains_hash(&self, hash: &str) -> Result<bool, TransferError> {
        let read = self.db.begin_read().map_err(index_error)?;
        let table = read.open_table(HASHES).map_err(index_error)?;
        Ok(table.get(hash).map_err(index_error)?.is_some())
    }

    pub(crate) fn is_received(&self, hash: &str) -> Result<bool, TransferError> {
        let read = self.db.begin_read().map_err(index_error)?;
        let table = read.open_table(RECEIVED).map_err(index_error)?;
        Ok(table.get(hash).map_err(index_error)?.is_some())
    }

    pub(crate) fn next_hash(
        &self,
        after: Option<&str>,
    ) -> Result<Option<(String, String)>, TransferError> {
        next_pair(&self.db, HASHES, after)
    }

    pub(crate) fn next_received(
        &self,
        after: Option<&str>,
    ) -> Result<Option<(String, String)>, TransferError> {
        next_pair(&self.db, RECEIVED, after)
    }

    pub(crate) fn next_association(
        &self,
        after: Option<&str>,
    ) -> Result<Option<(String, String, String, Option<String>)>, TransferError> {
        let Some((key, mime)) = next_pair(&self.db, ASSOCIATIONS, after)? else {
            return Ok(None);
        };
        let (hash, locator) = key
            .split_once('\0')
            .ok_or_else(|| TransferError::Index("malformed blob association key".into()))?;
        let hash = hash.to_string();
        let locator = locator.to_string();
        Ok(Some((
            key,
            hash,
            locator,
            (!mime.is_empty()).then_some(mime),
        )))
    }
}

fn next_pair(
    db: &redb::Database,
    definition: TableDefinition<&str, &str>,
    after: Option<&str>,
) -> Result<Option<(String, String)>, TransferError> {
    let read = db.begin_read().map_err(index_error)?;
    let table = read.open_table(definition).map_err(index_error)?;
    let bounds = match after {
        Some(after) => (Bound::Excluded(after), Bound::Unbounded),
        None => (Bound::Unbounded, Bound::Unbounded),
    };
    let mut range = table.range::<&str>(bounds).map_err(index_error)?;
    let Some(item) = range.next() else {
        return Ok(None);
    };
    let (key, value) = item.map_err(index_error)?;
    Ok(Some((key.value().to_string(), value.value().to_string())))
}

fn association_key(hash: &str, locator: &str) -> String {
    format!("{hash}\0{locator}")
}

fn aliased_string(
    record: &EntityRecord,
    qualified: &str,
    short: &str,
) -> Result<Option<String>, TransferError> {
    let qualified = optional_string(&record.object, qualified, record)?;
    let short = optional_string(&record.object, short, record)?;
    if let (Some(qualified), Some(short)) = (&qualified, &short)
        && qualified != short
    {
        return Err(TransferError::Blob(format!(
            "entity '{}:{}' has conflicting qualified and short blob attributes",
            record.collection, record.id
        )));
    }
    Ok(qualified.or(short))
}

fn optional_string(
    object: &Object,
    field: &str,
    record: &EntityRecord,
) -> Result<Option<String>, TransferError> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    let Value::String(value) = value else {
        return Err(TransferError::Blob(format!(
            "entity '{}:{}' has a non-string '{field}' attribute",
            record.collection, record.id
        )));
    };
    Ok(Some(value.clone()))
}

pub(crate) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn index_error(error: impl std::fmt::Display) -> TransferError {
    TransferError::Index(error.to_string())
}

#[cfg(test)]
mod tests {
    use semantic_data::filestore::{ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILESTORE_LOCATOR};
    use semantic_data::value::Object;

    use super::BlobIndex;

    #[test]
    fn indexes_only_hash_references_and_rejects_locator_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let index = BlobIndex::create(&temp.path().join("index.redb")).unwrap();
        let hash = "a".repeat(64);
        let mut object = Object::new();
        object.insert("id", semantic_data::value::Value::String("file-1".into()));
        object.insert(ATTR_FILE_CONTENT_HASH_SHA256, hash.clone());
        object.insert(
            ATTR_FILE_FILESTORE_LOCATOR,
            semantic_data::value::Value::String("files/one".into()),
        );
        let record = semantic_db_core::EntityRecord {
            id: "file-1".into(),
            collection: "entities".into(),
            object,
        };
        index.add_entity(&record).unwrap();
        assert_eq!(
            index.next_hash(None).unwrap(),
            Some((hash.clone(), "files/one".into()))
        );

        let mut conflicting = record.clone();
        conflicting
            .object
            .insert(ATTR_FILE_CONTENT_HASH_SHA256, "b".repeat(64));
        assert!(index.add_entity(&conflicting).is_err());

        let mut locator_only = Object::new();
        locator_only.insert("id", semantic_data::value::Value::String("file-2".into()));
        locator_only.insert(
            ATTR_FILE_FILESTORE_LOCATOR,
            semantic_data::value::Value::String("files/two".into()),
        );
        index
            .add_entity(&semantic_db_core::EntityRecord {
                id: "file-2".into(),
                collection: "entities".into(),
                object: locator_only,
            })
            .unwrap();
        assert!(index.next_hash(Some(&hash)).unwrap().is_none());
    }
}
