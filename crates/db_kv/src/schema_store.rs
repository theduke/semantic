use semantic_db_core::catalog::{Catalog, CatalogStorageSnapshot};

use crate::{error::DbError, storage::KvWriteOp};

const CATALOG_SNAPSHOT_KEY: &[u8] = b"__semantic/meta/catalog_snapshot";

pub fn decode_catalog(bytes: &[u8]) -> std::result::Result<Catalog, DbError> {
    let snapshot: CatalogStorageSnapshot =
        facet_json::from_slice(bytes).map_err(|err| DbError::Deserialization(err.to_string()))?;
    Catalog::from_storage_snapshot(snapshot).map_err(Into::into)
}

pub fn encode_catalog(catalog: &Catalog) -> std::result::Result<Vec<u8>, DbError> {
    let snapshot = catalog.to_storage_snapshot();
    facet_json::to_vec(&snapshot).map_err(|err| DbError::Serialization(err.to_string()))
}

pub fn catalog_snapshot_key() -> Vec<u8> {
    CATALOG_SNAPSHOT_KEY.to_vec()
}

pub fn catalog_write_op(catalog: &Catalog) -> std::result::Result<KvWriteOp, DbError> {
    Ok(KvWriteOp::Put {
        key: catalog_snapshot_key(),
        value: encode_catalog(catalog)?,
    })
}
