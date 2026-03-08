use semantic_data::schema::{
    attribute::attribute_ref::AttributeRef,
    attribute::attribute_type::AttributeType,
    class::class_attribute::ClassAttribute,
    class::class_type::ClassType,
    core::{meta::Meta, type_kind::TypeKind, type_node::Type, type_ref::TypeRef},
    primitives::{
        any_type::AnyType, bool_type::BoolType, number_type::NumberType, string_type::StringType,
        uint_width::UIntWidth,
    },
    record::record_type::RecordType,
    relation::relation_type::RelationType,
};

use crate::{
    CoreError,
    catalog::{Catalog, CollectionKind, RELATION_CLASS_ID},
};

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct DdlBatch {
    pub operations: Vec<DdlOperation>,
}

impl DdlBatch {
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    pub fn with_op(mut self, op: DdlOperation) -> Self {
        self.operations.push(op);
        self
    }
}

impl Default for DdlBatch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum DdlCollectionKind {
    Untyped,
    Record { record_type: String },
    Class { class: String },
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum DdlOperation {
    UpsertAttribute {
        attribute: AttributeType,
    },
    DeleteAttribute {
        id: String,
    },
    UpsertRecordType {
        id: String,
        name: String,
        record: RecordType,
    },
    DeleteRecordType {
        id: String,
    },
    UpsertClass {
        class: ClassType,
    },
    DeleteClass {
        id: String,
    },
    UpsertCollection {
        name: String,
        kind: DdlCollectionKind,
    },
    DeleteCollection {
        name: String,
    },
    UpsertIndex {
        name: String,
        collection: String,
        field: String,
        unique: bool,
    },
    DeleteIndex {
        name: String,
        collection: String,
    },
    UpsertRelationship {
        relationship: RelationType,
    },
    DeleteRelationship {
        id: String,
    },
    SetAutoIndex {
        enabled: bool,
    },
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DdlStats {
    pub upserted: usize,
    pub deleted: usize,
}

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq)]
pub struct DdlOutcome {
    pub stats: DdlStats,
}

