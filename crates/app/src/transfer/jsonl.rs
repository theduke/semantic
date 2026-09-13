use std::collections::BTreeSet;
use std::io::{BufRead as _, Read as _};
use std::path::Path;

use futures_util::StreamExt as _;
use semantic_data::attr::{
    ATTR_RELATION_FROM, ATTR_RELATION_RELATION, ATTR_RELATION_TO, RELATION_CLASS_ID,
};
use semantic_data::builtin::{ATTR_ID, ATTR_TYPE};
use semantic_data::schema::RelationMode;
use semantic_data::value::Value;
use semantic_data::value::serde::typed::{TypedRef, TypedValue};
use semantic_db_core::catalog::{Catalog, LocalClassId};
use semantic_db_core::{Batch, BatchOperation, BatchReply, BatchReturn, EntityRecord};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt as _, AsyncWrite, AsyncWriteExt as _};

use super::{ImportOptions, TransferError, TransferStats};

#[derive(Serialize)]
struct EnvelopeRef<'a> {
    version: u32,
    collection: &'a str,
    id: &'a str,
    object: TypedRef<'a>,
}

#[derive(Deserialize)]
struct Envelope {
    version: u32,
    collection: String,
    id: String,
    object: TypedValue,
}

pub(crate) fn encode_record(record: &EntityRecord) -> Result<Vec<u8>, TransferError> {
    validate_finite(&Value::Object(record.object.clone()))?;
    let object = Value::Object(record.object.clone());
    serde_json::to_vec(&EnvelopeRef {
        version: 1,
        collection: &record.collection,
        id: &record.id,
        object: TypedRef(&object),
    })
    .map_err(|error| TransferError::Json {
        source_name: "database entity".into(),
        line: 0,
        message: error.to_string(),
    })
}

pub(crate) fn decode_line(
    source_name: &str,
    line: u64,
    bytes: &[u8],
) -> Result<EntityRecord, TransferError> {
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|error| TransferError::Json {
            source_name: source_name.into(),
            line,
            message: error.to_string(),
        })?;
    if envelope.version != 1 {
        return Err(TransferError::InvalidRecord {
            source_name: source_name.into(),
            line,
            message: format!("unsupported transfer version {}", envelope.version),
        });
    }
    if envelope.collection.is_empty() {
        return Err(invalid(source_name, line, "collection must not be empty"));
    }
    if envelope.id.is_empty() {
        return Err(invalid(source_name, line, "id must not be empty"));
    }
    let Value::Object(object) = envelope.object.0 else {
        return Err(invalid(
            source_name,
            line,
            "object must be a typed object value",
        ));
    };
    match object.get(ATTR_ID).and_then(Value::as_str) {
        Some(id) if id == envelope.id => {}
        Some(id) => {
            return Err(invalid(
                source_name,
                line,
                format!(
                    "envelope id '{}' does not match object id '{id}'",
                    envelope.id
                ),
            ));
        }
        None => return Err(invalid(source_name, line, "object has no string id field")),
    }
    Ok(EntityRecord {
        id: envelope.id,
        collection: envelope.collection,
        object,
    })
}

pub(crate) async fn export_jsonl<W, F>(
    db: &dyn crate::SemanticDb,
    writer: &mut W,
    temp_dir: Option<&Path>,
    mut observe: F,
) -> Result<TransferStats, TransferError>
where
    W: AsyncWrite + Unpin,
    F: FnMut(&EntityRecord) -> Result<(), TransferError>,
{
    let catalog = db.catalog().await?;
    let relation_classes = relation_classes(&catalog);
    let temp = super::make_temp_dir(temp_dir)?;
    let relations_path = temp.path().join("relations.jsonl");
    let mut relations = tokio::io::BufWriter::new(tokio::fs::File::create(&relations_path).await?);
    let mut stream = db.scan_entities().await?;
    let mut stats = TransferStats::default();
    while let Some(record) = stream.next().await {
        let record = record?;
        observe(&record)?;
        let line = encode_record(&record)?;
        if is_relation(&catalog, &relation_classes, &record) {
            relations.write_all(&line).await?;
            relations.write_all(b"\n").await?;
        } else {
            writer.write_all(&line).await?;
            writer.write_all(b"\n").await?;
        }
        stats.entities += 1;
    }
    // Keep one database scan/snapshot and defer relations on disk, never in a
    // dataset-sized in-memory buffer. Preserve scan order within each group.
    relations.flush().await?;
    drop(relations);
    let mut relations = tokio::fs::File::open(&relations_path).await?;
    tokio::io::copy(&mut relations, writer).await?;
    writer.flush().await?;
    Ok(stats)
}

