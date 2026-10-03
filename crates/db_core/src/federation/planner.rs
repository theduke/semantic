use std::collections::BTreeMap;
use std::sync::Arc;

use semantic_data::builtin::DEFAULT_COLLECTION;
use semantic_data::query::{DdlBatch, DdlOperation, FieldFormat};

use super::{FederationSources, LOCAL_SOURCE_TAG, VirtualSource};
use crate::catalog::{Catalog, CollectionKind, IntegrityMode};
use crate::{DbError, Expr, LogicalPlan, Optimizer, QueryContext, SelectQuery, SourceRef};

fn definition_id(operation: &DdlOperation) -> Result<&str, String> {
    match operation {
        DdlOperation::UpsertTypeDef { type_def } => Ok(&type_def.name),
        DdlOperation::UpsertAttribute { attribute } => Ok(&attribute.id),
        DdlOperation::UpsertClass { class } => Ok(&class.id),
        DdlOperation::UpsertRelationship { relationship } => Ok(&relationship.id),
        _ => Err(
            "virtual schemas may contain only type, attribute, class and relationship upserts"
                .into(),
        ),
    }
}

/// Validate exposed runtime definitions without modifying the local catalog.
/// Existing local definitions may be referenced, but must never be redeclared.
fn validate_declarations(local: &Catalog, schema: &DdlBatch) -> Result<(), String> {
    let mut declared = BTreeMap::new();
    for operation in &schema.operations {
        let id = definition_id(operation)?;
        if local.type_def_by_name(id).is_some()
            || local.attribute_by_id(id).is_some()
            || local.class_id(id).is_some()
            || local.relationship_by_id(id).is_some()
        {
            return Err(format!("virtual schema redefines local definition '{id}'"));
        }
        if let Some(previous) = declared.insert(id, operation)
            && previous != operation
        {
            return Err(format!("conflicting virtual schema definition '{id}'"));
        }
    }
    Ok(())
}