pub fn apply_ddl_batch(
    catalog: &Catalog,
    batch: &DdlBatch,
) -> Result<(Catalog, DdlOutcome), CoreError> {
    let mut catalog = catalog.clone();
    let mut stats = DdlStats::default();

    for op in &batch.operations {
        match op {
            DdlOperation::UpsertAttribute { attribute } => {
                catalog.upsert_attribute(attribute.clone());
                stats.upserted += 1;
            }
            DdlOperation::DeleteAttribute { id } => {
                for (_, class) in catalog.classes() {
                    if class.attributes.values().any(|attr| {
                        catalog
                            .attribute_by_lid(*attr)
                            .is_some_and(|a| a.attribute.id == *id)
                    }) {
                        return Err(CoreError::new(format!(
                            "cannot delete attribute '{id}': referenced by class '{}'",
                            class.class.id
                        )));
                    }
                }
                if catalog.delete_attribute(id) {
                    stats.deleted += 1;
                }
            }
            DdlOperation::UpsertRecordType { id, name, record } => {
                catalog.upsert_record_type(id.clone(), name.clone(), record.clone());
                stats.upserted += 1;
            }
            DdlOperation::DeleteRecordType { id } => {
                if let Some(record_lid) = catalog.record_type_id(id) {
                    for (_, collection) in catalog.collections() {
                        if matches!(
                            collection.kind,
                            CollectionKind::Record { record_type } if record_type == record_lid
                        ) {
                            return Err(CoreError::new(format!(
                                "cannot delete record type '{id}': referenced by collection '{}'",
                                collection.name
                            )));
                        }
                    }
                }
                if catalog.delete_record_type(id) {
                    stats.deleted += 1;
                }
            }
            DdlOperation::UpsertClass { class } => {
                catalog
                    .upsert_class(class.clone())
                    .map_err(|e| CoreError::new(e.to_string()))?;
                stats.upserted += 1;
            }
            DdlOperation::DeleteClass { id } => {
                if let Some(class_lid) = catalog.class_id(id) {
                    for (_, collection) in catalog.collections() {
                        if matches!(
                            collection.kind,
                            CollectionKind::Class { class } if class == class_lid
                        ) {
                            return Err(CoreError::new(format!(
                                "cannot delete class '{id}': referenced by collection '{}'",
                                collection.name
                            )));
                        }
                    }
                }
                if catalog.delete_class(id) {
                    stats.deleted += 1;
                }
            }
            DdlOperation::UpsertCollection { name, kind } => {
                let kind = resolve_collection_kind(&catalog, kind)?;
                catalog
                    .upsert_collection(name.clone(), kind)
                    .map_err(|e| CoreError::new(e.to_string()))?;
                stats.upserted += 1;
            }
            DdlOperation::DeleteCollection { name } => {
                if catalog.delete_collection(name) {
                    stats.deleted += 1;
                }
            }
            DdlOperation::UpsertIndex {
                name,
                collection,
                field,
                unique,
            } => {
                let Some(collection_schema) = catalog.collection_by_name(collection) else {
                    return Err(CoreError::new(format!(
                        "collection '{collection}' not found"
                    )));
                };
                catalog
                    .upsert_index(name.clone(), collection_schema.lid, field.clone(), *unique)
                    .map_err(|e| CoreError::new(e.to_string()))?;
                stats.upserted += 1;
            }
            DdlOperation::DeleteIndex { name, collection } => {
                let Some(collection_schema) = catalog.collection_by_name(collection) else {
                    return Err(CoreError::new(format!(
                        "collection '{collection}' not found"
                    )));
                };
                if catalog.delete_index(collection_schema.lid, name) {
                    stats.deleted += 1;
                }
            }
            DdlOperation::UpsertRelationship { relationship } => {
                catalog
                    .upsert_relationship(relationship.clone())
                    .map_err(|e| CoreError::new(e.to_string()))?;
                stats.upserted += 1;
            }
            DdlOperation::DeleteRelationship { id } => {
                if catalog.delete_relationship(id) {
                    stats.deleted += 1;
                }
            }
            DdlOperation::SetAutoIndex { enabled } => {
                catalog.set_auto_index_enabled(*enabled);
                stats.upserted += 1;
            }
        }
    }

    Ok((catalog, DdlOutcome { stats }))
}

pub const CORE_CATALOG_ENTRY_CLASS_ID: &str = "semantic.catalog.entry";
pub const CORE_CATALOG_ATTRIBUTES_COLLECTION: &str = "__semantic.catalog.attributes";
pub const CORE_CATALOG_TYPE_DEFS_COLLECTION: &str = "__semantic.catalog.type_defs";
pub const CORE_CATALOG_RECORD_TYPES_COLLECTION: &str = "__semantic.catalog.record_types";
pub const CORE_CATALOG_CLASSES_COLLECTION: &str = "__semantic.catalog.classes";
pub const CORE_CATALOG_COLLECTIONS_COLLECTION: &str = "__semantic.catalog.collections";
pub const CORE_CATALOG_INDEXES_COLLECTION: &str = "__semantic.catalog.indexes";
pub const CORE_CATALOG_META_COLLECTION: &str = "__semantic.catalog.meta";

