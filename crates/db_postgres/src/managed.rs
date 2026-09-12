use std::collections::{BTreeMap, BTreeSet};

use semantic_data::schema::{
    FloatWidth, IndexKind, IntWidth, NumberType, TemporalType, Type, TypeKind, UIntWidth,
};
use semantic_data::value::Value;
use semantic_db_core::catalog::{
    Catalog, CollectionKind, IntegrityMode, LocalClassId, LocalCollectionId,
};
use semantic_db_core::embedded::{
    EmbeddedDb, StorageCommitOutcome, StorageTransactionCapabilities, StoredEntity,
    StoredEntityKind,
};
use semantic_db_core::{DEFAULT_COLLECTION, DbConfig, DbError};
use semantic_db_kv::{
    BoxKvPrefixScan, EntityStore, KvEngine, KvWriteOp, decode_entity, encode_entity,
    parse_entity_key,
};
use sha2::{Digest, Sha256};
use tokio_postgres::Transaction;

use crate::codec::{decode_object, encode_object, encode_value};
use crate::sql::quote_ident;

const FORMAT_VERSION: i32 = 1;

#[derive(Debug, Clone, Default)]
pub(crate) struct PostgresSnapshotEngine {
    entries: BTreeMap<Vec<u8>, Vec<u8>>,
    changes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    revision: u64,
}

impl PostgresSnapshotEngine {
    fn from_entries(entries: BTreeMap<Vec<u8>, Vec<u8>>, revision: u64) -> Self {
        Self {
            entries,
            changes: BTreeMap::new(),
            revision,
        }
    }

    fn entries(&self) -> &BTreeMap<Vec<u8>, Vec<u8>> {
        &self.entries
    }

    fn changes(&self) -> &BTreeMap<Vec<u8>, Option<Vec<u8>>> {
        &self.changes
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

impl KvEngine for PostgresSnapshotEngine {
    type PrefixScan = BoxKvPrefixScan;

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self.entries.get(key).cloned())
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), DbError> {
        self.entries.insert(key.clone(), value.clone());
        self.changes.insert(key, Some(value));
        self.bump_revision();
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), DbError> {
        self.entries.remove(key);
        self.changes.insert(key.to_vec(), None);
        self.bump_revision();
        Ok(())
    }

    fn scan_prefix_stream(&self, prefix: Vec<u8>) -> Result<Self::PrefixScan, DbError> {
        let rows = self
            .entries
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix))
            .map(|(key, value)| Ok((key.clone(), value.clone())))
            .collect::<Vec<_>>();
        Ok(Box::new(rows.into_iter()))
    }

    fn write_batch(&mut self, ops: &[KvWriteOp]) -> Result<(), DbError> {
        for op in ops {
            match op {
                KvWriteOp::Put { key, value } => {
                    self.entries.insert(key.clone(), value.clone());
                    self.changes.insert(key.clone(), Some(value.clone()));
                }
                KvWriteOp::Delete { key } => {
                    self.entries.remove(key);
                    self.changes.insert(key.clone(), None);
                }
            }
        }
        if !ops.is_empty() {
            self.bump_revision();
        }
        Ok(())
    }

    fn tx_capabilities(&self) -> StorageTransactionCapabilities {
        StorageTransactionCapabilities {
            conflict_detection: true,
            mvcc: false,
            snapshot_reads: false,
        }
    }

    fn current_revision(&self) -> Result<Option<u64>, DbError> {
        Ok(Some(self.revision))
    }

    fn write_batch_conditional(
        &mut self,
        ops: &[KvWriteOp],
        expected_revision: Option<u64>,
    ) -> Result<StorageCommitOutcome, DbError> {
        let actual = Some(self.revision);
        if expected_revision.is_some() && expected_revision != actual {
            return Ok(StorageCommitOutcome::Conflict {
                expected_revision,
                actual_revision: actual,
            });
        }
        self.write_batch(ops)?;
        Ok(StorageCommitOutcome::Committed {
            revision: Some(self.revision),
        })
    }
}

pub(crate) async fn bootstrap(
    transaction: &Transaction<'_>,
    schema: &str,
    layout: &str,
) -> Result<(), DbError> {
    let schema = quote_ident(schema);
    transaction
        .batch_execute(&format!(
            "CREATE SCHEMA IF NOT EXISTS {schema};
             CREATE TABLE IF NOT EXISTS {schema}.catalog_state (
                 singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
                 revision bigint NOT NULL,
                 format_version integer NOT NULL,
                 layout text NOT NULL,
                 snapshot jsonb NOT NULL,
                 updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
             );
             CREATE TABLE IF NOT EXISTS {schema}.kv_entries (
                 key_hash bytea PRIMARY KEY,
                 key bytea NOT NULL,
                 value bytea NOT NULL
             );
             CREATE TABLE IF NOT EXISTS {schema}.entities (
                 collection_lid bigint NOT NULL,
                 entity_id text NOT NULL,
                 object_type text,
                 parent_id text,
                 document jsonb NOT NULL,
                 row_revision bigint NOT NULL,
                 PRIMARY KEY(collection_lid, entity_id)
             );
             CREATE INDEX IF NOT EXISTS semantic_entities_collection_type_idx
                 ON {schema}.entities(collection_lid, object_type);
             CREATE INDEX IF NOT EXISTS semantic_entities_collection_parent_idx
                 ON {schema}.entities(collection_lid, parent_id);
             CREATE INDEX IF NOT EXISTS semantic_entities_document_gin
                 ON {schema}.entities USING GIN(document jsonb_ops);
             CREATE TABLE IF NOT EXISTS {schema}.relation_edges (
                 relation_lid bigint NOT NULL,
                 source_collection_lid bigint NOT NULL,
                 source_id text NOT NULL,
                 target_collection_lid bigint NOT NULL,
                 target_id text NOT NULL,
                 depth bigint NOT NULL DEFAULT 1,
                 edge_collection_lid bigint,
                 edge_entity_id text,
                 PRIMARY KEY(relation_lid, source_collection_lid, source_id,
                             target_collection_lid, target_id)
             );
             CREATE INDEX IF NOT EXISTS semantic_relation_edges_forward_idx
                 ON {schema}.relation_edges(relation_lid, source_collection_lid, source_id);
             CREATE INDEX IF NOT EXISTS semantic_relation_edges_reverse_idx
                 ON {schema}.relation_edges(relation_lid, target_collection_lid, target_id);
             CREATE TABLE IF NOT EXISTS {schema}.collection_map (
                 collection_lid bigint PRIMARY KEY,
                 collection_name text NOT NULL UNIQUE,
                 class_identity text,
                 physical_schema text,
                 physical_table text,
                 managed boolean NOT NULL DEFAULT true,
                 mapping jsonb NOT NULL DEFAULT '{{}}'::jsonb
             );
             CREATE TABLE IF NOT EXISTS {schema}.field_map (
                 collection_lid bigint NOT NULL,
                 field_id bigint NOT NULL,
                 canonical_field text NOT NULL,
                 physical_column text,
                 physical_type text,
                 writable boolean NOT NULL DEFAULT true,
                 mapping jsonb NOT NULL DEFAULT '{{}}'::jsonb,
                 PRIMARY KEY(collection_lid, field_id)
             );"
        ))
        .await
        .map_err(storage_error)?;
    transaction
        .execute(
            &format!(
                "INSERT INTO {schema}.catalog_state
                 (singleton, revision, format_version, layout, snapshot)
                 VALUES (true, 0, $1, $2, '{{}}'::jsonb)
                 ON CONFLICT (singleton) DO NOTHING"
            ),
            &[&FORMAT_VERSION, &layout],
        )
        .await
        .map_err(storage_error)?;
    let stored_layout: String = transaction
        .query_one(
            &format!("SELECT layout FROM {schema}.catalog_state WHERE singleton = true FOR UPDATE"),
            &[],
        )
        .await
        .map_err(storage_error)?
        .get(0);
    if stored_layout != layout {
        return Err(DbError::InvalidQuery(format!(
            "PostgreSQL metadata schema '{schema}' uses {stored_layout} layout, not {layout}"
        )));
    }
    Ok(())
}

