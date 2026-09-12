use std::collections::{BTreeMap, BTreeSet};

use semantic_data::value::Object;

use crate::{BatchOutcome, BatchStats, Dataset, DbError, EntityRecord, catalog::Catalog};

/// Select the result of a committed batch. Dataset preserves the original API.
#[derive(facet::Facet, Debug, Clone, PartialEq, Eq, Default)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BatchReturn {
    #[default]
    Dataset,
    Stats,
    Changes,
    Projection {
        fields: Vec<String>,
    },
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum EntityChangeKind {
    Upsert,
    Delete,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct EntityChange {
    pub collection: String,
    pub id: String,
    pub kind: EntityChangeKind,
}

/// Rust uses an enum; the application RPC emits each variant's fields directly.
#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BatchReply {
    Dataset(BatchOutcome),
    Stats {
        stats: BatchStats,
    },
    Changes {
        stats: BatchStats,
        changes: Vec<EntityChange>,
    },
    Projection {
        stats: BatchStats,
        changes: Vec<EntityChange>,
        rows: Vec<EntityRecord>,
    },
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BatchReturnErrorReason {
    UnknownMode,
    UnknownField,
    DuplicateField,
}

impl BatchReturnErrorReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownMode => "unknown_mode",
            Self::UnknownField => "unknown_field",
            Self::DuplicateField => "duplicate_field",
        }
    }
}

pub(crate) type EntityKey = (String, String);
pub(crate) struct RowChange {
    pub before: Option<Object>,
    pub after: Option<Object>,
}
pub(crate) type ChangeSet = BTreeMap<EntityKey, RowChange>;

pub(crate) fn changes(before: &Dataset, after: &Dataset) -> ChangeSet {
    let mut changes = BTreeMap::new();
    for collection in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        let old = before.get(collection);
        let new = after.get(collection);
        for id in old
            .into_iter()
            .flat_map(|rows| rows.keys())
            .chain(new.into_iter().flat_map(|rows| rows.keys()))
            .collect::<BTreeSet<_>>()
        {
            let before = old.and_then(|rows| rows.get(id));
            let after = new.and_then(|rows| rows.get(id));
            if before != after {
                changes.insert(
                    (collection.clone(), id.clone()),
                    RowChange {
                        before: before.cloned(),
                        after: after.cloned(),
                    },
                );
            }
        }
    }
    changes
}

fn field_error(reason: BatchReturnErrorReason, field: &str) -> DbError {
    DbError::BatchReturn {
        reason,
        field: Some(field.to_string()),
    }
}