const CORE_CATALOG_ATTR_ID: &str = "semantic.catalog.id";
const CORE_CATALOG_ATTR_LID: &str = "semantic.catalog.lid";
const CORE_CATALOG_ATTR_ATTRIBUTE: &str = "semantic.catalog.attribute";
const CORE_CATALOG_ATTR_TYPE_DEF: &str = "semantic.catalog.type_def";
const CORE_CATALOG_ATTR_RECORD: &str = "semantic.catalog.record";
const CORE_CATALOG_ATTR_CLASS: &str = "semantic.catalog.class";
const CORE_CATALOG_ATTR_NAME: &str = "semantic.catalog.name";
const CORE_CATALOG_ATTR_COLLECTION_KIND: &str = "semantic.catalog.collection_kind";
const CORE_CATALOG_ATTR_FIELD_IDS: &str = "semantic.catalog.field_ids";
const CORE_CATALOG_ATTR_COLLECTION: &str = "semantic.catalog.collection";
const CORE_CATALOG_ATTR_FIELD: &str = "semantic.catalog.field";
const CORE_CATALOG_ATTR_INDEX_KIND: &str = "semantic.catalog.index_kind";
const CORE_CATALOG_ATTR_UNIQUE: &str = "semantic.catalog.unique";
const CORE_CATALOG_ATTR_NEXT_FIELD_ID: &str = "semantic.catalog.next_field_id";
const CORE_CATALOG_ATTR_AUTO_INDEX_ENABLED: &str = "semantic.catalog.auto_index_enabled";
const RELATION_ATTR_RELATION: &str = "semantic.relation.relation";
const RELATION_ATTR_FROM: &str = "semantic.relation.from";
const RELATION_ATTR_TO: &str = "semantic.relation.to";