pub(crate) async fn lock_and_load(
    transaction: &Transaction<'_>,
    schema_name: &str,
    config: DbConfig,
    for_write: bool,
) -> Result<EmbeddedDb<EntityStore<PostgresSnapshotEngine>>, DbError> {
    if for_write {
        transaction
            .query_one(
                "SELECT pg_advisory_xact_lock(hashtext($1))",
                &[&schema_name],
            )
            .await
            .map_err(storage_error)?;
    }
    let schema = quote_ident(schema_name);
    let lock_clause = if for_write { " FOR UPDATE" } else { "" };
    let state = transaction
        .query_one(
            &format!(
                "SELECT revision, format_version, snapshot FROM {schema}.catalog_state
                 WHERE singleton = true{lock_clause}"
            ),
            &[],
        )
        .await
        .map_err(storage_error)?;
    let revision: i64 = state.get(0);
    let format_version: i32 = state.get(1);
    let snapshot: serde_json::Value = state.get(2);
    if format_version != FORMAT_VERSION {
        return Err(DbError::Deserialization(format!(
            "unsupported PostgreSQL metadata format {format_version}"
        )));
    }
    let rows = transaction
        .query(&format!("SELECT key, value FROM {schema}.kv_entries"), &[])
        .await
        .map_err(storage_error)?;
    let mut entries: BTreeMap<Vec<u8>, Vec<u8>> = rows
        .into_iter()
        .map(|row| (row.get::<_, Vec<u8>>(0), row.get::<_, Vec<u8>>(1)))
        .collect();
    let entity_rows = transaction
        .query(
            &format!("SELECT collection_lid, entity_id, document FROM {schema}.entities"),
            &[],
        )
        .await
        .map_err(storage_error)?;
    for row in entity_rows {
        let collection: i64 = row.get(0);
        let id: String = row.get(1);
        let document: serde_json::Value = row.get(2);
        let entity = decode_stored_document(
            usize::try_from(collection).map_err(|_| {
                DbError::Deserialization(format!("invalid collection lid {collection}"))
            })?,
            id.clone(),
            &document,
        )?;
        entries.insert(
            format!("c/{collection}/e/{id}").into_bytes(),
            encode_entity(&entity)?,
        );
    }
    let mut db = EmbeddedDb::open_with_config(
        EntityStore::new(PostgresSnapshotEngine::from_entries(
            entries,
            revision.max(0) as u64,
        )),
        config,
    )?;
    if snapshot
        .as_object()
        .is_some_and(|snapshot| !snapshot.is_empty())
    {
        let snapshot = facet_json::from_str::<semantic_db_core::catalog::CatalogStorageSnapshot>(
            &snapshot.to_string(),
        )
        .map_err(|error| DbError::Deserialization(error.to_string()))?;
        let catalog = semantic_db_core::catalog::Catalog::from_storage_snapshot(snapshot)
            .map_err(DbError::from)?;
        db.replace_catalog_snapshot(catalog);
    }
    Ok(db)
}

pub(crate) async fn save(
    transaction: &Transaction<'_>,
    schema_name: &str,
    db: EmbeddedDb<EntityStore<PostgresSnapshotEngine>>,
    layout: &str,
) -> Result<semantic_db_core::catalog::Catalog, DbError> {
    let catalog = db.catalog().as_ref().clone();
    let snapshot = facet_json::to_string(&catalog.to_storage_snapshot())
        .map_err(|err| DbError::Serialization(err.to_string()))?;
    let snapshot: serde_json::Value =
        serde_json::from_str(&snapshot).map_err(|err| DbError::Serialization(err.to_string()))?;
    let (_, storage) = db.into_parts();
    let engine = storage.into_inner();
    let schema = quote_ident(schema_name);
    let previous_snapshot: serde_json::Value = transaction
        .query_one(
            &format!("SELECT snapshot FROM {schema}.catalog_state WHERE singleton = true"),
            &[],
        )
        .await
        .map_err(storage_error)?
        .get(0);
    let schema_changed = previous_snapshot != snapshot;
    let kv_put = transaction
        .prepare(&format!(
            "INSERT INTO {schema}.kv_entries (key_hash, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (key_hash) DO UPDATE SET key = EXCLUDED.key, value = EXCLUDED.value"
        ))
        .await
        .map_err(storage_error)?;
    let entity_put = transaction
        .prepare(&format!(
            "INSERT INTO {schema}.entities
             (collection_lid, entity_id, object_type, parent_id, document, row_revision)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (collection_lid, entity_id) DO UPDATE SET
                 object_type = EXCLUDED.object_type,
                 parent_id = EXCLUDED.parent_id,
                 document = EXCLUDED.document,
                 row_revision = EXCLUDED.row_revision"
        ))
        .await
        .map_err(storage_error)?;
    let row_revision = i64::try_from(engine.revision).unwrap_or(i64::MAX);
    for (key, value) in engine.changes() {
        if let Some((collection, id)) = parse_entity_key(key) {
            let collection = i64::try_from(collection.0).map_err(|_| {
                DbError::Serialization("collection lid exceeds PostgreSQL bigint".to_string())
            })?;
            if let Some(value) = value {
                let entity = decode_entity(value)?;
                let object_type = entity
                    .object
                    .get(semantic_db_core::catalog::OBJECT_TYPE_FIELD)
                    .and_then(semantic_data::value::Value::as_str);
                let parent_id = entity
                    .object
                    .get(semantic_db_core::catalog::PARENT_RELATION_FIELD)
                    .and_then(semantic_data::value::Value::as_str);
                let document = encode_stored_document(&entity);
                transaction
                    .execute(
                        &entity_put,
                        &[
                            &collection,
                            &id,
                            &object_type,
                            &parent_id,
                            &document,
                            &row_revision,
                        ],
                    )
                    .await
                    .map_err(storage_error)?;
                transaction
                    .execute(
                        &format!("DELETE FROM {schema}.kv_entries WHERE key_hash = $1"),
                        &[&key_hash(key)],
                    )
                    .await
                    .map_err(storage_error)?;
            } else {
                transaction
                    .execute(
                        &format!(
                            "DELETE FROM {schema}.entities
                             WHERE collection_lid = $1 AND entity_id = $2"
                        ),
                        &[&collection, &id],
                    )
                    .await
                    .map_err(storage_error)?;
            }
        } else if let Some(value) = value {
            transaction
                .execute(&kv_put, &[&key_hash(key), key, value])
                .await
                .map_err(storage_error)?;
        } else {
            transaction
                .execute(
                    &format!("DELETE FROM {schema}.kv_entries WHERE key_hash = $1"),
                    &[&key_hash(key)],
                )
                .await
                .map_err(storage_error)?;
        }
    }
    rebuild_relation_edges(transaction, &schema, &catalog, &engine).await?;
    if layout == "relational" {
        sync_relational_projection(
            transaction,
            &schema,
            schema_name,
            &catalog,
            &engine,
            schema_changed,
        )
        .await?;
    }
    sync_mapping_metadata(transaction, &schema, schema_name, &catalog, layout).await?;
    let revision = row_revision;
    transaction
        .execute(
            &format!(
                "UPDATE {schema}.catalog_state SET revision = $1, layout = $2,
                 snapshot = $3, updated_at = clock_timestamp() WHERE singleton = true"
            ),
            &[&revision, &layout, &snapshot],
        )
        .await
        .map_err(storage_error)?;
    Ok(catalog)
}