/// Validate exposed definitions whose relationships use existing collections.
pub fn validate_virtual_schema(local: &Catalog, schema: &DdlBatch) -> Result<(), String> {
    validate_declarations(local, schema)?;
    crate::apply_ddl_batch(local, schema)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Validate exposed definitions with their own runtime collection available.
/// Relationship source collections must use the effective activation name.
pub fn validate_virtual_schema_for_collection(
    local: &Catalog,
    name: &str,
    schema: &DdlBatch,
) -> Result<(), String> {
    if local.collection_by_name(name).is_some() {
        return Err(format!(
            "virtual collection '{name}' conflicts with a local collection"
        ));
    }
    validate_declarations(local, schema)?;
    let mut catalog = local.clone();
    catalog
        .upsert_collection(name, CollectionKind::Polymorphic, IntegrityMode::Permissive)
        .map_err(|error| error.to_string())?;
    crate::apply_ddl_batch(&catalog, schema)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) fn overlay_catalog(
    local: &Catalog,
    virtual_sources: &[(&str, &VirtualSource)],
) -> Result<Catalog, DbError> {
    let mut overlay = local.clone();
    let mut declared = BTreeMap::<&str, (&str, &DdlOperation)>::new();
    // Collection shells must exist before relationship definitions are applied.
    for (name, _) in virtual_sources {
        if local.collection_by_name(name).is_some() {
            return Err(DbError::InvalidQuery(format!(
                "virtual collection '{name}' conflicts with a local collection"
            )));
        }
        overlay
            .upsert_collection(
                *name,
                CollectionKind::Polymorphic,
                IntegrityMode::Permissive,
            )
            .map_err(|error| DbError::InvalidQuery(error.to_string()))?;
    }
    for (name, source) in virtual_sources {
        validate_virtual_schema_for_collection(local, name, &source.schema)
            .map_err(|reason| DbError::InvalidQuery(format!("{name}: {reason}")))?;
        for operation in &source.schema.operations {
            let id = definition_id(operation).map_err(DbError::InvalidQuery)?;
            if let Some((previous_name, previous)) = declared.insert(id, (name, operation))
                && previous != operation
            {
                return Err(DbError::InvalidQuery(format!(
                    "virtual schemas '{previous_name}' and '{name}' conflict on '{id}'"
                )));
            }
        }
        overlay = crate::apply_ddl_batch(&overlay, &source.schema)
            .map_err(|error| DbError::InvalidQuery(format!("{name}: {error}")))?
            .0;
    }
    // Collection creation adds builtin indexes. Remove all indexes last.
    let indexes = overlay
        .indexes()
        .map(|(_, index)| (index.collection, index.schema.name.clone()))
        .collect::<Vec<_>>();
    for (collection, name) in indexes {
        overlay.delete_index(collection, &name);
    }
    Ok(overlay)
}

pub(crate) struct PlannedSelect {
    pub overlay: Arc<Catalog>,
    pub logical: LogicalPlan,
    pub field_format: FieldFormat,
}

pub(crate) fn plan_select(
    query: SelectQuery,
    overlay: Arc<Catalog>,
    sources: &FederationSources,
    local: &Catalog,
) -> Result<PlannedSelect, DbError> {
    let mut query = semantic_data::query::Query::Select(query.into());
    super::normalize_virtual_joins(
        &mut query,
        local,
        &sources.virtual_sources.keys().cloned().collect(),
    );
    let semantic_data::query::Query::Select(query) = query else {
        unreachable!()
    };
    let query: SelectQuery = query.into();
    let name = query.collection.as_deref().unwrap_or(DEFAULT_COLLECTION);
    let collection =
        overlay
            .collection_by_name(name)
            .ok_or_else(|| DbError::UnknownCollectionByName {
                name: name.to_owned(),
            })?;
    let query = crate::canonicalize_select_query(&query, &overlay, collection)?;
    let source = crate::source_ref_for_collection(
        Some(name.into()),
        query.source_alias.clone(),
        Some(collection.lid),
    );
    let context = QueryContext::new(overlay.clone());
    let mut logical = Optimizer::core()
        .optimize_query_with_source(&query, source, None, &context)
        .logical;
    visit_sources(&mut logical, &mut |source| {
        let name = source
            .source_name
            .clone()
            .unwrap_or_else(|| DEFAULT_COLLECTION.into());
        let collection =
            overlay
                .collection_by_name(&name)
                .ok_or_else(|| DbError::UnknownCollectionByName {
                    name: name.to_owned(),
                })?;
        source.collection_id = Some(collection.lid);
        source.source_name = Some(name.clone());
        source.backend_tag = Some(if sources.virtual_sources.contains_key(&name) {
            name.into()
        } else {
            LOCAL_SOURCE_TAG.into()
        });
        Ok(())
    })?;
    Ok(PlannedSelect {
        overlay,
        logical,
        field_format: query.field_format,
    })
}

/// Visit source references without duplicating the shape-preserving rewrite
/// used by the legacy backend's namespace resolver.
fn visit_sources(
    plan: &mut LogicalPlan,
    visit: &mut impl FnMut(&mut SourceRef) -> Result<(), DbError>,
) -> Result<(), DbError> {
    match plan {
        LogicalPlan::Source { source, .. } => visit(source)?,
        LogicalPlan::Values { .. } => {}
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Project { input, .. }
        | LogicalPlan::Aggregate { input, .. }
        | LogicalPlan::Limit { input, .. }
        | LogicalPlan::Distinct { input }
        | LogicalPlan::Exchange { input, .. }
        | LogicalPlan::RepartitionHash { input, .. } => visit_sources(input, visit)?,
        LogicalPlan::Union { inputs, .. } => {
            for input in inputs {
                visit_sources(input, visit)?;
            }
        }
        LogicalPlan::Join(join) => {
            visit_sources(&mut join.left, visit)?;
            visit_sources(&mut join.right, visit)?;
        }
        LogicalPlan::ApplyExists {
            input, subquery, ..
        }
        | LogicalPlan::ApplyInSubquery {
            input, subquery, ..
        } => {
            visit_sources(input, visit)?;
            visit_sources(subquery, visit)?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LeafKey {
    pub backend_tag: String,
    pub source_name: String,
    pub binding: Option<String>,
}

impl From<&SourceRef> for LeafKey {
    fn from(source: &SourceRef) -> Self {
        Self {
            backend_tag: source
                .backend_tag
                .clone()
                .unwrap_or_else(|| LOCAL_SOURCE_TAG.into()),
            source_name: source
                .source_name
                .clone()
                .unwrap_or_else(|| DEFAULT_COLLECTION.into()),
            binding: source.binding.clone(),
        }
    }
}

pub(crate) struct LeafInfo {
    pub key: LeafKey,
    pub pushed_predicate: Option<Expr>,
    pub sole_input: bool,
}

pub(crate) fn collect_leaves(plan: &LogicalPlan) -> Vec<LeafInfo> {
    fn visit(plan: &LogicalPlan, sole_input: bool, leaves: &mut Vec<LeafInfo>) {
        match plan {
            LogicalPlan::Source {
                source,
                pushed_predicate,
            } => leaves.push(LeafInfo {
                key: source.into(),
                pushed_predicate: pushed_predicate.clone(),
                sole_input,
            }),
            LogicalPlan::Values { .. } => {}
            LogicalPlan::Filter { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Project { input, .. }
            | LogicalPlan::Limit { input, .. } => visit(input, sole_input, leaves),
            LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Distinct { input }
            | LogicalPlan::Exchange { input, .. }
            | LogicalPlan::RepartitionHash { input, .. } => visit(input, false, leaves),
            LogicalPlan::Union { inputs, .. } => {
                for input in inputs {
                    visit(input, false, leaves);
                }
            }
            LogicalPlan::Join(join) => {
                visit(&join.left, false, leaves);
                visit(&join.right, false, leaves);
            }
            LogicalPlan::ApplyExists {
                input, subquery, ..
            }
            | LogicalPlan::ApplyInSubquery {
                input, subquery, ..
            } => {
                visit(input, false, leaves);
                visit(subquery, false, leaves);
            }
        }
    }
    let mut leaves = Vec::new();
    visit(plan, true, &mut leaves);
    leaves
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{Operand, QuerySource, SendableRecordBatchStream, SourceScan};
    use async_trait::async_trait;
    use futures::{StreamExt, stream};
    use semantic_data::query::BinaryOp;
    use semantic_data::schema::{
        AttributeRef, AttributeType, ClassAttribute, ClassType, Meta, StringType, Type, TypeKind,
    };
    use semantic_data::value::{FieldPath, Value};
    use semantic_data::vdb::{DatabaseSchema, ScanPlan, ScanRequest};

    struct UnusedSource;
    #[async_trait]
    impl QuerySource for UnusedSource {
        async fn negotiate(&self, _: &str, _: &ScanRequest) -> Result<ScanPlan, DbError> {
            unreachable!("planning does not negotiate")
        }
        fn scan(self: Arc<Self>, _: SourceScan) -> SendableRecordBatchStream {
            stream::empty().boxed()
        }
    }

    pub(crate) fn attribute(id: &str) -> AttributeType {
        AttributeType {
            id: id.into(),
            name: id.rsplit(':').next().unwrap().into(),
            ty: Type::new(TypeKind::String(StringType {
                format: None,
                normalization: None,
            })),
            constraints: vec![],
            meta: Meta::default(),
        }
    }

    pub(crate) fn class(id: &str, attributes: &[&str]) -> ClassType {
        let mut class = semantic_data::filestore::file_class();
        class.id = id.into();
        class.name = id.into();
        class.inherits = None;
        class.extends.clear();
        class.constraints.clear();
        class.attributes = attributes
            .iter()
            .map(|id| {
                (
                    (*id).into(),
                    ClassAttribute {
                        attribute: AttributeRef { id: (*id).into() },
                        required: false,
                        ui_order: None,
                        computed: None,
                        default: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )
            })
            .collect();
        class
    }

    pub(crate) fn setup() -> (Arc<Catalog>, FederationSources) {
        let mut local = Catalog::new();
        local.upsert_attribute(attribute("shared:owner"));
        local.upsert_attribute(attribute("local:name"));
        local
            .upsert_class(class("local:Item", &["shared:owner", "local:name"]))
            .unwrap();
        let lid = local
            .upsert_collection("local", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        local
            .upsert_index("name", lid, "local:name", false)
            .unwrap();
        let schema = DatabaseSchema {
            attributes: vec![attribute("virtual:title")],
            classes: vec![class("virtual:Item", &["shared:owner", "virtual:title"])],
            ..Default::default()
        };
        let source = Arc::new(UnusedSource);
        let sources = FederationSources {
            local: source.clone(),
            virtual_sources: BTreeMap::from([(
                "fx".into(),
                VirtualSource {
                    source,
                    schema: Arc::new(schema.to_ddl_batch()),
                    schema_revision: "1".into(),
                },
            )]),
        };
        (Arc::new(local), sources)
    }

    fn overlay(local: &Catalog, sources: &FederationSources) -> Arc<Catalog> {
        Arc::new(
            overlay_catalog(
                local,
                &sources
                    .virtual_sources
                    .iter()
                    .map(|(n, s)| (n.as_str(), s))
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        )
    }

    #[test]
    fn validates_reuse_and_rejects_invalid_runtime_definitions() {
        let (local, sources) = setup();
        assert!(validate_virtual_schema(&local, &sources.virtual_sources["fx"].schema).is_ok());
        for schema in [
            DatabaseSchema {
                attributes: vec![attribute("shared:owner")],
                ..Default::default()
            }
            .to_ddl_batch(),
            DatabaseSchema {
                classes: vec![class("broken:Item", &["missing:attribute"])],
                ..Default::default()
            }
            .to_ddl_batch(),
            DdlBatch {
                operations: vec![DdlOperation::DeleteClass {
                    id: "local:Item".into(),
                }],
            },
        ] {
            assert!(validate_virtual_schema(&local, &schema).is_err());
        }
    }

    #[test]
    fn overlay_is_runtime_only_and_canonicalizes_virtual_attributes() {
        let (local, sources) = setup();
        let snapshot = local.to_storage_snapshot();
        let overlay = overlay(&local, &sources);
        let predicate = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "title",
            ])))),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                "hello".into(),
            )))),
        };
        let planned = plan_select(
            SelectQuery::new()
                .with_collection("fx")
                .with_predicate(predicate),
            overlay,
            &sources,
            &local,
        )
        .unwrap();
        let leaves = collect_leaves(&planned.logical);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].key.backend_tag, "fx");
        assert!(leaves[0].sole_input);
        assert!(format!("{:?}", leaves[0].pushed_predicate).contains("virtual:title"));
        assert_eq!(local.to_storage_snapshot(), snapshot);
        assert!(local.class_id("virtual:Item").is_none());
    }

    #[cfg(feature = "sql")]
    #[test]
    fn join_leaves_remain_separate_and_lower_without_indexes() {
        let (local, sources) = setup();
        let overlay = overlay(&local, &sources);
        assert_eq!(overlay.indexes().count(), 0);
        assert!(local.indexes().count() > 0);
        let query = crate::sql::parse_sql_query_unbound(
            "SELECT a.id AS local_id, b.id AS vdb_id FROM local a JOIN fx b ON a.id = b.id WHERE a.name = 'x' AND b.title = 'y'",
            crate::sql::SqlDialectKind::Generic).unwrap();
        let semantic_data::query::Query::Select(query) = query else {
            panic!("select")
        };
        let planned = plan_select(query.into(), overlay.clone(), &sources, &local).unwrap();
        let leaves = collect_leaves(&planned.logical);
        assert_eq!(leaves.len(), 2);
        assert!(
            leaves
                .iter()
                .all(|leaf| !leaf.sole_input && leaf.pushed_predicate.is_some())
        );
        assert_eq!(leaves[0].key.backend_tag, LOCAL_SOURCE_TAG);
        assert_eq!(leaves[1].key.backend_tag, "fx");
        let physical = Optimizer::core().lower_to_physical(
            &planned.logical,
            None,
            &QueryContext::new(overlay),
        );
        let debug = format!("{physical:?}");
        for unsupported in ["IndexLookup", "IndexRange", "TextSearch", "IndexNestedLoop"] {
            assert!(!debug.contains(unsupported), "{debug}");
        }
    }

    #[test]
    fn own_collection_relationships_validate_without_persistence() {
        let (local, sources) = setup();
        let mut source = sources.virtual_sources["fx"].clone();
        let mut schema = (*source.schema).clone();
        schema.operations.push(DdlOperation::UpsertRelationship {
            relationship: semantic_data::schema::RelationType {
                id: "virtual:links".into(),
                name: "Links".into(),
                source_collection: "fx".into(),
                mode: semantic_data::schema::RelationMode::External,
                indexing_mode: semantic_data::schema::RelationIndexingMode::Disabled,
                meta: Meta::default(),
            },
        });
        assert!(validate_virtual_schema(&local, &schema).is_err());
        assert!(validate_virtual_schema_for_collection(&local, "fx", &schema).is_ok());
        source.schema = Arc::new(schema);
        let overlay = overlay_catalog(&local, &[("fx", &source)]).unwrap();
        assert!(overlay.relationship_by_id("virtual:links").is_some());
        assert!(local.relationship_by_id("virtual:links").is_none());
        assert_eq!(overlay.indexes().count(), 0);
    }

    #[test]
    fn virtual_conflicts_are_rejected_and_equal_definitions_are_shared() {
        let (local, sources) = setup();
        let first = &sources.virtual_sources["fx"];
        assert!(overlay_catalog(&local, &[("local", first)]).is_err());
        assert!(overlay_catalog(&local, &[("fx", first), ("fx2", first)]).is_ok());
        let mut conflicting = first.clone();
        conflicting.schema = Arc::new(
            DatabaseSchema {
                attributes: vec![AttributeType {
                    name: "Changed".into(),
                    ..attribute("virtual:title")
                }],
                ..Default::default()
            }
            .to_ddl_batch(),
        );
        assert!(overlay_catalog(&local, &[("fx", first), ("fx2", &conflicting)]).is_err());
    }
}