pub fn core_catalog_schema_batch() -> DdlBatch {
    let mut attrs = std::collections::BTreeMap::new();
    attrs.insert(
        "id".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_ID.to_string(),
            },
            required: true,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "lid".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_LID.to_string(),
            },
            required: true,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "attribute".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_ATTRIBUTE.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "type_def".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_TYPE_DEF.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "record".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_RECORD.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "class".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_CLASS.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "name".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_NAME.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "collection_kind".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_COLLECTION_KIND.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "field_ids".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_FIELD_IDS.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "collection".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_COLLECTION.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "field".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_FIELD.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "index_kind".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_INDEX_KIND.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "unique".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_UNIQUE.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "next_field_id".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_NEXT_FIELD_ID.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "auto_index_enabled".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_AUTO_INDEX_ENABLED.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );

    let core_entry = ClassType {
        id: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
        name: "CatalogEntry".to_string(),
        inherits: None,
        extends: vec![],
        attributes: attrs,
        constraints: vec![],
        meta: Meta::default(),
    };

    let mut relation_attrs = std::collections::BTreeMap::new();
    relation_attrs.insert(
        "relation".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: RELATION_ATTR_RELATION.to_string(),
            },
            required: true,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    relation_attrs.insert(
        "from".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: RELATION_ATTR_FROM.to_string(),
            },
            required: true,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    relation_attrs.insert(
        "to".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: RELATION_ATTR_TO.to_string(),
            },
            required: true,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    let relation_class = ClassType {
        id: RELATION_CLASS_ID.to_string(),
        name: "Relation".to_string(),
        inherits: None,
        extends: vec![],
        attributes: relation_attrs,
        constraints: vec![],
        meta: Meta::default(),
    };

    DdlBatch::new()
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_ID.to_string(),
                name: "id".to_string(),
                ty: Type {
                    kind: TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_LID.to_string(),
                name: "lid".to_string(),
                ty: Type {
                    kind: TypeKind::Number(NumberType::UInt(UIntWidth::U64)),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_ATTRIBUTE.to_string(),
                name: "attribute".to_string(),
                ty: Type {
                    kind: TypeKind::Any(AnyType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_TYPE_DEF.to_string(),
                name: "type_def".to_string(),
                ty: Type {
                    kind: TypeKind::Any(AnyType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_RECORD.to_string(),
                name: "record".to_string(),
                ty: Type {
                    kind: TypeKind::Any(AnyType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_CLASS.to_string(),
                name: "class".to_string(),
                ty: Type {
                    kind: TypeKind::Any(AnyType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_NAME.to_string(),
                name: "name".to_string(),
                ty: Type {
                    kind: TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_COLLECTION_KIND.to_string(),
                name: "collection_kind".to_string(),
                ty: Type {
                    kind: TypeKind::Any(AnyType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_FIELD_IDS.to_string(),
                name: "field_ids".to_string(),
                ty: Type {
                    kind: TypeKind::Any(AnyType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_COLLECTION.to_string(),
                name: "collection".to_string(),
                ty: Type {
                    kind: TypeKind::Number(NumberType::UInt(UIntWidth::U64)),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_FIELD.to_string(),
                name: "field".to_string(),
                ty: Type {
                    kind: TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_INDEX_KIND.to_string(),
                name: "index_kind".to_string(),
                ty: Type {
                    kind: TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_UNIQUE.to_string(),
                name: "unique".to_string(),
                ty: Type {
                    kind: TypeKind::Bool(BoolType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_NEXT_FIELD_ID.to_string(),
                name: "next_field_id".to_string(),
                ty: Type {
                    kind: TypeKind::Number(NumberType::UInt(UIntWidth::U64)),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: CORE_CATALOG_ATTR_AUTO_INDEX_ENABLED.to_string(),
                name: "auto_index_enabled".to_string(),
                ty: Type {
                    kind: TypeKind::Bool(BoolType),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: "parent".to_string(),
                name: "parent".to_string(),
                ty: Type {
                    kind: TypeKind::Ref(TypeRef {
                        name: "id".to_string(),
                        args: vec![],
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: RELATION_ATTR_RELATION.to_string(),
                name: "relation".to_string(),
                ty: Type {
                    kind: TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: RELATION_ATTR_FROM.to_string(),
                name: "from".to_string(),
                ty: Type {
                    kind: TypeKind::Ref(TypeRef {
                        name: "id".to_string(),
                        args: vec![],
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: RELATION_ATTR_TO.to_string(),
                name: "to".to_string(),
                ty: Type {
                    kind: TypeKind::Ref(TypeRef {
                        name: "id".to_string(),
                        args: vec![],
                    }),
                    constraints: vec![],
                    annotations: vec![],
                    meta: Meta::default(),
                },
                constraints: vec![],
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertClass {
            class: relation_class,
        })
        .with_op(DdlOperation::UpsertClass { class: core_entry })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_ATTRIBUTES_COLLECTION.to_string(),
            kind: DdlCollectionKind::Class {
                class: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
            },
        })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_TYPE_DEFS_COLLECTION.to_string(),
            kind: DdlCollectionKind::Class {
                class: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
            },
        })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_RECORD_TYPES_COLLECTION.to_string(),
            kind: DdlCollectionKind::Class {
                class: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
            },
        })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_CLASSES_COLLECTION.to_string(),
            kind: DdlCollectionKind::Class {
                class: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
            },
        })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_COLLECTIONS_COLLECTION.to_string(),
            kind: DdlCollectionKind::Class {
                class: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
            },
        })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_INDEXES_COLLECTION.to_string(),
            kind: DdlCollectionKind::Class {
                class: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
            },
        })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_META_COLLECTION.to_string(),
            kind: DdlCollectionKind::Class {
                class: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
            },
        })
}

pub fn fresh_catalog_with_core_schema() -> Result<Catalog, CoreError> {
    let (catalog, _) = apply_ddl_batch(&Catalog::new(), &core_catalog_schema_batch())?;
    Ok(catalog)
}

fn resolve_collection_kind(
    catalog: &Catalog,
    kind: &DdlCollectionKind,
) -> Result<CollectionKind, CoreError> {
    match kind {
        DdlCollectionKind::Untyped => Ok(CollectionKind::Untyped),
        DdlCollectionKind::Record { record_type } => {
            let Some(record_lid) = catalog.record_type_id(record_type) else {
                return Err(CoreError::new(format!(
                    "record type '{record_type}' not found"
                )));
            };
            Ok(CollectionKind::Record {
                record_type: record_lid,
            })
        }
        DdlCollectionKind::Class { class } => {
            let Some(class_lid) = catalog.class_id(class) else {
                return Err(CoreError::new(format!("class '{class}' not found")));
            };
            Ok(CollectionKind::Class { class: class_lid })
        }
    }
}