fn key_hash(key: &[u8]) -> Vec<u8> {
    Sha256::digest(key).to_vec()
}

async fn sync_mapping_metadata(
    transaction: &Transaction<'_>,
    schema: &str,
    schema_name: &str,
    catalog: &semantic_db_core::catalog::Catalog,
    layout: &str,
) -> Result<(), DbError> {
    transaction
        .execute(&format!("DELETE FROM {schema}.field_map"), &[])
        .await
        .map_err(storage_error)?;
    transaction
        .execute(&format!("DELETE FROM {schema}.collection_map"), &[])
        .await
        .map_err(storage_error)?;
    let collection_insert = transaction
        .prepare(&format!(
            "INSERT INTO {schema}.collection_map
             (collection_lid, collection_name, class_identity, physical_schema,
              physical_table, managed, mapping)
             VALUES ($1, $2, $3, $4, $5, true, $6)"
        ))
        .await
        .map_err(storage_error)?;
    let field_insert = transaction
        .prepare(&format!(
            "INSERT INTO {schema}.field_map
             (collection_lid, field_id, canonical_field, physical_column,
              physical_type, writable, mapping)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        ))
        .await
        .map_err(storage_error)?;
    let relational = if layout == "relational" {
        relational_collection_specs(catalog)?
            .into_iter()
            .map(|spec| (spec.collection_lid, spec))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    for (_, collection) in catalog.collections() {
        let collection_lid = i64::try_from(collection.lid.0)
            .map_err(|_| DbError::Serialization("collection lid exceeds bigint".to_string()))?;
        let class_identity = catalog
            .class_ids(&collection.name)
            .first()
            .and_then(|class| catalog.class_by_lid(*class))
            .map(|class| class.class.id.as_str());
        let spec = relational.get(&collection.lid);
        let physical_table = spec
            .map(|spec| spec.table_name.as_str())
            .unwrap_or("entities");
        let mapped_layout = if spec.is_some() {
            layout
        } else if layout == "relational" {
            "semantic-support"
        } else {
            layout
        };
        let mapping = serde_json::json!({ "layout": mapped_layout });
        transaction
            .execute(
                &collection_insert,
                &[
                    &collection_lid,
                    &collection.name,
                    &class_identity,
                    &schema_name,
                    &physical_table,
                    &mapping,
                ],
            )
            .await
            .map_err(storage_error)?;
        let fields = spec
            .map(|spec| {
                spec.fields
                    .iter()
                    .map(|field| {
                        (
                            field.field_id,
                            field.canonical.as_str(),
                            field.computed,
                            Some(field.semantic_type.as_str()),
                            Some(field.physical_type.sql()),
                            field.physical_type.codec(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                collection
                    .fields()
                    .map(|(field_id, canonical)| {
                        (field_id, canonical, false, None, None, "entity-document")
                    })
                    .collect()
            });
        for (field_id, canonical_field, computed, semantic_type, relational_physical_type, codec) in
            fields
        {
            let field_id = i64::try_from(field_id.0)
                .map_err(|_| DbError::Serialization("field id exceeds bigint".to_string()))?;
            let physical_column = spec
                .filter(|_| !computed)
                .map(|_| relational_field_name(field_id));
            let physical_type = physical_column.as_ref().and(relational_physical_type);
            let field_mapping = serde_json::json!({
                "layout": mapped_layout,
                "codec": codec,
                "semantic_type": semantic_type,
            });
            let writable = !computed;
            transaction
                .execute(
                    &field_insert,
                    &[
                        &collection_lid,
                        &field_id,
                        &canonical_field,
                        &physical_column,
                        &physical_type,
                        &writable,
                        &field_mapping,
                    ],
                )
                .await
                .map_err(storage_error)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct RelationalFieldSpec {
    field_id: semantic_db_core::catalog::LocalFieldId,
    canonical: String,
    computed: bool,
    semantic_type: String,
    physical_type: RelationalPhysicalType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationalPhysicalType {
    Bool,
    SmallInt,
    Integer,
    BigInt,
    Numeric,
    Real,
    Double,
    Text,
    Uuid,
    Inet,
    Bytea,
    Date,
    Time,
    Timestamptz,
    TaggedJsonb,
}

impl RelationalPhysicalType {
    fn sql(self) -> &'static str {
        match self {
            Self::Bool => "boolean",
            Self::SmallInt => "smallint",
            Self::Integer => "integer",
            Self::BigInt => "bigint",
            Self::Numeric => "numeric",
            Self::Real => "real",
            Self::Double => "double precision",
            Self::Text => "text",
            Self::Uuid => "uuid",
            Self::Inet => "inet",
            Self::Bytea => "bytea",
            Self::Date => "date",
            Self::Time => "time without time zone",
            Self::Timestamptz => "timestamp with time zone",
            Self::TaggedJsonb => "jsonb",
        }
    }

    fn codec(self) -> &'static str {
        match self {
            Self::TaggedJsonb => "tagged-jsonb-v1",
            _ => "native-v1",
        }
    }
}

#[derive(Debug, Clone)]
struct RelationalCollectionSpec {
    collection_lid: LocalCollectionId,
    collection_name: String,
    class_identity: String,
    table_name: String,
    fields: Vec<RelationalFieldSpec>,
}

/// Build and validate the strict relational binding. The generic entity tables
/// remain the semantic execution source, while these native tables are updated
/// in the same PostgreSQL transaction and are therefore an exact, reopen-safe
/// relational projection of every committed state.
fn relational_collection_specs(
    catalog: &Catalog,
) -> Result<Vec<RelationalCollectionSpec>, DbError> {
    let mut specs = Vec::new();
    for (_, collection) in catalog.collections() {
        if collection.internal || collection.name == DEFAULT_COLLECTION {
            continue;
        }
        if collection.kind != CollectionKind::Schema {
            return Err(relational_invariant(format!(
                "collection '{}' must use schema kind (untyped and polymorphic collections are unsupported)",
                collection.name
            )));
        }
        if collection.integrity_mode != IntegrityMode::StrictRegisteredSchema {
            return Err(relational_invariant(format!(
                "collection '{}' must use strict_registered_schema integrity",
                collection.name
            )));
        }
        let class_ids = catalog.class_ids(&collection.name);
        if class_ids.len() != 1 {
            return Err(relational_invariant(format!(
                "collection '{}' requires exactly one concrete class with the same identity; found {}",
                collection.name,
                class_ids.len()
            )));
        }
        let class = catalog.class_by_lid(class_ids[0]).ok_or_else(|| {
            relational_invariant(format!(
                "collection '{}' class binding does not resolve",
                collection.name
            ))
        })?;
        if class.class.id != collection.name {
            return Err(relational_invariant(format!(
                "collection '{}' resolves through an alias to class '{}'; exact identity matching is required",
                collection.name, class.class.id
            )));
        }

        let mut class_fields = BTreeMap::<String, bool>::new();
        collect_relational_class_fields(
            catalog,
            class.lid,
            &mut BTreeSet::new(),
            &mut class_fields,
        )?;
        let mut fields = Vec::with_capacity(class_fields.len());
        for (canonical, computed) in class_fields {
            let field_id = collection.field_id(&canonical).ok_or_else(|| {
                relational_invariant(format!(
                    "collection '{}' has no stable field mapping for class attribute '{}'",
                    collection.name, canonical
                ))
            })?;
            let ty = collection.field_type(&canonical).ok_or_else(|| {
                relational_invariant(format!(
                    "collection '{}' has no type for class attribute '{}'",
                    collection.name, canonical
                ))
            })?;
            fields.push(RelationalFieldSpec {
                field_id,
                semantic_type: facet_json::to_string(ty)
                    .map_err(|error| DbError::Serialization(error.to_string()))?,
                physical_type: relational_physical_type(ty),
                canonical,
                computed,
            });
        }
        fields.sort_by_key(|field| field.field_id);
        specs.push(RelationalCollectionSpec {
            collection_lid: collection.lid,
            collection_name: collection.name.clone(),
            class_identity: class.class.id.clone(),
            table_name: relational_table_name(collection.lid),
            fields,
        });
    }
    Ok(specs)
}

fn relational_physical_type(ty: &Type) -> RelationalPhysicalType {
    match &ty.kind {
        TypeKind::Optional(optional) => relational_physical_type(&optional.inner),
        TypeKind::Bool(_) => RelationalPhysicalType::Bool,
        TypeKind::Char(_) | TypeKind::String(_) | TypeKind::Ref(_) => RelationalPhysicalType::Text,
        TypeKind::Number(NumberType::Int(width)) => match width {
            IntWidth::I8 | IntWidth::I16 => RelationalPhysicalType::SmallInt,
            IntWidth::I24 | IntWidth::I32 => RelationalPhysicalType::Integer,
            IntWidth::I40 | IntWidth::I48 | IntWidth::I56 | IntWidth::I64 => {
                RelationalPhysicalType::BigInt
            }
            IntWidth::I128 | IntWidth::I256 => RelationalPhysicalType::Numeric,
        },
        TypeKind::Number(NumberType::UInt(width)) => match width {
            UIntWidth::U8 | UIntWidth::U16 | UIntWidth::U24 => RelationalPhysicalType::Integer,
            UIntWidth::U32 => RelationalPhysicalType::BigInt,
            UIntWidth::U40
            | UIntWidth::U48
            | UIntWidth::U56
            | UIntWidth::U64
            | UIntWidth::U128
            | UIntWidth::U256 => RelationalPhysicalType::Numeric,
        },
        TypeKind::Number(NumberType::Float(FloatWidth::F16 | FloatWidth::F32)) => {
            RelationalPhysicalType::Real
        }
        TypeKind::Number(NumberType::Float(FloatWidth::F64)) => RelationalPhysicalType::Double,
        TypeKind::Number(_) => RelationalPhysicalType::TaggedJsonb,
        TypeKind::Uuid => RelationalPhysicalType::Uuid,
        TypeKind::IpAddr(_) => RelationalPhysicalType::Inet,
        TypeKind::Bytes(_) => RelationalPhysicalType::Bytea,
        TypeKind::Temporal(TemporalType::Date) => RelationalPhysicalType::Date,
        TypeKind::Temporal(TemporalType::Time) => RelationalPhysicalType::Time,
        TypeKind::Temporal(TemporalType::DateTime | TemporalType::Timestamp(_)) => {
            RelationalPhysicalType::Timestamptz
        }
        _ => RelationalPhysicalType::TaggedJsonb,
    }
}

fn collect_relational_class_fields(
    catalog: &Catalog,
    class_lid: LocalClassId,
    visited: &mut BTreeSet<LocalClassId>,
    fields: &mut BTreeMap<String, bool>,
) -> Result<(), DbError> {
    if !visited.insert(class_lid) {
        return Ok(());
    }
    let class = catalog.class_by_lid(class_lid).ok_or_else(|| {
        relational_invariant(format!("class local id {} does not resolve", class_lid.0))
    })?;
    if let Some(base) = &class.class.inherits {
        let base_lid = catalog.class_id(&base.id).ok_or_else(|| {
            relational_invariant(format!(
                "class '{}' inherits unknown class '{}'",
                class.class.id, base.id
            ))
        })?;
        collect_relational_class_fields(catalog, base_lid, visited, fields)?;
    }
    for extension in &class.class.extends {
        let extension_lid = catalog.class_id(&extension.id).ok_or_else(|| {
            relational_invariant(format!(
                "class '{}' extends unknown class '{}'",
                class.class.id, extension.id
            ))
        })?;
        collect_relational_class_fields(catalog, extension_lid, visited, fields)?;
    }
    for class_attribute in class.class.attributes.values() {
        let attribute = catalog
            .attribute_by_id(&class_attribute.attribute.id)
            .ok_or_else(|| {
                relational_invariant(format!(
                    "class '{}' references unknown attribute '{}'",
                    class.class.id, class_attribute.attribute.id
                ))
            })?;
        fields.insert(
            attribute.attribute.id.clone(),
            class_attribute.computed.is_some(),
        );
    }
    Ok(())
}

async fn sync_relational_projection(
    transaction: &Transaction<'_>,
    schema: &str,
    schema_name: &str,
    catalog: &Catalog,
    engine: &PostgresSnapshotEngine,
    schema_changed: bool,
) -> Result<(), DbError> {
    let specs = relational_collection_specs(catalog)?;
    let specs_by_lid = specs
        .iter()
        .map(|spec| (spec.collection_lid.0, spec))
        .collect::<BTreeMap<_, _>>();
    let mut entities = BTreeMap::<usize, Vec<StoredEntity>>::new();
    for (key, value) in engine.entries() {
        let Some((collection_lid, _)) = parse_entity_key(key) else {
            continue;
        };
        let entity = decode_entity(value)?;
        let collection = catalog.collection_by_lid(collection_lid).ok_or_else(|| {
            DbError::Deserialization(format!(
                "entity '{}' references unknown collection {}",
                entity.id, collection_lid.0
            ))
        })?;
        if collection.internal {
            continue;
        }
        if collection.name == DEFAULT_COLLECTION {
            return Err(relational_invariant(format!(
                "classless entity '{}' in the default collection is unsupported",
                entity.id
            )));
        }
        let spec = specs_by_lid.get(&collection_lid.0).ok_or_else(|| {
            relational_invariant(format!(
                "entity '{}' belongs to collection '{}' without a strict class binding",
                entity.id, collection.name
            ))
        })?;
        if entity.kind != StoredEntityKind::Class {
            return Err(relational_invariant(format!(
                "entity '{}' in collection '{}' is not a typed class entity",
                entity.id, collection.name
            )));
        }
        let object_type = entity
            .object
            .get(semantic_db_core::catalog::OBJECT_TYPE_FIELD)
            .and_then(semantic_data::value::Value::as_str)
            .ok_or_else(|| {
                relational_invariant(format!(
                    "entity '{}' in collection '{}' requires an explicit string object type",
                    entity.id, collection.name
                ))
            })?;
        if object_type != spec.class_identity {
            return Err(relational_invariant(format!(
                "entity '{}' in collection '{}' has object type '{}', expected '{}'",
                entity.id, collection.name, object_type, spec.class_identity
            )));
        }
        let allowed = spec
            .fields
            .iter()
            .map(|field| field.canonical.as_str())
            .chain([
                semantic_db_core::catalog::PRIMARY_ID_FIELD,
                semantic_db_core::catalog::OBJECT_TYPE_FIELD,
                semantic_db_core::catalog::PARENT_RELATION_FIELD,
            ])
            .collect::<BTreeSet<_>>();
        if let Some(field) = entity
            .object
            .keys()
            .find(|field| !allowed.contains(field.as_str()))
        {
            return Err(relational_invariant(format!(
                "entity '{}' in collection '{}' contains unmapped field '{}'",
                entity.id, collection.name, field
            )));
        }
        if let Some(field) = spec
            .fields
            .iter()
            .find(|field| field.computed && entity.object.contains_key(&field.canonical))
        {
            return Err(relational_invariant(format!(
                "entity '{}' in collection '{}' stores computed field '{}'",
                entity.id, collection.name, field.canonical
            )));
        }
        entities.entry(collection_lid.0).or_default().push(entity);
    }

    reject_lossy_relational_changes(transaction, schema, &specs, &entities).await?;

    let old_tables = transaction
        .query(
            &format!(
                "SELECT physical_table FROM {schema}.collection_map
                 WHERE managed = true AND mapping->>'layout' = 'relational'"
            ),
            &[],
        )
        .await
        .map_err(storage_error)?
        .into_iter()
        .filter_map(|row| row.get::<_, Option<String>>(0))
        .collect::<BTreeSet<_>>();
    let desired_tables = specs
        .iter()
        .map(|spec| spec.table_name.clone())
        .collect::<BTreeSet<_>>();
    for table in old_tables.difference(&desired_tables) {
        transaction
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS {schema}.{}",
                quote_ident(table)
            ))
            .await
            .map_err(storage_error)?;
    }

    let mut desired_indexes = BTreeSet::new();
    for spec in &specs {
        let table = quote_ident(&spec.table_name);
        transaction
            .batch_execute(&format!(
                "CREATE TABLE IF NOT EXISTS {schema}.{table} (
                    _semantic_id text PRIMARY KEY,
                    _semantic_type text NOT NULL,
                    _semantic_parent text
                 )"
            ))
            .await
            .map_err(storage_error)?;
        for field in &spec.fields {
            if field.computed {
                continue;
            }
            transaction
                .batch_execute(&format!(
                    "ALTER TABLE {schema}.{table} ADD COLUMN IF NOT EXISTS {} {}",
                    quote_ident(&relational_field_name(
                        i64::try_from(field.field_id.0).map_err(|_| DbError::Serialization(
                            "field id exceeds PostgreSQL bigint".to_string()
                        ))?
                    )),
                    field.physical_type.sql()
                ))
                .await
                .map_err(storage_error)?;
            let column_name =
                relational_field_name(i64::try_from(field.field_id.0).map_err(|_| {
                    DbError::Serialization("field id exceeds PostgreSQL bigint".to_string())
                })?);
            let actual_type: String = transaction
                .query_one(
                    "SELECT pg_catalog.format_type(a.atttypid, a.atttypmod)
                     FROM pg_catalog.pg_attribute a
                     JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
                     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
                     WHERE n.nspname = $1 AND c.relname = $2 AND a.attname = $3
                       AND a.attnum > 0 AND NOT a.attisdropped",
                    &[&schema_name, &spec.table_name, &column_name],
                )
                .await
                .map_err(storage_error)?
                .get(0);
            if actual_type != field.physical_type.sql() {
                transaction
                    .batch_execute(&format!(
                        "ALTER TABLE {schema}.{table} ALTER COLUMN {} TYPE {} USING NULL",
                        quote_ident(&column_name),
                        field.physical_type.sql()
                    ))
                    .await
                    .map_err(storage_error)?;
            }
        }
        let desired_columns = spec
            .fields
            .iter()
            .filter(|field| !field.computed)
            .map(|field| {
                i64::try_from(field.field_id.0)
                    .map(relational_field_name)
                    .map_err(|_| {
                        DbError::Serialization("field id exceeds PostgreSQL bigint".to_string())
                    })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let existing_columns = transaction
            .query(
                "SELECT column_name FROM information_schema.columns
                 WHERE table_schema = $1 AND table_name = $2
                   AND column_name LIKE 'semantic_f_%'",
                &[&schema_name, &spec.table_name],
            )
            .await
            .map_err(storage_error)?
            .into_iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<BTreeSet<_>>();
        for column in existing_columns.difference(&desired_columns) {
            transaction
                .batch_execute(&format!(
                    "ALTER TABLE {schema}.{table} DROP COLUMN {}",
                    quote_ident(column)
                ))
                .await
                .map_err(storage_error)?;
        }
        let changed_ids: BTreeSet<_> = engine
            .changes()
            .keys()
            .filter_map(|key| parse_entity_key(key))
            .filter(|(collection, _)| *collection == spec.collection_lid)
            .map(|(_, id)| id)
            .collect();
        if schema_changed {
            transaction
                .execute(&format!("DELETE FROM {schema}.{table}"), &[])
                .await
                .map_err(storage_error)?;
        } else {
            // Delete all replaced rows before inserting any, allowing unique-key swaps.
            for id in &changed_ids {
                transaction
                    .execute(
                        &format!("DELETE FROM {schema}.{table} WHERE _semantic_id = $1"),
                        &[id],
                    )
                    .await
                    .map_err(storage_error)?;
            }
        }
        for entity in entities.get(&spec.collection_lid.0).into_iter().flatten() {
            if !schema_changed && !changed_ids.contains(entity.id.as_str()) {
                continue;
            }
            let object_type = entity
                .object
                .get(semantic_db_core::catalog::OBJECT_TYPE_FIELD)
                .and_then(semantic_data::value::Value::as_str)
                .expect("relational entity type was validated");
            let parent = entity
                .object
                .get(semantic_db_core::catalog::PARENT_RELATION_FIELD)
                .and_then(semantic_data::value::Value::as_str);
            transaction
                .execute(
                    &format!(
                        "INSERT INTO {schema}.{table}
                         (_semantic_id, _semantic_type, _semantic_parent) VALUES ($1, $2, $3)"
                    ),
                    &[&entity.id, &object_type, &parent],
                )
                .await
                .map_err(storage_error)?;
            for field in &spec.fields {
                if field.computed {
                    continue;
                }
                let Some(value) = entity.object.get(&field.canonical) else {
                    continue;
                };
                let column =
                    relational_field_name(i64::try_from(field.field_id.0).map_err(|_| {
                        DbError::Serialization("field id exceeds PostgreSQL bigint".to_string())
                    })?);
                write_relational_field(
                    transaction,
                    schema,
                    &table,
                    &column,
                    &entity.id,
                    value,
                    field.physical_type,
                )
                .await?;
            }
        }

        for index in catalog.indexes_for_collection(spec.collection_lid) {
            let column = match index.canonical_field.as_str() {
                semantic_db_core::catalog::PRIMARY_ID_FIELD => "_semantic_id".to_string(),
                semantic_db_core::catalog::OBJECT_TYPE_FIELD => "_semantic_type".to_string(),
                semantic_db_core::catalog::PARENT_RELATION_FIELD => "_semantic_parent".to_string(),
                _ => {
                    let Some(field_id) = index.field_id else {
                        continue;
                    };
                    let Some(field) = spec.fields.iter().find(|field| field.field_id == field_id)
                    else {
                        return Err(relational_invariant(format!(
                            "index '{}' targets field '{}' outside class '{}'",
                            index.schema.id, index.canonical_field, spec.class_identity
                        )));
                    };
                    if field.computed {
                        return Err(relational_invariant(format!(
                            "index '{}' targets computed field '{}' which has no stored column",
                            index.schema.id, field.canonical
                        )));
                    }
                    relational_field_name(i64::try_from(field_id.0).map_err(|_| {
                        DbError::Serialization("field id exceeds PostgreSQL bigint".to_string())
                    })?)
                }
            };
            let index_name = format!("semantic_i_{}", index.lid.0);
            desired_indexes.insert(index_name.clone());
            let unique = if index.schema.unique { "UNIQUE " } else { "" };
            let index_target = match index.schema.kind {
                IndexKind::FullText => {
                    if field_physical_type(spec, &index.canonical_field)
                        != Some(RelationalPhysicalType::Text)
                    {
                        return Err(relational_invariant(format!(
                            "full-text index '{}' requires a text field",
                            index.schema.id
                        )));
                    }
                    format!(
                        "USING GIN (to_tsvector('simple'::regconfig, coalesce({}, '')))",
                        quote_ident(&column)
                    )
                }
                IndexKind::Equality | IndexKind::Range => format!("({})", quote_ident(&column)),
                IndexKind::PathEquality => continue,
            };
            transaction
                .batch_execute(&format!(
                    "CREATE {unique}INDEX IF NOT EXISTS {} ON {schema}.{table} {index_target}",
                    quote_ident(&index_name),
                ))
                .await
                .map_err(storage_error)?;
        }
    }
    let existing_indexes = transaction
        .query(
            "SELECT indexname FROM pg_catalog.pg_indexes
             WHERE schemaname = $1 AND indexname LIKE 'semantic_i_%'",
            &[&schema_name],
        )
        .await
        .map_err(storage_error)?
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<BTreeSet<_>>();
    for index in existing_indexes.difference(&desired_indexes) {
        transaction
            .batch_execute(&format!(
                "DROP INDEX IF EXISTS {schema}.{}",
                quote_ident(index)
            ))
            .await
            .map_err(storage_error)?;
    }
    Ok(())
}

/// Verify that backend-owned native tables still match the canonical committed
/// entities. Managed relational tables are intentionally not an out-of-band
/// write API: drift is rejected before reads or writes instead of being ignored.
pub(crate) async fn verify_relational_projection(
    transaction: &Transaction<'_>,
    schema_name: &str,
    catalog: &Catalog,
) -> Result<(), DbError> {
    let schema = quote_ident(schema_name);
    for spec in relational_collection_specs(catalog)? {
        let canonical_rows = transaction
            .query(
                &format!(
                    "SELECT entity_id, document FROM {schema}.entities WHERE collection_lid = $1"
                ),
                &[&(i64::try_from(spec.collection_lid.0).map_err(|_| {
                    DbError::Serialization("collection id exceeds PostgreSQL bigint".to_string())
                })?)],
            )
            .await
            .map_err(storage_error)?;
        let mut canonical = BTreeMap::new();
        for row in canonical_rows {
            let id: String = row.get(0);
            let document: serde_json::Value = row.get(1);
            canonical.insert(
                id.clone(),
                decode_stored_document(spec.collection_lid.0, id, &document)?,
            );
        }
        let table = quote_ident(&spec.table_name);
        let native_rows = transaction
            .query(
                &format!(
                    "SELECT _semantic_id, _semantic_type, _semantic_parent FROM {schema}.{table}"
                ),
                &[],
            )
            .await
            .map_err(storage_error)?;
        if native_rows.len() != canonical.len() {
            return Err(relational_drift(format!(
                "collection '{}' has {} canonical rows but {} native rows",
                spec.collection_name,
                canonical.len(),
                native_rows.len()
            )));
        }
        for row in native_rows {
            let id: String = row.get(0);
            let object_type: String = row.get(1);
            let parent: Option<String> = row.get(2);
            let entity = canonical.get(&id).ok_or_else(|| {
                relational_drift(format!(
                    "native table '{}' contains unknown row '{}'",
                    spec.table_name, id
                ))
            })?;
            let expected_type = entity
                .object
                .get(semantic_db_core::catalog::OBJECT_TYPE_FIELD)
                .and_then(Value::as_str);
            let expected_parent = entity
                .object
                .get(semantic_db_core::catalog::PARENT_RELATION_FIELD)
                .and_then(Value::as_str);
            if expected_type != Some(object_type.as_str()) || expected_parent != parent.as_deref() {
                return Err(relational_drift(format!(
                    "native identity metadata for '{}:{}' differs from canonical storage",
                    spec.collection_name, id
                )));
            }
            for field in spec.fields.iter().filter(|field| !field.computed) {
                let column =
                    relational_field_name(i64::try_from(field.field_id.0).map_err(|_| {
                        DbError::Serialization("field id exceeds PostgreSQL bigint".to_string())
                    })?);
                let expected = entity.object.get(&field.canonical);
                let matches =
                    if expected.is_none_or(|value| matches!(value, Value::Null | Value::Void)) {
                        transaction
                        .query_one(
                            &format!(
                                "SELECT {} IS NULL FROM {schema}.{table} WHERE _semantic_id = $1",
                                quote_ident(&column)
                            ),
                            &[&id],
                        )
                        .await
                        .map_err(storage_error)?
                        .get::<_, bool>(0)
                    } else if field.physical_type == RelationalPhysicalType::TaggedJsonb {
                        let document = encode_value(expected.expect("non-null value was checked"));
                        transaction
                            .query_one(
                                &format!(
                                    "SELECT {} IS NOT DISTINCT FROM $2::jsonb
                                 FROM {schema}.{table} WHERE _semantic_id = $1",
                                    quote_ident(&column)
                                ),
                                &[&id, &document],
                            )
                            .await
                            .map_err(storage_error)?
                            .get::<_, bool>(0)
                    } else {
                        let encoded = encode_native_scalar(
                            expected.expect("non-null value was checked"),
                            field.physical_type,
                        )?;
                        let expression =
                            native_parameter_expression(field.physical_type).replace("$1", "$2");
                        transaction
                            .query_one(
                                &format!(
                                    "SELECT {} IS NOT DISTINCT FROM {expression}
                                 FROM {schema}.{table} WHERE _semantic_id = $1",
                                    quote_ident(&column)
                                ),
                                &[&id, &encoded],
                            )
                            .await
                            .map_err(storage_error)?
                            .get::<_, bool>(0)
                    };
                if !matches {
                    return Err(relational_drift(format!(
                        "native field '{}' for '{}:{}' differs from canonical storage",
                        field.canonical, spec.collection_name, id
                    )));
                }
            }
        }
    }
    Ok(())
}

fn relational_drift(message: String) -> DbError {
    DbError::InvalidQuery(format!(
        "managed relational native table drift detected: {message}; direct writes are unsupported"
    ))
}

fn field_physical_type(
    spec: &RelationalCollectionSpec,
    canonical: &str,
) -> Option<RelationalPhysicalType> {
    match canonical {
        semantic_db_core::catalog::PRIMARY_ID_FIELD
        | semantic_db_core::catalog::OBJECT_TYPE_FIELD
        | semantic_db_core::catalog::PARENT_RELATION_FIELD => Some(RelationalPhysicalType::Text),
        _ => spec
            .fields
            .iter()
            .find(|field| field.canonical == canonical)
            .map(|field| field.physical_type),
    }
}

async fn reject_lossy_relational_changes(
    transaction: &Transaction<'_>,
    schema: &str,
    specs: &[RelationalCollectionSpec],
    entities: &BTreeMap<usize, Vec<StoredEntity>>,
) -> Result<(), DbError> {
    for spec in specs {
        if entities
            .get(&spec.collection_lid.0)
            .is_none_or(Vec::is_empty)
        {
            continue;
        }
        let old_fields = transaction
            .query(
                &format!(
                    "SELECT field_id, canonical_field, mapping FROM {schema}.field_map
                     WHERE collection_lid = $1 AND physical_column IS NOT NULL"
                ),
                &[&(i64::try_from(spec.collection_lid.0).map_err(|_| {
                    DbError::Serialization("collection id exceeds PostgreSQL bigint".to_string())
                })?)],
            )
            .await
            .map_err(storage_error)?;
        let desired = spec
            .fields
            .iter()
            .filter(|field| !field.computed)
            .map(|field| {
                (
                    field.field_id.0,
                    (field.canonical.as_str(), field.semantic_type.as_str()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for row in old_fields {
            let field_id: i64 = row.get(0);
            let canonical: String = row.get(1);
            let mapping: serde_json::Value = row.get(2);
            let field_id = usize::try_from(field_id).map_err(|_| {
                DbError::Deserialization("negative relational field id".to_string())
            })?;
            let old_type = mapping
                .get("semantic_type")
                .and_then(serde_json::Value::as_str);
            let desired_field = desired.get(&field_id).copied();
            if desired_field.map(|field| field.0) != Some(canonical.as_str()) {
                return Err(relational_invariant(format!(
                    "schema change would remove or remap stored field '{}' in non-empty collection '{}'",
                    canonical, spec.collection_name
                )));
            }
            if old_type.is_some() && old_type != desired_field.map(|field| field.1) {
                return Err(relational_invariant(format!(
                    "schema change would alter the type of stored field '{}' in non-empty collection '{}'",
                    canonical, spec.collection_name
                )));
            }
        }
    }
    Ok(())
}

async fn write_relational_field(
    transaction: &Transaction<'_>,
    schema: &str,
    quoted_table: &str,
    column: &str,
    entity_id: &str,
    value: &Value,
    physical_type: RelationalPhysicalType,
) -> Result<(), DbError> {
    let column = quote_ident(column);
    if matches!(value, Value::Null | Value::Void) {
        transaction
            .execute(
                &format!(
                    "UPDATE {schema}.{quoted_table} SET {column} = NULL WHERE _semantic_id = $1"
                ),
                &[&entity_id],
            )
            .await
            .map_err(storage_error)?;
        return Ok(());
    }
    if physical_type == RelationalPhysicalType::TaggedJsonb {
        let document = encode_value(value);
        transaction
            .execute(
                &format!(
                    "UPDATE {schema}.{quoted_table} SET {column} = $1 WHERE _semantic_id = $2"
                ),
                &[&document, &entity_id],
            )
            .await
            .map_err(storage_error)?;
        return Ok(());
    }
    let encoded = encode_native_scalar(value, physical_type)?;
    let expression = native_parameter_expression(physical_type);
    transaction
        .execute(
            &format!(
                "UPDATE {schema}.{quoted_table} SET {column} = {expression} WHERE _semantic_id = $2"
            ),
            &[&encoded, &entity_id],
        )
        .await
        .map_err(storage_error)?;
    Ok(())
}

fn native_parameter_expression(physical_type: RelationalPhysicalType) -> &'static str {
    match physical_type {
        RelationalPhysicalType::Bool => "$1::text::boolean",
        RelationalPhysicalType::SmallInt => "$1::text::smallint",
        RelationalPhysicalType::Integer => "$1::text::integer",
        RelationalPhysicalType::BigInt => "$1::text::bigint",
        RelationalPhysicalType::Numeric => "$1::text::numeric",
        RelationalPhysicalType::Real => "$1::text::real",
        RelationalPhysicalType::Double => "$1::text::double precision",
        RelationalPhysicalType::Text => "$1::text",
        RelationalPhysicalType::Uuid => "$1::text::uuid",
        RelationalPhysicalType::Inet => "$1::text::inet",
        RelationalPhysicalType::Bytea => "decode($1::text, 'hex')",
        RelationalPhysicalType::Date => "$1::text::date",
        RelationalPhysicalType::Time => "$1::text::time without time zone",
        RelationalPhysicalType::Timestamptz => "$1::text::timestamp with time zone",
        RelationalPhysicalType::TaggedJsonb => "$1::jsonb",
    }
}

fn encode_native_scalar(
    value: &Value,
    physical_type: RelationalPhysicalType,
) -> Result<String, DbError> {
    let encoded = match physical_type {
        RelationalPhysicalType::Bool => match value {
            Value::Bool(value) => value.to_string(),
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::SmallInt
        | RelationalPhysicalType::Integer
        | RelationalPhysicalType::BigInt
        | RelationalPhysicalType::Numeric => match value {
            Value::I8(value) => value.to_string(),
            Value::I16(value) => value.to_string(),
            Value::I32(value) => value.to_string(),
            Value::I64(value) => value.to_string(),
            Value::I128(value) => value.to_string(),
            Value::U8(value) => value.to_string(),
            Value::U16(value) => value.to_string(),
            Value::U32(value) => value.to_string(),
            Value::U64(value) => value.to_string(),
            Value::U128(value) => value.to_string(),
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Real => match value {
            Value::F32(value) => postgres_float(value.into_inner() as f64),
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Double => match value {
            Value::F32(value) => postgres_float(value.into_inner() as f64),
            Value::F64(value) => postgres_float(value.into_inner()),
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Text => match value {
            Value::String(value) => value.clone(),
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Uuid => match value {
            Value::Uuid(value) => {
                let value: uuid::Uuid = (*value).into();
                value.to_string()
            }
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Inet => match value {
            Value::IpAddr(value) => value.to_string(),
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Bytea => match value {
            Value::Bytes(value) => value.iter().map(|byte| format!("{byte:02x}")).collect(),
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Date => match value {
            Value::Date(value) => {
                let value: time::Date = (*value).into();
                value.to_string()
            }
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Time => match value {
            Value::Time(value) => {
                let value: time::Time = (*value).into();
                value.to_string()
            }
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::Timestamptz => match value {
            Value::DateTime(value) => {
                let value: time::OffsetDateTime = (*value).into();
                value.to_string()
            }
            _ => return native_value_mismatch(value, physical_type),
        },
        RelationalPhysicalType::TaggedJsonb => {
            return Err(DbError::Serialization(
                "tagged jsonb value was passed to native scalar encoder".to_string(),
            ));
        }
    };
    Ok(encoded)
}

fn postgres_float(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value == f64::INFINITY {
        "Infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        value.to_string()
    }
}

fn native_value_mismatch<T>(
    value: &Value,
    physical_type: RelationalPhysicalType,
) -> Result<T, DbError> {
    Err(relational_invariant(format!(
        "value variant {value:?} cannot be stored losslessly as {}",
        physical_type.sql()
    )))
}

fn relational_table_name(collection: LocalCollectionId) -> String {
    format!("semantic_c_{}", collection.0)
}

fn relational_field_name(field_id: i64) -> String {
    format!("semantic_f_{field_id}")
}

fn relational_invariant(message: String) -> DbError {
    DbError::InvalidQuery(format!("managed relational invariant: {message}"))
}

fn encode_stored_document(entity: &StoredEntity) -> serde_json::Value {
    let mut document = encode_object(&entity.object);
    document["kind"] = serde_json::Value::String(
        match entity.kind {
            StoredEntityKind::Untyped => "untyped",
            StoredEntityKind::Record => "record",
            StoredEntityKind::Class => "class",
        }
        .to_string(),
    );
    document
}

fn decode_stored_document(
    collection: usize,
    id: String,
    document: &serde_json::Value,
) -> Result<StoredEntity, DbError> {
    let kind = document
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| DbError::Deserialization("entity document is missing kind".to_string()))?;
    let kind = match kind {
        "untyped" => StoredEntityKind::Untyped,
        "record" => StoredEntityKind::Record,
        "class" => StoredEntityKind::Class,
        _ => {
            return Err(DbError::Deserialization(format!(
                "unknown entity kind '{kind}'"
            )));
        }
    };
    Ok(StoredEntity {
        id,
        collection,
        kind,
        object: decode_object(document)?,
    })
}

async fn rebuild_relation_edges(
    transaction: &Transaction<'_>,
    schema: &str,
    catalog: &semantic_db_core::catalog::Catalog,
    engine: &PostgresSnapshotEngine,
) -> Result<(), DbError> {
    let mut previous = BTreeMap::new();
    for row in transaction
        .query(
            &format!(
                "SELECT relation_lid, source_collection_lid, source_id,
        target_collection_lid, target_id, depth FROM {schema}.relation_edges"
            ),
            &[],
        )
        .await
        .map_err(storage_error)?
    {
        previous.insert(
            (
                row.get::<_, i64>(0),
                row.get::<_, i64>(1),
                row.get::<_, String>(2),
                row.get::<_, i64>(3),
                row.get::<_, String>(4),
            ),
            row.get::<_, i64>(5),
        );
    }
    let Some(edge_collection) = catalog.collection_by_name("__semantic.relationship_edges") else {
        return Ok(());
    };
    let insert = transaction
        .prepare(&format!(
            "INSERT INTO {schema}.relation_edges
             (relation_lid, source_collection_lid, source_id,
              target_collection_lid, target_id, depth, edge_collection_lid, edge_entity_id)
                 VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL)
                 ON CONFLICT (relation_lid, source_collection_lid, source_id, target_collection_lid, target_id)
                 DO UPDATE SET depth = EXCLUDED.depth"
        ))
        .await
        .map_err(storage_error)?;
    for (key, payload) in engine.entries() {
        let Some((collection, _)) = parse_entity_key(key) else {
            continue;
        };
        if collection != edge_collection.lid {
            continue;
        }
        let entity = decode_entity(payload)?;
        let Some(relation_id) = entity
            .object
            .get("relation")
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        let relationship = catalog.relationship_by_id(relation_id).ok_or_else(|| {
            DbError::InvalidQuery(format!(
                "relationship edge references unknown '{relation_id}'"
            ))
        })?;
        let source_collection = catalog
            .collection_by_name(&relationship.relationship.source_collection)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: relationship.relationship.source_collection.clone(),
            })?;
        let target_collection = source_collection;
        let source_id = entity
            .object
            .get("source")
            .and_then(|value| value.as_str())
            .ok_or_else(|| DbError::Deserialization("edge is missing source".to_string()))?;
        let target_id = entity
            .object
            .get("target")
            .and_then(|value| value.as_str())
            .ok_or_else(|| DbError::Deserialization("edge is missing target".to_string()))?;
        let depth = entity
            .object
            .get("depth")
            .and_then(|value| match value {
                semantic_data::value::Value::U64(value) => i64::try_from(*value).ok(),
                _ => None,
            })
            .unwrap_or(1);
        let relation_lid = i64::try_from(relationship.lid.0)
            .map_err(|_| DbError::Serialization("relation lid exceeds bigint".to_string()))?;
        let source_lid = i64::try_from(source_collection.lid.0)
            .map_err(|_| DbError::Serialization("collection lid exceeds bigint".to_string()))?;
        let target_lid = i64::try_from(target_collection.lid.0)
            .map_err(|_| DbError::Serialization("collection lid exceeds bigint".to_string()))?;
        if previous.remove(&(
            relation_lid,
            source_lid,
            source_id.to_string(),
            target_lid,
            target_id.to_string(),
        )) == Some(depth)
        {
            continue;
        }
        transaction
            .execute(
                &insert,
                &[
                    &relation_lid,
                    &source_lid,
                    &source_id,
                    &target_lid,
                    &target_id,
                    &depth,
                ],
            )
            .await
            .map_err(storage_error)?;
    }
    for ((relation, source_collection, source, target_collection, target), _) in previous {
        transaction
            .execute(
                &format!(
                    "DELETE FROM {schema}.relation_edges
            WHERE relation_lid = $1 AND source_collection_lid = $2 AND source_id = $3
              AND target_collection_lid = $4 AND target_id = $5"
                ),
                &[
                    &relation,
                    &source_collection,
                    &source,
                    &target_collection,
                    &target,
                ],
            )
            .await
            .map_err(storage_error)?;
    }
    Ok(())
}

pub(crate) fn storage_error(error: tokio_postgres::Error) -> DbError {
    if let Some(code) = error.code() {
        return match code.code() {
            "40001" | "40P01" => DbError::TransactionConflict(code.code().to_string()),
            "23505" | "23503" | "23502" | "23514" => DbError::InvalidQuery(error.to_string()),
            _ => DbError::Storage(error.to_string()),
        };
    }
    DbError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::schema::{BoolType, BytesEncoding, BytesType, IpAddrType, StringType};
    use semantic_data::value::{Object, Value};

    #[test]
    fn snapshot_engine_conditional_updates_are_atomic() {
        let mut engine = PostgresSnapshotEngine::default();
        let committed = engine
            .write_batch_conditional(
                &[KvWriteOp::Put {
                    key: b"a".to_vec(),
                    value: b"b".to_vec(),
                }],
                Some(0),
            )
            .unwrap();
        assert!(matches!(committed, StorageCommitOutcome::Committed { .. }));
        let conflict = engine.write_batch_conditional(&[], Some(0)).unwrap();
        assert!(matches!(conflict, StorageCommitOutcome::Conflict { .. }));
    }

    #[test]
    fn snapshot_engine_tracks_only_changed_keys() {
        let mut initial = BTreeMap::new();
        initial.insert(b"untouched".to_vec(), b"old".to_vec());
        initial.insert(b"deleted".to_vec(), b"old".to_vec());
        let mut engine = PostgresSnapshotEngine::from_entries(initial, 7);

        engine.put(b"changed".to_vec(), b"new".to_vec()).unwrap();
        engine.delete(b"deleted").unwrap();

        assert_eq!(engine.changes().len(), 2);
        assert!(!engine.changes().contains_key(b"untouched".as_slice()));
        assert_eq!(
            engine.changes().get(b"changed".as_slice()),
            Some(&Some(b"new".to_vec()))
        );
        assert_eq!(engine.changes().get(b"deleted".as_slice()), Some(&None));
    }

    #[test]
    fn stored_entity_document_round_trips_kind_and_object() {
        for kind in [
            StoredEntityKind::Untyped,
            StoredEntityKind::Record,
            StoredEntityKind::Class,
        ] {
            let entity = StoredEntity {
                id: "entity".into(),
                collection: 42,
                kind,
                object: Object::from_iter([
                    ("wide".into(), Value::U128(u128::MAX)),
                    ("void".into(), Value::Void),
                ]),
            };
            let document = encode_stored_document(&entity);
            assert_eq!(
                decode_stored_document(42, "entity".into(), &document).unwrap(),
                entity
            );
        }
    }

    #[test]
    fn stored_entity_document_rejects_missing_kind() {
        let document = encode_object(&Object::new());
        assert!(decode_stored_document(1, "id".into(), &document).is_err());
    }

    #[test]
    fn relational_scalar_types_use_native_postgresql_columns() {
        let ty = |kind| Type {
            kind,
            constraints: vec![],
            annotations: vec![],
        };
        assert_eq!(
            relational_physical_type(&ty(TypeKind::Bool(BoolType))),
            RelationalPhysicalType::Bool
        );
        assert_eq!(
            relational_physical_type(&ty(TypeKind::Number(NumberType::Int(IntWidth::I32)))),
            RelationalPhysicalType::Integer
        );
        assert_eq!(
            relational_physical_type(&ty(TypeKind::Number(NumberType::UInt(UIntWidth::U128)))),
            RelationalPhysicalType::Numeric
        );
        assert_eq!(
            relational_physical_type(&ty(TypeKind::String(StringType {
                format: None,
                normalization: None,
            }))),
            RelationalPhysicalType::Text
        );
        assert_eq!(
            relational_physical_type(&ty(TypeKind::Uuid)),
            RelationalPhysicalType::Uuid
        );
        assert_eq!(
            relational_physical_type(&ty(TypeKind::IpAddr(IpAddrType::Any))),
            RelationalPhysicalType::Inet
        );
        assert_eq!(
            relational_physical_type(&ty(TypeKind::Bytes(BytesType {
                encoding: Some(BytesEncoding::Base64),
            }))),
            RelationalPhysicalType::Bytea
        );
        assert_eq!(
            relational_physical_type(&ty(TypeKind::Temporal(TemporalType::Date))),
            RelationalPhysicalType::Date
        );
    }
}
