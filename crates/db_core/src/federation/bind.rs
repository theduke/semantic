//! Federation-only lookup selection. Core optimizer statistics remain unchanged.
use std::collections::BTreeMap;

use semantic_data::query::JoinType;
use semantic_data::value::PathSegment;

use super::FederationSources;
use super::planner::LeafKey;
use super::pushdown::LeafFragment;
use crate::{
    Expr, FieldRef, Operand, PhysicalIndexProbe, PhysicalJoinAlgorithm, PhysicalJoinCondition,
    PhysicalPlan, PhysicalSource,
};

pub(crate) const BIND_JOIN_BATCH_SIZE: usize = 64;
pub(crate) const KEYS_PARAMETER: &str = "__keys";
pub(crate) const BIND_JOIN_THRESHOLD: u64 = 10_000;

#[derive(Clone)]
pub(crate) struct BindCandidate {
    pub field: FieldRef,
    pub key: Expr,
    pub index_field: String,
}

fn visit(plan: &mut PhysicalPlan, visitor: &mut impl FnMut(&mut PhysicalPlan)) {
    visitor(plan);
    match plan {
        PhysicalPlan::Source(_) | PhysicalPlan::Values { .. } => {}
        PhysicalPlan::Filter { input, .. }
        | PhysicalPlan::Sort { input, .. }
        | PhysicalPlan::TopN { input, .. }
        | PhysicalPlan::Project { input, .. }
        | PhysicalPlan::Aggregate { input, .. }
        | PhysicalPlan::Limit { input, .. }
        | PhysicalPlan::Distinct { input }
        | PhysicalPlan::Exchange { input, .. }
        | PhysicalPlan::RepartitionHash { input, .. }
        | PhysicalPlan::Materialize { input } => visit(input, visitor),
        PhysicalPlan::Union { inputs, .. } => {
            for input in inputs {
                visit(input, visitor);
            }
        }
        PhysicalPlan::Join(join) => {
            visit(&mut join.left, visitor);
            visit(&mut join.right, visitor);
        }
        PhysicalPlan::ApplyExists {
            input, subquery, ..
        }
        | PhysicalPlan::ApplyInSubquery {
            input, subquery, ..
        } => {
            visit(input, visitor);
            visit(subquery, visitor);
        }
    }
}

pub(crate) fn candidates(
    plan: &PhysicalPlan,
    sources: &FederationSources,
    overlay: &crate::catalog::Catalog,
) -> Result<BTreeMap<LeafKey, BindCandidate>, crate::DbError> {
    let mut candidates = BTreeMap::new();
    visit(&mut plan.clone(), &mut |plan| {
        let PhysicalPlan::Join(join) = plan else {
            return;
        };
        if join.join_type != JoinType::Inner {
            return;
        }
        let PhysicalJoinCondition::Eq { right, .. } = &join.condition else {
            return;
        };
        let PhysicalPlan::Source(
            PhysicalSource::Scan { source } | PhysicalSource::FilteredScan { source, .. },
        ) = join.right.as_ref()
        else {
            return;
        };
        let leaf = LeafKey::from(source);
        if !sources.virtual_sources.contains_key(&leaf.source_name) {
            return;
        }
        // Join lowering strips the binding; field aliases still need resolution.
        // Synthetic indexes describe scalar fields, not nested expressions.
        let [PathSegment::Field(name)] = right.source_path.segments() else {
            return;
        };
        candidates.insert(
            leaf,
            BindCandidate {
                field: FieldRef::Path(right.source_path.clone()),
                key: Expr::Operand(Operand::Field(right.source_path.clone())),
                index_field: name.clone(),
            },
        );
    });
    for (leaf, candidate) in &mut candidates {
        let schema = overlay
            .collection_by_name(&leaf.source_name)
            .ok_or_else(|| crate::DbError::UnknownCollectionByName {
                name: leaf.source_name.clone(),
            })?;
        candidate.key = crate::canonical::canonicalize_filter_expr(&candidate.key, overlay, schema)
            .map_err(crate::DbError::from)?;
        if let Expr::Operand(Operand::Field(path)) = &candidate.key
            && let [PathSegment::Field(name)] = path.segments()
        {
            candidate.index_field = name.clone();
        }
    }
    Ok(candidates)
}

