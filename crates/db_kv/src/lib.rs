pub mod storage;

pub use storage::{
    BoxKvPrefixScan, EntityScan, EntityStore, IndexEntityIdScan, KvEngine, KvKeyScan, KvScanItem,
    KvWriteOp, MemoryKvEngine, decode_entity, encode_entity, parse_entity_key,
};

use semantic_db_core::embedded::{EmbeddedBackend, EmbeddedDb};
use semantic_db_core::{DbConfig, DbError};

pub type MemoryDatabase = EmbeddedDb<EntityStore<MemoryKvEngine>>;
pub type MemoryBackend = EmbeddedBackend<EntityStore<MemoryKvEngine>>;

pub fn open_memory() -> std::result::Result<MemoryDatabase, DbError> {
    open_memory_with_config(DbConfig::default())
}

pub fn open_memory_with_config(config: DbConfig) -> std::result::Result<MemoryDatabase, DbError> {
    EmbeddedDb::open_with_config(EntityStore::new(MemoryKvEngine::new()), config)
}

pub fn open_memory_mvcc() -> std::result::Result<MemoryDatabase, DbError> {
    EmbeddedDb::open(EntityStore::new(MemoryKvEngine::with_mvcc(true)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::query::BinaryOp;
    use semantic_data::schema::{
        Meta, Migration, MigrationCollectionKind, MigrationDdlOperation, MigrationIntegrityMode,
        MigrationOperation, Module, Package,
    };
    use semantic_data::value::{FieldPath, Object, Value};
    use semantic_db_core::{Expr, Operand, SelectQuery};

    use super::open_memory;

    #[tokio::test(flavor = "multi_thread")]
    async fn memory_backend_testsuite() {
        use semantic_db_core::Db;

        let backend =
            semantic_db_core::embedded::EmbeddedBackend::new(super::open_memory().unwrap());
        semantic_db_test::suite::test_db(&Db::new(backend)).await;
    }

    #[test]
    fn package_migration_indexes_post_migration_rows() {
        let mut seed = Object::new();
        seed.insert("id", Value::String("seed".to_string()));
        seed.insert("kind", Value::String("music".to_string()));
        let package = Package {
            name: "test.migration_index".to_string(),
            root: Module {
                name: "test".to_string(),
                constants: BTreeMap::new(),
                types: BTreeMap::new(),
                attributes: BTreeMap::new(),
                classes: BTreeMap::new(),
                interfaces: BTreeMap::new(),
                contracts: BTreeMap::new(),
                meta: Meta::default(),
            },
            modules: BTreeMap::new(),
            migrations: vec![Migration {
                module: "test".to_string(),
                name: "001_seed".to_string(),
                description: None,
                operations: vec![
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
                        name: "migration_items".to_string(),
                        kind: MigrationCollectionKind::Untyped,
                        integrity_mode: MigrationIntegrityMode::Permissive,
                    }),
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertIndex {
                        name: "by_kind".to_string(),
                        collection: "migration_items".to_string(),
                        field: "kind".to_string(),
                        unique: false,
                    }),
                    MigrationOperation::Insert {
                        collection: "migration_items".to_string(),
                        id: "seed".to_string(),
                        object: seed,
                    },
                ],
                meta: Meta::default(),
            }],
            version: None,
            meta: Meta::default(),
        };
        let mut db = open_memory().unwrap();

        db.upsert_package(package).unwrap();

        assert!(db.get("migration_items", "seed").unwrap().is_some());
        let predicate = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "kind",
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                "music".to_string(),
            )))),
        };
        let rows = db
            .select(
                SelectQuery::new()
                    .with_collection("migration_items")
                    .with_predicate(predicate),
            )
            .unwrap();
        assert_eq!(rows.len(), 1);
    }
}
