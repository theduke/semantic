//! Schema operations accepted by the public query AST.

use crate::schema::{AttributeType, ClassType, IndexKind, RecordType, RelationType, TypeDef};

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum IntegrityMode {
    Permissive,
    StrictRegisteredSchema,
}

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
    Schema,
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
        /// The first (for single-column indexes the only) key column.
        field: String,
        unique: bool,
        /// Index kind: equality (the default) or range.
        #[facet(default = IndexKind::Equality)]
        #[facet(skip_serializing_if = IndexKind::is_equality)]
        kind: IndexKind,
        /// Key columns after `field` of a composite index, in key order.
        #[facet(default)]
        #[facet(skip_serializing_if = Vec::is_empty)]
        extra_fields: Vec<String>,
        /// Predicate of a partial index; `None` indexes every row.
        #[facet(default)]
        #[facet(skip_serializing_if = Option::is_none)]
        predicate: Option<super::Expr>,
        /// Tokenization of a full-text index; must be the default for other
        /// kinds.
        #[facet(default)]
        #[facet(skip_serializing_if = super::TextAnalyzer::is_default)]
        analyzer: super::TextAnalyzer,
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

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct DdlQuery {
    pub batch: DdlBatch,
}