pub(crate) fn apply(plan: &mut PhysicalPlan, fragments: &BTreeMap<LeafKey, LeafFragment>) {
    visit(plan, &mut |plan| {
        let PhysicalPlan::Join(join) = plan else {
            return;
        };
        if join.join_type != JoinType::Inner {
            return;
        }
        let PhysicalJoinCondition::Eq { right, .. } = &join.condition else {
            return;
        };
        let PhysicalPlan::Source(
            PhysicalSource::Scan { source } | PhysicalSource::FilteredScan { source, .. },
        ) = join.right.as_ref()
        else {
            return;
        };
        let Some(fragment) = fragments.get(&LeafKey::from(source)) else {
            return;
        };
        let Some(field) = &fragment.bind_field else {
            return;
        };
        if field != &FieldRef::Path(right.source_path.clone()) {
            return;
        }
        join.algorithm = PhysicalJoinAlgorithm::IndexNestedLoop;
        join.index_probe = Some(PhysicalIndexProbe {
            source: source.clone(),
            field: field.clone(),
            residual_predicate: fragment.pushed_predicate.clone(),
        });
    });
}

/// A separate negotiation/token lets a source decline bulk lookup while
/// retaining its accepted singleton lookup. The contract uses list membership
/// in both cases; only the concrete scan bindings differ in cardinality.
pub(crate) async fn negotiate_batches(
    fragments: &mut BTreeMap<LeafKey, LeafFragment>,
    sources: &FederationSources,
) -> Result<(), super::FederatedError> {
    use super::pushdown::{BatchFragment, combine_filters};
    use semantic_data::vdb::{FilterSupport, ScanPlan};
    let negotiations = fragments
        .values_mut()
        .filter(|fragment| fragment.bind_field.is_some())
        .map(|fragment| async move {
            let request = fragment.request.clone();
            let ScanPlan::Accepted { plan } = fragment
                .source
                .negotiate(&fragment.collection, &request)
                .await?
            else {
                return Ok::<_, super::FederatedError>(());
            };
            plan.validate(&request).map_err(|error| {
                crate::DbError::InvalidQuery(format!(
                    "{}: protocol violation: {}",
                    fragment.collection, error.message
                ))
            })?;
            if sources.virtual_sources[&fragment.collection].schema_revision != plan.schema_revision
            {
                return Err(super::FederatedError::SchemaChanged {
                    collection: fragment.collection.clone(),
                });
            }
            if plan.filters.last() != Some(&FilterSupport::Exact) {
                return Ok(());
            }
            let residual = combine_filters(
                request
                    .filters
                    .iter()
                    .cloned()
                    .zip(&plan.filters)
                    .filter_map(|(filter, support)| {
                        (*support != FilterSupport::Exact).then(|| Expr::from(filter))
                    })
                    .chain(fragment.host_residual.clone()),
            );
            fragment.batch = Some(BatchFragment {
                request,
                plan,
                residual,
            });
            Ok(())
        });
    futures::future::try_join_all(negotiations).await?;
    Ok(())
}

#[cfg(all(test, feature = "sql"))]
mod tests {
    use super::super::test_support::{MemorySource, NegotiationMode, accepted};
    use super::super::{FederatedEngine, QuerySource, SourceScan};
    use super::*;
    use crate::{DbError, DynObject, SendableRecordBatchStream};
    use async_trait::async_trait;
    use futures::{StreamExt, stream};
    use semantic_data::query::{BinaryOp, Query};
    use semantic_data::value::{FieldPath, Object, Value};
    use semantic_data::vdb::{FilterSupport, ScanPlan, ScanRequest};
    use std::sync::{Arc, Mutex};

