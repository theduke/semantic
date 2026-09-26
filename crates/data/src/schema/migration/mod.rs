use crate::query::{DeleteQuery, UpdateQuery};
use crate::value::Object;

#[derive(facet::Facet, Debug, Clone, PartialEq)]
pub struct Migration {
    pub module: String,
    pub name: String,
    pub description: Option<String>,
    pub operations: Vec<MigrationOperation>,
    pub meta: crate::schema::core::meta::Meta,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationCollectionKind {
    Untyped,
    Schema,
    Polymorphic,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationIntegrityMode {
    Permissive,
    StrictRegisteredSchema,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationDdlOperation {
    UpsertAttribute {
        attribute: crate::schema::attribute::attribute_type::AttributeType,
    },
    DeleteAttribute {
        id: String,
    },
    UpsertTypeDef {
        type_def: crate::schema::core::type_def::TypeDef,
    },
    DeleteTypeDef {
        name: crate::schema::core::type_name::TypeName,
    },
    UpsertRecordType {
        id: String,
        name: String,
        record: crate::schema::record::record_type::RecordType,
    },
    DeleteRecordType {
        id: String,
    },
    UpsertClass {
        class: crate::schema::class::class_type::ClassType,
    },
    DeleteClass {
        id: String,
    },
    UpsertCollection {
        name: String,
        kind: MigrationCollectionKind,
        integrity_mode: MigrationIntegrityMode,
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
        /// Index kind; migrations written before kinds were configurable
        /// create equality indexes.
        #[facet(default = crate::schema::IndexKind::Equality)]
        #[facet(skip_serializing_if = crate::schema::IndexKind::is_equality)]
        kind: crate::schema::IndexKind,
        /// Key columns after `field` of a composite index, in key order.
        #[facet(default)]
        #[facet(skip_serializing_if = Vec::is_empty)]
        extra_fields: Vec<String>,
        /// Predicate of a partial index; `None` indexes every row.
        #[facet(default)]
        #[facet(skip_serializing_if = Option::is_none)]
        predicate: Option<crate::query::Expr>,
        /// Tokenization of a full-text index; must be the default for
        /// other kinds.
        #[facet(default)]
        #[facet(skip_serializing_if = crate::query::TextAnalyzer::is_default)]
        analyzer: crate::query::TextAnalyzer,
    },
    DeleteIndex {
        name: String,
        collection: String,
    },
    UpsertRelationship {
        relationship: crate::schema::relation::relation_type::RelationType,
    },
    DeleteRelationship {
        id: String,
    },
    SetAutoIndex {
        enabled: bool,
    },
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum MigrationOperation {
    Ddl(MigrationDdlOperation),
    Insert {
        collection: String,
        id: String,
        object: Object,
    },
    Update {
        query: UpdateQuery,
    },
    Delete {
        query: DeleteQuery,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{BinaryOp, Expr, Operand};
    use crate::schema::IndexKind;
    use crate::value::{FieldPath, Value};

    fn single_column_index() -> MigrationDdlOperation {
        MigrationDdlOperation::UpsertIndex {
            name: "by_title".to_string(),
            collection: "items".to_string(),
            field: "title".to_string(),
            unique: false,
            kind: IndexKind::Equality,
            extra_fields: Vec::new(),
            predicate: None,
            analyzer: Default::default(),
        }
    }

    #[test]
    fn single_column_index_ops_keep_their_serialized_form() {
        // Persisted migrations written before composite/partial indexes.
        let legacy = r#"{"upsert_index":{"name":"by_title","collection":"items","field":"title","unique":false}}"#;
        let decoded: MigrationDdlOperation = facet_json::from_str(legacy).unwrap();
        assert_eq!(decoded, single_column_index());
        assert_eq!(facet_json::to_string(&decoded).unwrap(), legacy);
    }

    #[test]
    fn composite_partial_index_ops_round_trip() {
        let op = MigrationDdlOperation::UpsertIndex {
            name: "open_by_owner".to_string(),
            collection: "items".to_string(),
            field: "owner".to_string(),
            unique: true,
            kind: IndexKind::Range,
            extra_fields: vec!["due".to_string()],
            predicate: Some(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "status",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                    "open".to_string(),
                )))),
            }),
            analyzer: Default::default(),
        };
        let encoded = facet_json::to_string(&op).unwrap();
        let decoded: MigrationDdlOperation = facet_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, op);
    }

    #[test]
    fn full_text_index_ops_round_trip() {
        let index = |extra_fields: Vec<String>, analyzer| MigrationDdlOperation::UpsertIndex {
            name: "search".to_string(),
            collection: "items".to_string(),
            field: "title".to_string(),
            unique: false,
            kind: IndexKind::FullText,
            extra_fields,
            predicate: None,
            analyzer,
        };
        let op = index(
            vec!["tags".to_string()],
            crate::query::TextAnalyzer {
                stemming: true,
                min_token_len: 2,
            },
        );
        let encoded = facet_json::to_string(&op).unwrap();
        assert!(encoded.contains(r#""analyzer":{"stemming":true,"min_token_len":2}"#));
        let decoded: MigrationDdlOperation = facet_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, op);

        // Default analyzers are omitted.
        assert_eq!(
            facet_json::to_string(&index(Vec::new(), Default::default())).unwrap(),
            r#"{"upsert_index":{"name":"search","collection":"items","field":"title","unique":false,"kind":"full_text"}}"#
        );
    }
}
