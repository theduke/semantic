//! Runtime-only virtual database interface. Exposed database schemas are never
//! persisted by this package; persisted definitions require a new migration.

use std::collections::BTreeMap;

use crate::query;
use crate::schema::*;
use crate::value::{FromValue, IntoValue, SemanticType, Value};

pub const PACKAGE_NAME: &str = "semantic.vdb";
pub const MODULE_NAME: &str = "v1";
pub const INTERFACE_NAME: &str = "VirtualDatabase";

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq)]
pub struct DatabaseDescriptor {
    pub title: String,
    pub description: Option<String>,
    /// Exactly the definitions exposed by the collection.
    pub schema: DatabaseSchema,
    /// Changes whenever the exposed schema changes.
    pub schema_revision: String,
    #[semantic(default)]
    pub allow_untyped: bool,
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, Default, PartialEq)]
pub struct DatabaseSchema {
    #[semantic(default)]
    pub types: Vec<TypeDef>,
    #[semantic(default)]
    pub attributes: Vec<AttributeType>,
    #[semantic(default)]
    pub classes: Vec<ClassType>,
    #[semantic(default)]
    pub relationships: Vec<RelationType>,
}

impl DatabaseSchema {
    /// Upsert-only DDL, ordered as types, attributes, classes, relationships.
    /// Applying it to an in-memory catalog does not persist the definitions.
    pub fn to_ddl_batch(&self) -> query::DdlBatch {
        let operations =
            self.types
                .iter()
                .cloned()
                .map(|type_def| query::DdlOperation::UpsertTypeDef { type_def })
                .chain(
                    self.attributes
                        .iter()
                        .cloned()
                        .map(|attribute| query::DdlOperation::UpsertAttribute { attribute }),
                )
                .chain(
                    self.classes
                        .iter()
                        .cloned()
                        .map(|class| query::DdlOperation::UpsertClass { class }),
                )
                .chain(
                    self.relationships.iter().cloned().map(|relationship| {
                        query::DdlOperation::UpsertRelationship { relationship }
                    }),
                )
                .collect();
        query::DdlBatch { operations }
    }

    pub fn class_ids(&self) -> impl Iterator<Item = &str> {
        self.classes.iter().map(|class| class.id.as_str())
    }
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
#[semantic(rename_all = "snake_case")]
pub enum FilterSupport {
    Exact,
    Inexact,
    Unsupported,
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, Default, PartialEq)]
pub struct ScanRequest {
    /// AND-ed, canonical entity-relative expressions.
    pub filters: Vec<query::Expr>,
    pub order_by: Vec<query::OrderBy>,
    pub limit: Option<u64>,
    #[semantic(default)]
    pub offset: u64,
    /// Attribute ids needed by the host; a hint, not a restriction.
    pub projection: Option<Vec<String>>,
    /// Parameters bound separately on each scan, for bind joins.
    #[semantic(default)]
    pub parameters: Vec<String>,
    pub fetch_hint: Option<u64>,
}

/// The accepted payload is nested under `plan` on the wire. Struct variants
/// allow the existing tagged-enum derives to encode the complete contract.
#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
#[semantic(tag = "status", rename_all = "snake_case")]
pub enum ScanPlan {
    Accepted { plan: AcceptedScan },
    Rejected { reason: String },
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq)]
pub struct AcceptedScan {
    /// One entry for every request filter, in the same order.
    pub filters: Vec<FilterSupport>,
    pub ordered_prefix: u64,
    pub limit_applied: bool,
    pub offset_applied: bool,
    pub estimated_rows: Option<u64>,
    /// Opaque plugin value, echoed unchanged in scan.
    pub token: Option<Value>,
    pub schema_revision: String,
}

impl AcceptedScan {
    pub fn all_exact(&self) -> bool {
        self.filters
            .iter()
            .all(|support| *support == FilterSupport::Exact)
    }