fn relation_classes(catalog: &Catalog) -> BTreeSet<LocalClassId> {
    let mut classes = BTreeSet::new();
    loop {
        let previous_len = classes.len();
        for (lid, schema) in catalog.classes() {
            let class = &schema.class;
            if class.id == RELATION_CLASS_ID
                || class.inherits.iter().chain(&class.extends).any(|base| {
                    catalog
                        .class_id(&base.id)
                        .is_some_and(|lid| classes.contains(&lid))
                })
            {
                classes.insert(lid);
            }
        }
        if classes.len() == previous_len {
            return classes;
        }
    }
}

fn is_relation(catalog: &Catalog, classes: &BTreeSet<LocalClassId>, record: &EntityRecord) -> bool {
    let class_ids = record
        .object
        .get(ATTR_TYPE)
        .and_then(Value::as_str)
        .map(|name| catalog.class_ids(name))
        .unwrap_or_default();
    if class_ids.iter().any(|lid| classes.contains(lid)) {
        return true;
    }
    // External relationships can also use untyped rows or independent classes.
    // Embedded relationships remain part of their regular owner entity.
    let Some(collection) = catalog.collection_by_name(&record.collection) else {
        return false;
    };
    let field_value = |alias: &str, fallback: &str| {
        let field = if class_ids.len() == 1 {
            catalog.class_field_for_alias(class_ids[0], alias)
        } else {
            None
        };
        record.object.get(
            field
                .as_deref()
                .unwrap_or_else(|| collection.canonical_field_name(fallback)),
        )
    };
    catalog.relationships().any(|(_, schema)| {
        schema.relationship.source_collection == record.collection
            && matches!(schema.relationship.mode, RelationMode::External)
            && field_value("relation", ATTR_RELATION_RELATION)
                .is_none_or(|value| value.as_str() == Some(schema.relationship.id.as_str()))
            && [("from", ATTR_RELATION_FROM), ("to", ATTR_RELATION_TO)]
                .into_iter()
                .all(|(alias, fallback)| {
                    field_value(alias, fallback)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
                })
    })
}

pub(crate) async fn import_jsonl<R>(
    db: &dyn crate::SemanticDb,
    reader: &mut R,
    source_name: &str,
    options: &ImportOptions,
) -> Result<TransferStats, TransferError>
where
    R: AsyncBufRead + Unpin,
{
    options.validate()?;
    let mut stats = TransferStats::default();
    let mut physical_line = 0u64;
    let mut batch = Batch::new();
    loop {
        let Some(line) = read_bounded_line(
            reader,
            options.max_record_bytes,
            source_name,
            physical_line + 1,
        )
        .await?
        else {
            break;
        };
        physical_line += 1;
        let line = strip_line_ending(&line);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let record = decode_line(source_name, physical_line, line)?;
        batch.operations.push(BatchOperation::Upsert {
            collection: record.collection,
            id: record.id,
            object: record.object,
        });
        if batch.operations.len() == options.batch_size {
            flush_batch(db, &mut batch, options, &mut stats).await?;
        }
    }
    flush_batch(db, &mut batch, options, &mut stats).await?;
    Ok(stats)
}

pub(crate) fn scan_jsonl_file(
    path: &std::path::Path,
    source_name: &str,
    max_record_bytes: usize,
    mut visit: impl FnMut(&EntityRecord) -> Result<(), TransferError>,
) -> Result<u64, TransferError> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut bytes = Vec::new();
    let mut line = 0u64;
    let mut entities = 0u64;
    loop {
        bytes.clear();
        let read = reader
            .by_ref()
            .take(max_record_bytes.saturating_add(1) as u64)
            .read_until(b'\n', &mut bytes)?;
        if read == 0 {
            break;
        }
        line += 1;
        if bytes.len() > max_record_bytes {
            return Err(TransferError::RecordTooLarge {
                source_name: source_name.into(),
                line,
                limit: max_record_bytes,
            });
        }
        let bytes = strip_line_ending(&bytes);
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let record = decode_line(source_name, line, bytes)?;
        visit(&record)?;
        entities += 1;
    }
    Ok(entities)
}

async fn flush_batch(
    db: &dyn crate::SemanticDb,
    batch: &mut Batch,
    options: &ImportOptions,
    stats: &mut TransferStats,
) -> Result<(), TransferError> {
    if batch.operations.is_empty() {
        return Ok(());
    }
    let entity_count = batch.operations.len() as u64;
    let pending = std::mem::replace(batch, Batch::new());
    let reply = db
        .execute_batch_returning_bounded_with_settings(
            pending,
            BatchReturn::Stats,
            options.write_settings,
        )
        .await
        .map_err(|source| TransferError::ImportDatabase {
            committed_entities: stats.entities,
            committed_batches: stats.batches,
            source,
        })?;
    match reply {
        BatchReply::Stats { .. } => {}
        _ => {
            return Err(TransferError::DatabaseProtocol(
                "database returned a non-stats reply for an import batch".into(),
            ));
        }
    }
    stats.entities += entity_count;
    stats.batches += 1;
    Ok(())
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
    source_name: &str,
    line: u64,
) -> Result<Option<Vec<u8>>, TransferError> {
    let mut out = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok((!out.is_empty()).then_some(out));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |index| index + 1);
        if out.len().saturating_add(take) > limit {
            return Err(TransferError::RecordTooLarge {
                source_name: source_name.into(),
                line,
                limit,
            });
        }
        out.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline.is_some() {
            return Ok(Some(out));
        }
    }
}