/// Resolve a flat projection once against the committing catalog and touched rows.
/// Ambiguous aliases are rejected; callers can use qualified attribute IDs.
fn projection_fields(
    catalog: &Catalog,
    before: &Dataset,
    after: &Dataset,
    fields: &[String],
) -> Result<Vec<String>, DbError> {
    let mut requested = BTreeSet::new();
    for field in fields {
        if !requested.insert(field) {
            return Err(field_error(BatchReturnErrorReason::DuplicateField, field));
        }
    }
    let mut selected = BTreeSet::new();
    let mut resolved = Vec::new();
    for field in fields {
        let mut candidates = BTreeSet::new();
        for id in catalog.attribute_ids(field) {
            if let Some(attribute) = catalog.attribute_by_lid(id) {
                candidates.insert(attribute.attribute.id.clone());
            }
        }
        for (name, rows) in before.iter().chain(after.iter()) {
            if let Some(collection) = catalog.collection_by_name(name) {
                let canonical = collection.canonical_field_name(field);
                if collection.knows_field(canonical) {
                    candidates.insert(canonical.to_string());
                }
            }
            for row in rows.values() {
                if row.contains_key(field) {
                    candidates.insert(field.clone());
                }
                if let Some(class) = row
                    .get("type")
                    .and_then(|v| v.as_str())
                    .and_then(|id| catalog.class_id(id))
                    && let Some(canonical) = catalog.class_field_for_alias(class, field)
                {
                    candidates.insert(canonical);
                }
            }
        }
        if candidates.len() != 1 {
            return Err(field_error(BatchReturnErrorReason::UnknownField, field));
        }
        let canonical = candidates.into_iter().next().expect("one candidate");
        if !selected.insert(canonical.clone()) {
            return Err(field_error(BatchReturnErrorReason::DuplicateField, field));
        }
        resolved.push(canonical);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Batch, BatchOperation,
        catalog::CollectionKind,
        embedded::{EmbeddedDb, MemoryEntityStorage},
    };
    use semantic_data::value::Value;

    fn row(id: &str, name: &str) -> Object {
        Object::from_iter([
            ("id".into(), Value::String(id.into())),
            ("name".into(), Value::String(name.into())),
        ])
    }
    fn upsert(id: &str, name: &str) -> BatchOperation {
        BatchOperation::Upsert {
            collection: "items".into(),
            id: id.into(),
            object: row(id, name),
        }
    }
    fn delete(id: &str) -> BatchOperation {
        BatchOperation::DeleteById {
            collection: "items".into(),
            id: id.into(),
        }
    }
    fn db() -> EmbeddedDb<MemoryEntityStorage> {
        let mut db = EmbeddedDb::new(MemoryEntityStorage::new());
        db.create_collection("items", CollectionKind::Polymorphic)
            .unwrap();
        db
    }

    #[test]
    fn batch_return_net_changes_are_sorted_and_create_aware() {
        let mut db = db();
        db.execute_batch(
            Batch::new()
                .with_op(upsert("same", "same"))
                .with_op(upsert("gone", "old"))
                .with_op(upsert("rewrite", "old")),
        )
        .unwrap();
        let result = db
            .execute_batch_returning(
                Batch::new()
                    .with_op(upsert("same", "same"))
                    .with_op(upsert("transient", "temp"))
                    .with_op(delete("transient"))
                    .with_op(delete("gone"))
                    .with_op(delete("missing"))
                    .with_op(delete("rewrite"))
                    .with_op(upsert("rewrite", "new"))
                    .with_op(BatchOperation::Create {
                        collection: "items".into(),
                        id: "created".into(),
                        object: row("created", "new"),
                    }),
                BatchReturn::Changes,
            )
            .unwrap();
        let BatchReply::Changes { stats, changes } = result else {
            panic!("changes reply")
        };
        assert_eq!(stats.upserted, 4);
        assert_eq!(stats.deleted, 3);
        assert_eq!(
            changes,
            vec![
                EntityChange {
                    collection: "items".into(),
                    id: "created".into(),
                    kind: EntityChangeKind::Upsert
                },
                EntityChange {
                    collection: "items".into(),
                    id: "gone".into(),
                    kind: EntityChangeKind::Delete
                },
                EntityChange {
                    collection: "items".into(),
                    id: "rewrite".into(),
                    kind: EntityChangeKind::Upsert
                },
            ]
        );
    }

    #[test]
    fn batch_return_projection_is_final_state_and_invalid_selection_is_atomic() {
        let mut db = db();
        db.execute_batch(Batch::new().with_op(upsert("gone", "old")))
            .unwrap();
        let result = db
            .execute_batch_returning(
                Batch::new()
                    .with_op(upsert("new", "initial"))
                    .with_op(upsert("new", "final"))
                    .with_op(delete("gone")),
                BatchReturn::Projection {
                    fields: vec!["name".into()],
                },
            )
            .unwrap();
        let BatchReply::Projection { rows, changes, .. } = result else {
            panic!("projection")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "new");
        assert_eq!(
            rows[0].object,
            Object::from_iter([("semantic:name".into(), Value::String("final".into()))])
        );
        assert_eq!(changes.len(), 2);
        for (fields, reason) in [
            (
                vec!["name".into(), "name".into()],
                BatchReturnErrorReason::DuplicateField,
            ),
            (vec!["unknown".into()], BatchReturnErrorReason::UnknownField),
        ] {
            let err = db
                .execute_batch_returning(
                    Batch::new().with_op(upsert("new", "must not commit")),
                    BatchReturn::Projection { fields },
                )
                .unwrap_err();
            assert!(matches!(err, DbError::BatchReturn { reason: actual, .. } if actual == reason));
            assert_eq!(
                db.get("items", "new")
                    .unwrap()
                    .unwrap()
                    .object
                    .get("semantic:name"),
                Some(&Value::String("final".into()))
            );
        }
        let BatchReply::Projection { rows, .. } = db
            .execute_batch_returning(
                Batch::new().with_op(upsert("new", "next")),
                BatchReturn::Projection { fields: vec![] },
            )
            .unwrap()
        else {
            panic!("projection")
        };
        assert!(rows[0].object.is_empty());
        assert_eq!(rows[0].id, "new");
    }

    #[test]
    fn batch_return_default_stats_empty_and_failed_create() {
        let mut db = db();
        let original = db
            .execute_batch(Batch::new().with_op(upsert("one", "first")))
            .unwrap();
        let BatchReply::Dataset(reply) = db
            .execute_batch_returning(
                Batch::new().with_op(upsert("one", "first")),
                BatchReturn::default(),
            )
            .unwrap()
        else {
            panic!("dataset")
        };
        assert_eq!(original, reply);
        let stats = db
            .execute_batch_returning(
                Batch::new().with_op(upsert("one", "second")),
                BatchReturn::Stats,
            )
            .unwrap();
        assert!(matches!(
            stats,
            BatchReply::Stats {
                stats: BatchStats { upserted: 1, .. }
            }
        ));
        assert!(
            matches!(db.execute_batch_returning(Batch::new(), BatchReturn::Changes).unwrap(), BatchReply::Changes { changes, .. } if changes.is_empty())
        );
        let error = db
            .execute_batch_returning(
                Batch::new()
                    .with_op(upsert("other", "must roll back"))
                    .with_op(BatchOperation::Create {
                        collection: "items".into(),
                        id: "one".into(),
                        object: row("one", "collision"),
                    }),
                BatchReturn::Changes,
            )
            .unwrap_err();
        assert!(matches!(error, DbError::EntityExists { .. }));
        assert!(db.get("items", "other").unwrap().is_none());
    }

    #[test]
    fn batch_return_registered_aliases_are_canonical_and_optional_values_omitted() {
        use semantic_data::schema::{
            AttributeType, ClassAttribute, ClassType, Meta, StringType, Type, TypeKind,
            attribute::attribute_ref::AttributeRef,
        };
        let mut catalog = Catalog::new();
        catalog.upsert_attribute(AttributeType {
            id: "example:label".into(),
            name: "label".into(),
            ty: Type::new(TypeKind::String(StringType {
                format: None,
                normalization: None,
            })),
            constraints: vec![],
            meta: Meta::default(),
        });
        catalog
            .upsert_class(ClassType {
                id: "example:item".into(),
                name: "Item".into(),
                inherits: None,
                extends: vec![],
                strict_schema: false,
                creatable_in_ui: None,
                attributes: BTreeMap::from([(
                    "caption".into(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: "example:label".into(),
                        },
                        required: false,
                        ui_order: None,
                        computed: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )]),
                constraints: vec![],
                meta: Meta::default(),
            })
            .unwrap();
        let mut one = Object::new();
        one.insert("type", "example:item".to_string());
        one.insert("example:label", "hello".to_string());
        let mut two = Object::new();
        two.insert("type", "example:item".to_string());
        let out = BatchOutcome {
            dataset: BTreeMap::from([(
                "items".into(),
                BTreeMap::from([("one".into(), one), ("two".into(), two)]),
            )]),
            stats: BatchStats {
                upserted: 2,
                updated: 0,
                deleted: 0,
            },
        };
        let Some(BatchReply::Projection { rows, .. }) = compact_reply(
            &catalog,
            &Dataset::new(),
            &out,
            &BatchReturn::Projection {
                fields: vec!["caption".into()],
            },
        )
        .unwrap() else {
            panic!("projection")
        };
        assert_eq!(
            rows[0].object.get("example:label"),
            Some(&Value::String("hello".into()))
        );
        assert!(rows[1].object.is_empty());
        assert!(matches!(
            compact_reply(
                &catalog,
                &Dataset::new(),
                &out,
                &BatchReturn::Projection {
                    fields: vec!["caption".into(), "example:label".into()]
                }
            ),
            Err(DbError::BatchReturn {
                reason: BatchReturnErrorReason::DuplicateField,
                ..
            })
        ));
    }
}