    /// Fail a malformed negotiation rather than repairing its guarantees.
    pub fn validate(&self, request: &ScanRequest) -> Result<(), VdbError> {
        let violation = |message: &str| VdbError {
            code: "protocol_violation".into(),
            message: message.into(),
        };
        if self.filters.len() != request.filters.len() {
            return Err(violation("filter support count must match the request"));
        }
        if self.ordered_prefix > request.order_by.len() as u64 {
            return Err(violation("ordered prefix exceeds the requested ordering"));
        }
        let fully_ordered = self.ordered_prefix == request.order_by.len() as u64;
        if self.limit_applied && (request.limit.is_none() || !self.all_exact() || !fully_ordered) {
            return Err(violation(
                "applied limit requires a requested limit, exact filters and full ordering",
            ));
        }
        if self.offset_applied && (!self.all_exact() || !fully_ordered) {
            return Err(violation(
                "applied offset requires exact filters and full ordering",
            ));
        }
        Ok(())
    }
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct ScanSummary {
    pub rows: u64,
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct VdbError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for VdbError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for VdbError {}

pub fn package() -> Package {
    let entity = Type::new(TypeKind::Record(RecordType {
        fields: BTreeMap::new(),
        open: true,
        additional: None,
        required_order: None,
    }));
    let method = |name: &str, params: Vec<(&str, Type)>, result: Type| InterfaceMethod {
        name: name.into(),
        signature: FunctionType {
            params: params
                .into_iter()
                .map(|(name, ty)| FunctionParam {
                    name: Some(name.into()),
                    ty,
                })
                .collect(),
            results: vec![result],
            throws: Some(Box::new(VdbError::semantic_type())),
            async_fn: true,
        },
    };
    let interface = InterfaceType {
        methods: vec![
            method("describe", vec![], DatabaseDescriptor::semantic_type()),
            method(
                "negotiate",
                vec![("request", ScanRequest::semantic_type())],
                ScanPlan::semantic_type(),
            ),
            method(
                "scan",
                vec![
                    ("request", ScanRequest::semantic_type()),
                    ("plan", AcceptedScan::semantic_type()),
                    ("bindings", entity.clone()),
                ],
                Type::new(TypeKind::Stream(StreamType {
                    element: Box::new(entity),
                    end: Some(Box::new(ScanSummary::semantic_type())),
                })),
            ),
        ],
    };
    Package {
        name: PACKAGE_NAME.into(),
        root: Module {
            name: MODULE_NAME.into(),
            constants: BTreeMap::new(),
            types: BTreeMap::new(),
            attributes: BTreeMap::new(),
            classes: BTreeMap::new(),
            interfaces: BTreeMap::from([(INTERFACE_NAME.into(), interface)]),
            contracts: BTreeMap::new(),
            meta: Meta::default(),
        },
        modules: BTreeMap::new(),
        migrations: vec![],
        version: Some(SchemaVersion {
            major: 1,
            minor: 0,
            patch: 0,
            pre: None,
            build: None,
        }),
        meta: Meta::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{BinaryOp, Expr, Operand, OrderBy, SortDirection};
    use crate::value::FieldPath;

    fn request() -> ScanRequest {
        ScanRequest {
            filters: vec![Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "id",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String("one".into())))),
            }],
            order_by: vec![OrderBy {
                expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                direction: SortDirection::Asc,
            }],
            limit: Some(1),
            offset: 2,
            projection: Some(vec!["id".into()]),
            parameters: vec!["__keys".into()],
            fetch_hint: Some(3),
        }
    }

    fn accepted() -> AcceptedScan {
        AcceptedScan {
            filters: vec![FilterSupport::Exact],
            ordered_prefix: 1,
            limit_applied: true,
            offset_applied: true,
            estimated_rows: Some(3),
            token: Some(Value::String("opaque".into())),
            schema_revision: "1".into(),
        }
    }

    fn round_trip<T: IntoValue + FromValue + Clone + PartialEq + std::fmt::Debug>(value: T) {
        assert_eq!(T::from_value(value.clone().into_value()).unwrap(), value);
    }

    #[test]
    fn dto_round_trip() {
        round_trip(DatabaseDescriptor {
            title: "Test".into(),
            description: Some("A test VDB".into()),
            schema: DatabaseSchema::default(),
            schema_revision: "1".into(),
            allow_untyped: true,
        });
        round_trip(DatabaseSchema::default());
        for support in [
            FilterSupport::Exact,
            FilterSupport::Inexact,
            FilterSupport::Unsupported,
        ] {
            round_trip(support);
        }
        round_trip(request());
        round_trip(accepted());
        round_trip(ScanPlan::Accepted { plan: accepted() });
        round_trip(ScanPlan::Rejected {
            reason: "Needs a filter".into(),
        });
        round_trip(ScanSummary { rows: 3 });
        round_trip(VdbError {
            code: "unavailable".into(),
            message: "Offline".into(),
        });
        let Value::Object(rejected) = (ScanPlan::Rejected {
            reason: "no".into(),
        })
        .into_value() else {
            panic!("scan plans must encode as objects");
        };
        assert_eq!(
            rejected.get("status"),
            Some(&Value::String("rejected".into()))
        );
    }

    #[test]
    fn validate_rules() {
        let request = request();
        let accepted = accepted();
        assert!(accepted.validate(&request).is_ok());
        assert!(accepted.all_exact());
        let mut invalid = Vec::new();
        let mut plan = accepted.clone();
        plan.filters.clear();
        invalid.push((request.clone(), plan));
        let mut plan = accepted.clone();
        plan.ordered_prefix = 2;
        invalid.push((request.clone(), plan));
        let mut no_limit = request.clone();
        no_limit.limit = None;
        invalid.push((no_limit, accepted.clone()));
        for support in [FilterSupport::Inexact, FilterSupport::Unsupported] {
            let mut plan = accepted.clone();
            plan.filters[0] = support;
            invalid.push((request.clone(), plan));
        }
        let mut plan = accepted.clone();
        plan.ordered_prefix = 0;
        invalid.push((request.clone(), plan));
        let mut plan = accepted.clone();
        plan.limit_applied = false;
        plan.filters[0] = FilterSupport::Inexact;
        invalid.push((request.clone(), plan));
        let mut plan = accepted.clone();
        plan.limit_applied = false;
        plan.ordered_prefix = 0;
        invalid.push((request.clone(), plan));
        for (request, plan) in invalid {
            assert_eq!(
                plan.validate(&request).unwrap_err().code,
                "protocol_violation"
            );
        }
        for support in [
            FilterSupport::Exact,
            FilterSupport::Inexact,
            FilterSupport::Unsupported,
        ] {
            let mut plan = accepted.clone();
            plan.filters[0] = support;
            plan.ordered_prefix = 0;
            plan.limit_applied = false;
            plan.offset_applied = false;
            assert!(plan.validate(&request).is_ok());
            assert_eq!(plan.all_exact(), support == FilterSupport::Exact);
        }
        assert!(
            AcceptedScan {
                filters: vec![],
                ordered_prefix: 0,
                limit_applied: false,
                offset_applied: true,
                estimated_rows: None,
                token: None,
                schema_revision: "1".into()
            }
            .validate(&ScanRequest::default())
            .is_ok()
        );
    }

    #[test]
    fn schema_round_trip_and_ddl() {
        let ty = Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }));
        let schema = DatabaseSchema {
            types: vec![TypeDef {
                name: "test:Name".into(),
                module: None,
                params: vec![],
                ty: ty.clone(),
                visibility: Visibility::Public,
                meta: Meta::default(),
            }],
            attributes: ["test:title", "test:tag"]
                .into_iter()
                .map(|id| AttributeType {
                    id: id.into(),
                    name: id.into(),
                    ty: ty.clone(),
                    constraints: vec![],
                    meta: Meta::default(),
                })
                .collect(),
            classes: vec![ClassType {
                id: "test:Item".into(),
                name: "Item".into(),
                inherits: None,
                extends: vec![],
                strict_schema: false,
                creatable_in_ui: None,
                include_in_ui_listings: None,
                attributes: BTreeMap::from([(
                    "title".into(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: "test:title".into(),
                        },
                        required: true,
                        ui_order: None,
                        computed: None,
                        default: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )]),
                constraints: vec![],
                meta: Meta::default(),
            }],
            relationships: vec![RelationType {
                id: "test:Related".into(),
                name: "Related".into(),
                source_collection: "test".into(),
                mode: RelationMode::External,
                indexing_mode: RelationIndexingMode::Disabled,
                meta: Meta::default(),
            }],
        };
        round_trip(schema.clone());
        assert_eq!(schema.class_ids().collect::<Vec<_>>(), vec!["test:Item"]);
        assert_eq!(
            schema.to_ddl_batch().operations,
            vec![
                query::DdlOperation::UpsertTypeDef {
                    type_def: schema.types[0].clone()
                },
                query::DdlOperation::UpsertAttribute {
                    attribute: schema.attributes[0].clone()
                },
                query::DdlOperation::UpsertAttribute {
                    attribute: schema.attributes[1].clone()
                },
                query::DdlOperation::UpsertClass {
                    class: schema.classes[0].clone()
                },
                query::DdlOperation::UpsertRelationship {
                    relationship: schema.relationships[0].clone()
                },
            ]
        );
    }

    #[test]
    fn interface_fingerprints() {
        let package = package();
        let interface = &package.root.interfaces[INTERFACE_NAME];
        assert!(
            crate::schema::interface_fingerprint(interface, &query::semantic::definitions())
                .is_ok()
        );
        assert!(package.migrations.is_empty());
        assert!(package.root.types.is_empty());
        assert!(package.root.attributes.is_empty());
        assert!(package.root.classes.is_empty());
        assert_eq!(interface.methods.len(), 3);
        assert!(
            interface
                .methods
                .iter()
                .all(|method| method.signature.async_fn && method.signature.throws.is_some())
        );
    }
}