fn strip_line_ending(mut line: &[u8]) -> &[u8] {
    if line.ends_with(b"\n") {
        line = &line[..line.len() - 1];
    }
    if line.ends_with(b"\r") {
        line = &line[..line.len() - 1];
    }
    line
}

fn invalid(source_name: &str, line: u64, message: impl Into<String>) -> TransferError {
    TransferError::InvalidRecord {
        source_name: source_name.into(),
        line,
        message: message.into(),
    }
}

fn validate_finite(value: &Value) -> Result<(), TransferError> {
    match value {
        Value::F32(value) if !value.into_inner().is_finite() => Err(TransferError::InvalidRecord {
            source_name: "database entity".into(),
            line: 0,
            message: "non-finite f32 values cannot be represented in JSON".into(),
        }),
        Value::F64(value) if !value.into_inner().is_finite() => Err(TransferError::InvalidRecord {
            source_name: "database entity".into(),
            line: 0,
            message: "non-finite f64 values cannot be represented in JSON".into(),
        }),
        Value::List(values) => values.iter().try_for_each(validate_finite),
        Value::Map(values) => values
            .iter()
            .try_for_each(|(key, value)| validate_finite(key).and_then(|_| validate_finite(value))),
        Value::Object(values) => values.values().try_for_each(validate_finite),
        Value::Variant(value) => validate_finite(&value.value),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::value::{Object, Value, VariantValue};

    use super::{decode_line, encode_record};

    #[test]
    fn relation_classification_includes_inheritance_and_extensions() {
        use semantic_data::attr::RELATION_CLASS_ID;
        use semantic_data::schema::ClassRef;

        let mut catalog = semantic_db_core::catalog::Catalog::new();
        let base = semantic_data::schema::ClassType {
            id: RELATION_CLASS_ID.into(),
            name: "Relation".into(),
            inherits: None,
            extends: Vec::new(),
            strict_schema: false,
            creatable_in_ui: None,
            attributes: Default::default(),
            constraints: Vec::new(),
            meta: Default::default(),
        };
        catalog.upsert_class(base.clone()).unwrap();
        let mut child = base.clone();
        child.id = "test:child".into();
        child.name = "Child".into();
        child.inherits = Some(ClassRef {
            id: RELATION_CLASS_ID.into(),
        });
        catalog.upsert_class(child).unwrap();
        let mut extension = base;
        extension.id = "test:extension".into();
        extension.name = "Extension".into();
        extension.inherits = None;
        extension.extends = vec![ClassRef {
            id: "test:child".into(),
        }];
        catalog.upsert_class(extension).unwrap();
        let classes = super::relation_classes(&catalog);
        for name in [RELATION_CLASS_ID, "test:child", "test:extension"] {
            let mut object = Object::new();
            object.insert("type", Value::String(name.into()));
            let record = semantic_db_core::EntityRecord {
                collection: "entities".into(),
                id: name.into(),
                object,
            };
            assert!(super::is_relation(&catalog, &classes, &record), "{name}");
        }
    }

    #[test]
    fn typed_entity_round_trips_without_flattening_values() {
        let mut nested = Object::new();
        nested.insert("small", Value::I8(-7));
        nested.insert("large", Value::U128(u128::MAX));
        nested.insert("bytes", Value::Bytes(vec![0, 1, 255].into()));
        nested.insert(
            "choice",
            Value::Variant(Box::new(VariantValue {
                r#type: Some("result".into()),
                variant: "ok".into(),
                value: Value::I16(42),
            })),
        );
        let mut object = Object::new();
        object.insert("id", Value::String("record-1".into()));
        object.insert("nested", Value::Object(nested));
        let record = semantic_db_core::EntityRecord {
            id: "record-1".into(),
            collection: "custom".into(),
            object,
        };
        let encoded = encode_record(&record).unwrap();
        let decoded = decode_line("memory", 1, &encoded).unwrap();
        assert_eq!(decoded, record);
    }

    #[test]
    fn envelope_identity_mismatch_is_rejected() {
        let line = br#"{"version":1,"collection":"entities","id":"outer","object":{"object":{"id":{"string":"inner"}}}}"#;
        let error = decode_line("fixture.jsonl", 7, line).unwrap_err();
        assert!(error.to_string().contains("fixture.jsonl:7"));
        assert!(error.to_string().contains("does not match"));
    }
}