/// Prepare fallible output before committing; no post-commit reads are needed.
pub(crate) fn compact_reply(
    catalog: &Catalog,
    before: &Dataset,
    outcome: &BatchOutcome,
    returning: &BatchReturn,
) -> Result<Option<BatchReply>, DbError> {
    if matches!(returning, BatchReturn::Dataset) {
        return Ok(None);
    }
    compact_reply_from_changes(
        catalog,
        before,
        &outcome.dataset,
        &changes(before, &outcome.dataset),
        &outcome.stats,
        returning,
    )
}

pub(crate) fn compact_reply_from_changes(
    catalog: &Catalog,
    before: &Dataset,
    after: &Dataset,
    delta: &ChangeSet,
    stats: &BatchStats,
    returning: &BatchReturn,
) -> Result<Option<BatchReply>, DbError> {
    let fields = match returning {
        BatchReturn::Dataset => return Ok(None),
        BatchReturn::Stats => {
            return Ok(Some(BatchReply::Stats {
                stats: stats.clone(),
            }));
        }
        BatchReturn::Changes => None,
        BatchReturn::Projection { fields } => {
            Some(projection_fields(catalog, before, after, fields)?)
        }
    };
    let changes = delta
        .iter()
        .map(|((collection, id), change)| EntityChange {
            collection: collection.clone(),
            id: id.clone(),
            kind: if change.after.is_some() {
                EntityChangeKind::Upsert
            } else {
                EntityChangeKind::Delete
            },
        })
        .collect();
    let stats = stats.clone();
    Ok(Some(if let Some(fields) = fields {
        let rows = delta
            .iter()
            .filter_map(|((collection, id), change)| {
                change.after.as_ref().map(|row| EntityRecord {
                    collection: collection.clone(),
                    id: id.clone(),
                    object: fields
                        .iter()
                        .filter_map(|field| {
                            row.get(field).cloned().map(|value| (field.clone(), value))
                        })
                        .collect(),
                })
            })
            .collect();
        BatchReply::Projection {
            stats,
            changes,
            rows,
        }
    } else {
        BatchReply::Changes { stats, changes }
    }))
}