    struct BoundSource {
        reject_plain: bool,
        reject_bound: bool,
        batch_accept: bool,
        batch_support: FilterSupport,
        batch_base_support: FilterSupport,
        batch_revision: String,
        batch_malformed: bool,
        estimate: Option<u64>,
        key_support: FilterSupport,
        base_support: FilterSupport,
        bound_revision: String,
        malformed: bool,
        rows: Vec<Object>,
        requests: Mutex<Vec<ScanRequest>>,
        scans: Mutex<Vec<SourceScan>>,
    }
    #[async_trait]
    impl QuerySource for BoundSource {
        async fn negotiate(&self, _: &str, request: &ScanRequest) -> Result<ScanPlan, DbError> {
            let bound = request.parameters == [KEYS_PARAMETER];
            let mut requests = self.requests.lock().unwrap();
            let batch = bound
                && requests
                    .last()
                    .is_some_and(|request| request.parameters == [KEYS_PARAMETER]);
            requests.push(request.clone());
            drop(requests);
            if batch && !self.batch_accept {
                return Ok(ScanPlan::Rejected {
                    reason: "batch unavailable".into(),
                });
            }
            if bound && self.reject_bound {
                return Ok(ScanPlan::Rejected {
                    reason: "dynamic lookup unavailable".into(),
                });
            }
            if !bound && self.reject_plain {
                return Ok(ScanPlan::Rejected {
                    reason: "key filter required".into(),
                });
            }
            let mut plan = accepted(request);
            plan.estimated_rows = self.estimate;
            plan.filters.fill(if batch {
                self.batch_base_support
            } else {
                self.base_support
            });
            if bound {
                *plan.filters.last_mut().unwrap() = if batch {
                    self.batch_support
                } else {
                    self.key_support
                };
                plan.schema_revision = if batch {
                    self.batch_revision.clone()
                } else {
                    self.bound_revision.clone()
                };
                plan.token = Some(Value::String(
                    if batch { "batch-token" } else { "bound-token" }.into(),
                ));
                if self.malformed || (batch && self.batch_malformed) {
                    plan.filters.clear();
                }
            }
            Ok(ScanPlan::Accepted { plan })
        }
        fn scan(self: Arc<Self>, scan: SourceScan) -> SendableRecordBatchStream {
            self.scans.lock().unwrap().push(scan.clone());
            let mut rows = self.rows.clone();
            for (filter, support) in scan.request.filters.iter().zip(&scan.plan.filters) {
                if *support == FilterSupport::Exact {
                    let query = crate::Query::Select(crate::SelectQuery {
                        predicate: Some(filter.clone().into()),
                        ..Default::default()
                    })
                    .into_bound(&scan.bindings)
                    .unwrap();
                    let crate::Query::Select(query) = query else {
                        unreachable!()
                    };
                    rows.retain(|row| {
                        crate::evaluate_filter_expr(row, query.predicate.as_ref().unwrap())
                    });
                }
            }
            stream::iter(
                rows.into_iter()
                    .map(|row| Ok(vec![Box::new(row) as DynObject])),
            )
            .boxed()
        }
    }
    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        futures::executor::block_on(future)
    }
    fn sql(text: &str) -> semantic_data::query::SelectQuery {
        let Query::Select(query) =
            crate::sql::parse_sql_query_unbound(text, crate::sql::SqlDialectKind::Generic).unwrap()
        else {
            unreachable!()
        };
        query
    }
    fn row(id: Value, title: &str) -> Object {
        Object::from_iter([
            ("shared:owner".into(), id.clone()),
            ("id".into(), id),
            ("type".into(), Value::String("virtual:Item".into())),
            ("virtual:title".into(), Value::String(title.into())),
        ])
    }
    fn setup(
        configure: impl FnOnce(&mut BoundSource),
        virtual_outer: bool,
    ) -> (FederatedEngine, Arc<BoundSource>) {
        let outer = vec![
            Object::from_iter([("id".into(), Value::String("a".into()))]),
            Object::from_iter([("id".into(), Value::String("a".into()))]),
            Object::from_iter([("id".into(), Value::String("b".into()))]),
            Object::from_iter([("id".into(), Value::Null)]),
        ];
        setup_outer(configure, virtual_outer, outer)
    }
    fn setup_outer(
        configure: impl FnOnce(&mut BoundSource),
        virtual_outer: bool,
        outer: Vec<Object>,
    ) -> (FederatedEngine, Arc<BoundSource>) {
        let (local, mut sources) = super::super::planner::tests::setup();
        sources.local = Arc::new(MemorySource {
            rows: BTreeMap::from([("local".into(), outer.clone())]),
            mode: NegotiationMode::Exact,
            revision: "local".into(),
        });
        let mut source = BoundSource {
            reject_plain: true,
            reject_bound: false,
            batch_accept: false,
            batch_support: FilterSupport::Exact,
            batch_base_support: FilterSupport::Exact,
            batch_revision: "1".into(),
            batch_malformed: false,
            estimate: None,
            key_support: FilterSupport::Exact,
            base_support: FilterSupport::Exact,
            bound_revision: "1".into(),
            malformed: false,
            rows: vec![
                row(Value::String("a".into()), "Alpha"),
                row(Value::String("a".into()), "Alternate"),
                row(Value::String("b".into()), "Beta"),
                row(Value::Null, "Null"),
            ],
            requests: Mutex::new(Vec::new()),
            scans: Mutex::new(Vec::new()),
        };
        configure(&mut source);
        let source = Arc::new(source);
        sources.virtual_sources.get_mut("fx").unwrap().source = source.clone();
        if virtual_outer {
            let mut other = sources.virtual_sources["fx"].clone();
            other.source = Arc::new(MemorySource {
                rows: BTreeMap::from([("fx2".into(), outer)]),
                mode: NegotiationMode::Exact,
                revision: "1".into(),
            });
            sources.virtual_sources.insert("fx2".into(), other);
        }
        (FederatedEngine::new(local, sources), source)
    }
    const JOIN: &str =
        "SELECT l.id AS left_id, r.title AS title FROM local AS l JOIN fx AS r ON l.id = r.id";

    #[test]
    fn rejected_plain_scan_uses_exact_singleton_lookups_and_explains_parameters() {
        let (engine, source) = setup(|_| {}, false);
        let explanation = run(engine.explain(sql(JOIN), &BTreeMap::new())).unwrap();
        assert!(format!("{:?}", explanation.physical).contains("IndexNestedLoop"));
        let leaf = explanation
            .leaves
            .iter()
            .find(|leaf| leaf.collection == "fx")
            .unwrap();
        assert!(format!("{:?}", leaf.filters).contains("Parameter(\"__keys\")"));
        let rows = run(engine.select(sql(JOIN), &BTreeMap::new())).unwrap();
        assert_eq!(rows.len(), 5); // Two outer a rows times two inner matches, plus b.
        assert_eq!(
            rows.iter()
                .filter(|row| row.get("title") == Some(&Value::String("Alpha".into())))
                .count(),
            2
        );
        let scans = source.scans.lock().unwrap();
        assert!(!scans.is_empty());
        for scan in scans.iter() {
            assert_eq!(scan.request.parameters, [KEYS_PARAMETER]);
            assert_eq!(scan.request.limit, None);
            assert_eq!(scan.request.offset, 0);
            assert_eq!(scan.plan.token, Some(Value::String("bound-token".into())));
            assert!(
                matches!(scan.bindings.get(KEYS_PARAMETER), Some(Value::List(values)) if values.len() == 1)
            );
        }
    }
    #[test]
    fn aliased_virtual_attribute_lookup_uses_its_canonical_scalar_key() {
        let (engine, source) = setup(|_| {}, false);
        let query = "SELECT r.title AS title FROM local AS l JOIN fx AS r ON l.id = r.owner";
        assert_eq!(
            run(engine.select(sql(query), &BTreeMap::new()))
                .unwrap()
                .len(),
            5
        );
        let scans = source.scans.lock().unwrap();
        for scan in scans.iter() {
            let semantic_data::query::Expr::Binary {
                op: BinaryOp::In,
                left,
                ..
            } = scan.request.filters.last().unwrap()
            else {
                panic!("missing membership");
            };
            assert_eq!(
                left.as_ref(),
                &semantic_data::query::Expr::Operand(semantic_data::query::Operand::Field(
                    FieldPath::from_fields(["shared:owner"])
                ))
            );
        }
    }
    #[test]
    fn virtual_outer_and_non_exact_base_filter_keep_join_residuals() {
        let (engine, _) = setup(|source| source.base_support = FilterSupport::Inexact, true);
        let query = "SELECT l.id AS left_id, r.title AS title FROM fx2 AS l JOIN fx AS r ON l.id = r.id AND r.title <> 'Alternate' WHERE r.title = 'Alpha'";
        let rows = run(engine.select(sql(query), &BTreeMap::new())).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.get("title") == Some(&Value::String("Alpha".into())))
        );
    }
    #[test]
    fn estimate_threshold_and_unknown_estimate_preserve_hash_fallback() {
        for (estimate, bind) in [(None, false), (Some(10_000), false), (Some(10_001), true)] {
            let (engine, source) = setup(
                |source| {
                    source.reject_plain = false;
                    source.estimate = estimate;
                },
                false,
            );
            let explanation = run(engine.explain(sql(JOIN), &BTreeMap::new())).unwrap();
            assert_eq!(
                format!("{:?}", explanation.physical).contains("IndexNestedLoop"),
                bind
            );
            assert_eq!(
                run(engine.select(sql(JOIN), &BTreeMap::new()))
                    .unwrap()
                    .len(),
                5
            );
            assert!(
                source
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|request| request.parameters == [KEYS_PARAMETER])
            );
            assert!(
                source
                    .scans
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|scan| scan.request.parameters.is_empty() != bind)
            );
        }
    }
    #[test]
    fn non_exact_membership_falls_back_or_preserves_plain_rejection() {
        for support in [FilterSupport::Inexact, FilterSupport::Unsupported] {
            let (engine, _) = setup(
                |source| {
                    source.reject_plain = false;
                    source.estimate = Some(20_000);
                    source.key_support = support;
                },
                false,
            );
            assert_eq!(
                run(engine.select(sql(JOIN), &BTreeMap::new()))
                    .unwrap()
                    .len(),
                5
            );
            assert!(
                !format!(
                    "{:?}",
                    run(engine.explain(sql(JOIN), &BTreeMap::new()))
                        .unwrap()
                        .physical
                )
                .contains("IndexNestedLoop")
            );
            let (engine, _) = setup(|source| source.key_support = support, false);
            let error = run(engine.select(sql(JOIN), &BTreeMap::new())).unwrap_err();
            assert!(error.to_string().contains("key filter required"));
        }
    }
    #[test]
    fn rejected_bound_candidate_keeps_accepted_plain_scan() {
        let (engine, _) = setup(
            |source| {
                source.reject_plain = false;
                source.reject_bound = true;
                source.estimate = Some(20_000);
            },
            false,
        );
        assert_eq!(
            run(engine.select(sql(JOIN), &BTreeMap::new()))
                .unwrap()
                .len(),
            5
        );
        assert!(
            !format!(
                "{:?}",
                run(engine.explain(sql(JOIN), &BTreeMap::new()))
                    .unwrap()
                    .physical
            )
            .contains("IndexNestedLoop")
        );
        let (engine, _) = setup(|source| source.reject_bound = true, false);
        assert!(
            run(engine.select(sql(JOIN), &BTreeMap::new()))
                .unwrap_err()
                .to_string()
                .contains("key filter required")
        );
    }
    #[test]
    fn bound_protocol_and_revision_errors_are_not_hidden_by_plain_fallback() {
        let (engine, _) = setup(
            |source| {
                source.reject_plain = false;
                source.bound_revision = "2".into();
            },
            false,
        );
        assert!(
            matches!(run(engine.select(sql(JOIN), &BTreeMap::new())), Err(super::super::FederatedError::SchemaChanged { collection }) if collection == "fx")
        );
        let (engine, _) = setup(
            |source| {
                source.reject_plain = false;
                source.malformed = true;
            },
            false,
        );
        assert!(
            run(engine.select(sql(JOIN), &BTreeMap::new()))
                .unwrap_err()
                .to_string()
                .contains("protocol violation")
        );
    }
    #[test]
    fn only_inner_equality_joins_offer_dynamic_lookup() {
        for query in [
            "SELECT l.id FROM local AS l LEFT JOIN fx AS r ON l.id = r.id",
            "SELECT l.id FROM local AS l JOIN fx AS r ON l.id <> r.id",
        ] {
            let (engine, source) = setup(|source| source.reject_plain = false, false);
            run(engine.explain(sql(query), &BTreeMap::new())).unwrap();
            assert!(
                source
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|request| request.parameters.is_empty())
            );
        }
    }
    #[test]
    fn host_parameterized_residual_is_bound_before_evaluation() {
        let predicate = Expr::Binary {
            op: BinaryOp::In,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "id",
            ])))),
            right: Box::new(Expr::Operand(Operand::Parameter(KEYS_PARAMETER.into()))),
        };
        let bound = super::super::composite::bind_residual(
            &predicate,
            &BTreeMap::from([(
                KEYS_PARAMETER.into(),
                Value::List(vec![Value::String("a".into())]),
            )]),
        )
        .unwrap();
        assert!(crate::evaluate_filter_expr(
            &row(Value::String("a".into()), "Alpha"),
            &bound
        ));
        assert!(!crate::evaluate_filter_expr(
            &row(Value::String("b".into()), "Beta"),
            &bound
        ));
        assert!(super::super::composite::bind_residual(&predicate, &BTreeMap::new()).is_err());
    }
    fn batch_outer() -> Vec<Object> {
        (0..130)
            .map(|i| {
                Object::from_iter([(
                    "id".into(),
                    match i % 3 {
                        0 => Value::String("a".into()),
                        1 => Value::String("b".into()),
                        _ => Value::Null,
                    },
                )])
            })
            .collect()
    }
    #[test]
    fn batches_use_own_token_list_bindings_residual_and_explain_size() {
        use semantic_data::value::IntoValue;
        let (engine, source) = setup_outer(
            |source| {
                source.batch_accept = true;
                source.batch_base_support = FilterSupport::Inexact;
            },
            false,
            batch_outer(),
        );
        let query = format!("{JOIN} WHERE r.title = 'Alpha'");
        let explanation = run(engine.explain(sql(&query), &BTreeMap::new())).unwrap();
        assert_eq!(
            explanation
                .leaves
                .iter()
                .find(|leaf| leaf.collection == "fx")
                .unwrap()
                .batch_size,
            Some(64)
        );
        assert!(
            explanation
                .leaves
                .iter()
                .filter(|leaf| leaf.collection != "fx")
                .all(|leaf| leaf.batch_size.is_none())
        );
        let Value::Object(wire) = explanation.into_value() else {
            panic!("explain object");
        };
        let Some(Value::List(leaves)) = wire.get("leaves") else {
            panic!("explain leaves");
        };
        assert!(leaves.iter().any(|leaf| matches!(leaf, Value::Object(leaf) if leaf.get("batch_size") == Some(&Value::U64(64)))));
        assert!(source.scans.lock().unwrap().is_empty());
        assert_eq!(
            run(engine.select(sql(&query), &BTreeMap::new()))
                .unwrap()
                .len(),
            44
        );
        let scans = source.scans.lock().unwrap();
        assert_eq!(scans.len(), 3);
        for (index, scan) in scans.iter().enumerate() {
            assert_eq!(scan.plan.token, Some(Value::String("batch-token".into())));
            assert_eq!(scan.request.limit, None);
            assert_eq!(scan.request.offset, 0);
            assert!(
                matches!(scan.bindings.get(KEYS_PARAMETER), Some(Value::List(keys)) if keys.len() == if index == 2 { 1 } else { 2 } && !keys.contains(&Value::Null))
            );
            assert_eq!(scan.plan.filters.last(), Some(&FilterSupport::Exact));
        }
    }
    #[test]
    fn rejected_or_nonexact_batch_keeps_valid_singleton_lookup() {
        for support in [
            None,
            Some(FilterSupport::Inexact),
            Some(FilterSupport::Unsupported),
        ] {
            let (engine, source) = setup_outer(
                |source| {
                    source.batch_accept = support.is_some();
                    source.batch_support = support.unwrap_or(FilterSupport::Exact);
                },
                false,
                batch_outer(),
            );
            let explanation = run(engine.explain(sql(JOIN), &BTreeMap::new())).unwrap();
            assert!(
                explanation
                    .leaves
                    .iter()
                    .all(|leaf| leaf.batch_size.is_none())
            );
            assert_eq!(
                run(engine.select(sql(JOIN), &BTreeMap::new()))
                    .unwrap()
                    .len(),
                131
            );
            let scans = source.scans.lock().unwrap();
            assert_eq!(scans.len(), 2);
            assert!(
                scans
                    .iter()
                    .all(|scan| scan.plan.token == Some(Value::String("bound-token".into())))
            );
        }
    }
    #[test]
    fn malformed_or_changed_batch_is_fatal_after_valid_singleton() {
        let (engine, _) = setup(
            |source| {
                source.batch_accept = true;
                source.batch_revision = "2".into();
            },
            false,
        );
        assert!(
            matches!(run(engine.select(sql(JOIN), &BTreeMap::new())), Err(super::super::FederatedError::SchemaChanged { collection }) if collection == "fx")
        );
        let (engine, _) = setup(
            |source| {
                source.batch_accept = true;
                source.batch_malformed = true;
            },
            false,
        );
        assert!(
            run(engine.select(sql(JOIN), &BTreeMap::new()))
                .unwrap_err()
                .to_string()
                .contains("protocol violation")
        );
    }
}
