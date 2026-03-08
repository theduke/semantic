use semantic_data::schema::{
    ClassRef,
    attribute::attribute_ref::AttributeRef,
    attribute::attribute_type::AttributeType,
    class::class_attribute::ClassAttribute,
    class::class_type::ClassType,
    core::{
        meta::Meta, type_def::TypeDef, type_kind::TypeKind, type_node::Type, type_ref::TypeRef,
    },
    primitives::{
        any_type::AnyType, bool_type::BoolType, number_type::NumberType, string_type::StringType,
        uint_width::UIntWidth,
    },
    record::record_type::RecordType,
    relation::relation_type::RelationType,
};

use crate::{
    CoreError,
    catalog::{Catalog, CatalogBatchOperation, CollectionKind, IntegrityMode, RELATION_CLASS_ID},
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
    Polymorphic,
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
    UpsertTypeDef {
        type_def: TypeDef,
    },
    DeleteTypeDef {
        name: String,
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
        integrity_mode: IntegrityMode,
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
    let catalog_ops = batch
        .operations
        .iter()
        .map(|op| catalog_batch_operation(&catalog, op))
        .collect::<Result<Vec<_>, _>>()?;
    catalog
        .apply_batch(&catalog_ops)
        .map_err(|err| CoreError::new(err.to_string()))?;

    let mut stats = DdlStats::default();
    for op in &batch.operations {
        match op {
            DdlOperation::DeleteAttribute { .. }
            | DdlOperation::DeleteTypeDef { .. }
            | DdlOperation::DeleteRecordType { .. }
            | DdlOperation::DeleteClass { .. }
            | DdlOperation::DeleteCollection { .. }
            | DdlOperation::DeleteIndex { .. }
            | DdlOperation::DeleteRelationship { .. } => {
                stats.deleted += 1;
            }
            DdlOperation::UpsertAttribute { .. }
            | DdlOperation::UpsertTypeDef { .. }
            | DdlOperation::UpsertRecordType { .. }
            | DdlOperation::UpsertClass { .. }
            | DdlOperation::UpsertCollection { .. }
            | DdlOperation::UpsertIndex { .. }
            | DdlOperation::UpsertRelationship { .. }
            | DdlOperation::SetAutoIndex { .. } => {
                stats.upserted += 1;
            }
        }
    }

    Ok((catalog, DdlOutcome { stats }))
}

pub const CORE_CATALOG_ENTRY_CLASS_ID: &str = "semantic.catalog.entry";
pub const CORE_CATALOG_ATTRIBUTE_ENTRY_CLASS_ID: &str = "semantic.catalog.entry.attribute";
pub const CORE_CATALOG_TYPE_DEF_ENTRY_CLASS_ID: &str = "semantic.catalog.entry.type_def";
pub const CORE_CATALOG_RECORD_TYPE_ENTRY_CLASS_ID: &str = "semantic.catalog.entry.record_type";
pub const CORE_CATALOG_CLASS_ENTRY_CLASS_ID: &str = "semantic.catalog.entry.class";
pub const CORE_CATALOG_COLLECTION_ENTRY_CLASS_ID: &str = "semantic.catalog.entry.collection";
pub const CORE_CATALOG_INDEX_ENTRY_CLASS_ID: &str = "semantic.catalog.entry.index";
pub const CORE_CATALOG_META_ENTRY_CLASS_ID: &str = "semantic.catalog.entry.meta";
pub const CORE_CATALOG_SCHEMA_COLLECTION: &str = "__semantic.catalog.schema";
pub const CORE_CATALOG_ATTRIBUTES_COLLECTION: &str = CORE_CATALOG_SCHEMA_COLLECTION;
pub const CORE_CATALOG_TYPE_DEFS_COLLECTION: &str = CORE_CATALOG_SCHEMA_COLLECTION;
pub const CORE_CATALOG_RECORD_TYPES_COLLECTION: &str = CORE_CATALOG_SCHEMA_COLLECTION;
pub const CORE_CATALOG_CLASSES_COLLECTION: &str = CORE_CATALOG_SCHEMA_COLLECTION;
pub const CORE_CATALOG_COLLECTIONS_COLLECTION: &str = CORE_CATALOG_SCHEMA_COLLECTION;
pub const CORE_CATALOG_INDEXES_COLLECTION: &str = CORE_CATALOG_SCHEMA_COLLECTION;
pub const CORE_CATALOG_META_COLLECTION: &str = CORE_CATALOG_SCHEMA_COLLECTION;

const CORE_CATALOG_ATTR_ID: &str = "semantic.catalog.id";
const CORE_CATALOG_ATTR_LID: &str = "semantic.catalog.lid";
const CORE_CATALOG_ATTR_ATTRIBUTE: &str = "semantic.catalog.attribute";
const CORE_CATALOG_ATTR_TYPE_DEF: &str = "semantic.catalog.type_def";
const CORE_CATALOG_ATTR_RECORD: &str = "semantic.catalog.record";
const CORE_CATALOG_ATTR_CLASS: &str = "semantic.catalog.class";
const CORE_CATALOG_ATTR_NAME: &str = "semantic.catalog.name";
const CORE_CATALOG_ATTR_INTEGRITY_MODE: &str = "semantic.catalog.integrity_mode";
const CORE_CATALOG_ATTR_FIELD_IDS: &str = "semantic.catalog.field_ids";
const CORE_CATALOG_ATTR_COLLECTION: &str = "semantic.catalog.collection";
const CORE_CATALOG_ATTR_FIELD: &str = "semantic.catalog.field";
const CORE_CATALOG_ATTR_INDEX_KIND: &str = "semantic.catalog.index_kind";
const CORE_CATALOG_ATTR_UNIQUE: &str = "semantic.catalog.unique";
const CORE_CATALOG_ATTR_NEXT_FIELD_ID: &str = "semantic.catalog.next_field_id";
const CORE_CATALOG_ATTR_AUTO_INDEX_ENABLED: &str = "semantic.catalog.auto_index_enabled";
const CORE_CATALOG_ATTR_PACKAGES: &str = "semantic.catalog.packages";
const CORE_CATALOG_ATTR_APPLIED_MIGRATIONS: &str = "semantic.catalog.applied_migrations";
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
        "integrity_mode".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_INTEGRITY_MODE.to_string(),
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
    attrs.insert(
        "packages".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_PACKAGES.to_string(),
            },
            required: false,
            constraints: vec![],
            meta: Meta::default(),
        },
    );
    attrs.insert(
        "applied_migrations".to_string(),
        ClassAttribute {
            attribute: AttributeRef {
                id: CORE_CATALOG_ATTR_APPLIED_MIGRATIONS.to_string(),
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
    let catalog_entry_kind = |id: &str, name: &str| ClassType {
        id: id.to_string(),
        name: name.to_string(),
        inherits: Some(ClassRef {
            id: CORE_CATALOG_ENTRY_CLASS_ID.to_string(),
        }),
        extends: vec![],
        attributes: std::collections::BTreeMap::new(),
        constraints: vec![],
        meta: Meta::default(),
    };
    let attribute_entry =
        catalog_entry_kind(CORE_CATALOG_ATTRIBUTE_ENTRY_CLASS_ID, "CatalogAttribute");
    let type_def_entry = catalog_entry_kind(CORE_CATALOG_TYPE_DEF_ENTRY_CLASS_ID, "CatalogTypeDef");
    let record_type_entry =
        catalog_entry_kind(CORE_CATALOG_RECORD_TYPE_ENTRY_CLASS_ID, "CatalogRecordType");
    let class_entry = catalog_entry_kind(CORE_CATALOG_CLASS_ENTRY_CLASS_ID, "CatalogClass");
    let collection_entry =
        catalog_entry_kind(CORE_CATALOG_COLLECTION_ENTRY_CLASS_ID, "CatalogCollection");
    let index_entry = catalog_entry_kind(CORE_CATALOG_INDEX_ENTRY_CLASS_ID, "CatalogIndex");
    let meta_entry = catalog_entry_kind(CORE_CATALOG_META_ENTRY_CLASS_ID, "CatalogMeta");

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
                id: CORE_CATALOG_ATTR_INTEGRITY_MODE.to_string(),
                name: "integrity_mode".to_string(),
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
                id: CORE_CATALOG_ATTR_PACKAGES.to_string(),
                name: "packages".to_string(),
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
                id: CORE_CATALOG_ATTR_APPLIED_MIGRATIONS.to_string(),
                name: "applied_migrations".to_string(),
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
        .with_op(DdlOperation::UpsertClass {
            class: attribute_entry,
        })
        .with_op(DdlOperation::UpsertClass {
            class: type_def_entry,
        })
        .with_op(DdlOperation::UpsertClass {
            class: record_type_entry,
        })
        .with_op(DdlOperation::UpsertClass { class: class_entry })
        .with_op(DdlOperation::UpsertClass {
            class: collection_entry,
        })
        .with_op(DdlOperation::UpsertClass { class: index_entry })
        .with_op(DdlOperation::UpsertClass { class: meta_entry })
        .with_op(DdlOperation::UpsertCollection {
            name: CORE_CATALOG_SCHEMA_COLLECTION.to_string(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::StrictRegisteredSchema,
        })
}

pub fn fresh_catalog_with_core_schema() -> Result<Catalog, CoreError> {
    let (catalog, _) = apply_ddl_batch(&Catalog::new(), &core_catalog_schema_batch())?;
    Ok(catalog)
}

fn resolve_collection_kind(
    _catalog: &Catalog,
    kind: &DdlCollectionKind,
) -> Result<CollectionKind, CoreError> {
    match kind {
        DdlCollectionKind::Polymorphic => Ok(CollectionKind::Polymorphic),
    }
}

fn catalog_batch_operation(
    catalog: &Catalog,
    operation: &DdlOperation,
) -> Result<CatalogBatchOperation, CoreError> {
    match operation {
        DdlOperation::UpsertAttribute { attribute } => Ok(CatalogBatchOperation::UpsertAttribute {
            attribute: attribute.clone(),
            module: None,
        }),
        DdlOperation::DeleteAttribute { id } => {
            Ok(CatalogBatchOperation::DeleteAttribute { id: id.clone() })
        }
        DdlOperation::UpsertTypeDef { type_def } => Ok(CatalogBatchOperation::UpsertTypeDef {
            type_def: type_def.clone(),
        }),
        DdlOperation::DeleteTypeDef { name } => {
            Ok(CatalogBatchOperation::DeleteTypeDef { name: name.clone() })
        }
        DdlOperation::UpsertRecordType { id, name, record } => {
            Ok(CatalogBatchOperation::UpsertRecordType {
                id: id.clone(),
                name: name.clone(),
                record: record.clone(),
                module: None,
            })
        }
        DdlOperation::DeleteRecordType { id } => {
            Ok(CatalogBatchOperation::DeleteRecordType { id: id.clone() })
        }
        DdlOperation::UpsertClass { class } => Ok(CatalogBatchOperation::UpsertClass {
            class: class.clone(),
            module: None,
        }),
        DdlOperation::DeleteClass { id } => {
            Ok(CatalogBatchOperation::DeleteClass { id: id.clone() })
        }
        DdlOperation::UpsertCollection {
            name,
            kind,
            integrity_mode,
        } => Ok(CatalogBatchOperation::UpsertCollection {
            name: name.clone(),
            kind: resolve_collection_kind(catalog, kind)?,
            integrity_mode: *integrity_mode,
        }),
        DdlOperation::DeleteCollection { name } => {
            Ok(CatalogBatchOperation::DeleteCollection { name: name.clone() })
        }
        DdlOperation::UpsertIndex {
            name,
            collection,
            field,
            unique,
        } => Ok(CatalogBatchOperation::UpsertIndex {
            name: name.clone(),
            collection: collection.clone(),
            field: field.clone(),
            unique: *unique,
        }),
        DdlOperation::DeleteIndex { name, collection } => Ok(CatalogBatchOperation::DeleteIndex {
            name: name.clone(),
            collection: collection.clone(),
        }),
        DdlOperation::UpsertRelationship { relationship } => {
            Ok(CatalogBatchOperation::UpsertRelationship {
                relationship: relationship.clone(),
            })
        }
        DdlOperation::DeleteRelationship { id } => {
            Ok(CatalogBatchOperation::DeleteRelationship { id: id.clone() })
        }
        DdlOperation::SetAutoIndex { enabled } => {
            Ok(CatalogBatchOperation::SetAutoIndex { enabled: *enabled })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::schema::{
        AttributeRef, ClassAttribute, collections::list_type::ListType,
        core::visibility::Visibility, record::field::Field,
    };

    use super::*;

    #[test]
    fn ddl_batch_registers_recursive_types_in_multiple_steps() {
        let batch = DdlBatch::new()
            .with_op(DdlOperation::UpsertClass {
                class: recursive_node_class(),
            })
            .with_op(DdlOperation::UpsertAttribute {
                attribute: recursive_payload_attribute(),
            })
            .with_op(DdlOperation::UpsertTypeDef {
                type_def: recursive_payload_type_def(None),
            });

        let (catalog, outcome) = apply_ddl_batch(&Catalog::new(), &batch).unwrap();

        assert_eq!(outcome.stats.upserted, 3);
        assert!(catalog.attribute_id("suite.tree.payload").is_some());
        assert!(catalog.record_type_id("suite.tree.payload_type").is_some());
        let class_id = catalog.class_id("suite.tree.node").unwrap();
        let class = catalog.class_by_lid(class_id).unwrap();
        assert_eq!(
            class.attributes.get("payload"),
            catalog.attribute_id("suite.tree.payload").as_ref()
        );
    }

    fn recursive_node_class() -> ClassType {
        ClassType {
            id: "suite.tree.node".to_string(),
            name: "TreeNode".to_string(),
            inherits: None,
            extends: vec![],
            attributes: BTreeMap::from([(
                "payload".to_string(),
                ClassAttribute {
                    attribute: AttributeRef {
                        id: "suite.tree.payload".to_string(),
                    },
                    required: false,
                    constraints: vec![],
                    meta: Meta::default(),
                },
            )]),
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn recursive_payload_attribute() -> AttributeType {
        AttributeType {
            id: "suite.tree.payload".to_string(),
            name: "payload".to_string(),
            ty: Type {
                kind: TypeKind::Ref(TypeRef {
                    name: "suite.tree.payload_type".to_string(),
                    args: vec![],
                }),
                constraints: vec![],
                annotations: vec![],
                meta: Meta::default(),
            },
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    fn recursive_payload_type_def(module: Option<String>) -> TypeDef {
        TypeDef {
            name: "suite.tree.payload_type".to_string(),
            module,
            params: Vec::new(),
            ty: Type {
                kind: TypeKind::Record(RecordType {
                    fields: BTreeMap::from([(
                        "children".to_string(),
                        Field {
                            ty: Type {
                                kind: TypeKind::List(ListType {
                                    items: Box::new(Type {
                                        kind: TypeKind::Ref(TypeRef {
                                            name: "suite.tree.payload_type".to_string(),
                                            args: vec![],
                                        }),
                                        constraints: vec![],
                                        annotations: vec![],
                                        meta: Meta::default(),
                                    }),
                                }),
                                constraints: vec![],
                                annotations: vec![],
                                meta: Meta::default(),
                            },
                            required: false,
                            readonly: false,
                            writeonly: false,
                            default: None,
                            meta: Meta::default(),
                        },
                    )]),
                    open: false,
                    additional: None,
                    required_order: None,
                }),
                constraints: vec![],
                annotations: vec![],
                meta: Meta::default(),
            },
            visibility: Visibility::Public,
            meta: Meta {
                title: Some("TreePayload".to_string()),
                ..Meta::default()
            },
        }
    }
}
