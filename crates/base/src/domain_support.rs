use crate::labels::LabelStore;
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    query::{Batch, BatchOperation},
    value::{FromValue, Object, Value},
};
use semantic_rpc_core::RpcError;
pub(crate) fn error(message: impl Into<String>) -> RpcError {
    RpcError::new("invalid_input", message)
}
pub(crate) fn read<T: FromValue>(object: &Object, key: &str) -> Result<T, RpcError> {
    T::from_value(
        object
            .get(key)
            .cloned()
            .ok_or_else(|| error(format!("Missing {key}")))?,
    )
    .map_err(|e| error(e.to_string()))
}
pub(crate) fn optional<T: FromValue>(object: &Object, key: &str) -> Result<Option<T>, RpcError> {
    object
        .get(key)
        .filter(|v| !matches!(v, Value::Null))
        .cloned()
        .map(T::from_value)
        .transpose()
        .map_err(|e| error(e.to_string()))
}
pub(crate) fn upsert(id: String, object: Object) -> BatchOperation {
    BatchOperation::Upsert {
        collection: DEFAULT_COLLECTION.into(),
        id,
        object,
    }
}
pub(crate) async fn persist(
    store: &impl LabelStore,
    id: String,
    object: Object,
) -> Result<(), RpcError> {
    let mut batch = Batch::new();
    batch.operations.push(upsert(id, object));
    store.commit(batch).await
}
pub(crate) fn new_id(prefix: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}-{now:x}-{:x}",
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
pub(crate) fn class(
    id: &str,
    name: &str,
    attrs: &[(&str, &str, bool)],
) -> semantic_data::schema::ClassType {
    use semantic_data::schema::*;
    ClassType {
        id: id.into(),
        name: name.into(),
        inherits: None,
        extends: vec![],
        strict_schema: false,
        creatable_in_ui: Some(false),
        include_in_ui_listings: None,
        attributes: attrs
            .iter()
            .enumerate()
            .map(|(i, (name, id, required))| {
                (
                    (*name).into(),
                    crate::schema::common::helpers::class_attribute_with_ui_order(
                        id,
                        *required,
                        Some(i as u32 * 10),
                    ),
                )
            })
            .collect(),
        constraints: vec![],
        meta: Meta::default(),
    }
}
pub(crate) fn package(
    name: &str,
    module: &str,
    attributes: Vec<semantic_data::schema::AttributeType>,
    classes: Vec<semantic_data::schema::ClassType>,
    migration: semantic_data::schema::Migration,
) -> semantic_data::schema::Package {
    use semantic_data::schema::*;
    Package {
        name: name.into(),
        root: Module {
            name: module.into(),
            attributes: attributes.into_iter().map(|a| (a.id.clone(), a)).collect(),
            classes: classes.into_iter().map(|c| (c.id.clone(), c)).collect(),
            constants: Default::default(),
            types: Default::default(),
            interfaces: Default::default(),
            contracts: Default::default(),
            meta: Meta::default(),
        },
        modules: Default::default(),
        migrations: vec![migration],
        version: None,
        meta: Meta::default(),
    }
}

pub(crate) fn with_content_dependency(
    mut package: semantic_data::schema::Package,
) -> semantic_data::schema::Package {
    use semantic_data::schema::*;
    let attributes = crate::migrations::content_dependency_attributes();
    let module = Module {
        name: "base".into(),
        constants: Default::default(),
        types: Default::default(),
        attributes: attributes
            .iter()
            .cloned()
            .map(|a| (a.id.clone(), a))
            .collect(),
        classes: Default::default(),
        interfaces: Default::default(),
        contracts: Default::default(),
        meta: Meta::default(),
    };
    package.modules.insert("base".into(), module);
    package.migrations.insert(
        0,
        Migration {
            module: "base".into(),
            name: "001_embedded_content_dependency".into(),
            description: Some("Declare shared embedded content schema dependency".into()),
            operations: attributes
                .into_iter()
                .map(|attribute| {
                    MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute })
                })
                .collect(),
            meta: Meta::default(),
        },
    );
    package
}
