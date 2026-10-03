use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use semantic_data::query::BinaryOp;
use semantic_data::value::PathSegment;
use semantic_data::vdb::{AcceptedScan, FilterSupport, ScanPlan, ScanRequest};

use super::planner::{LeafKey, PlannedSelect, collect_leaves};
use super::{FederatedError, FederationSources, QuerySource};
use crate::{DbError, Expr, LogicalPlan, OrderBy};

#[derive(Clone)]
pub(crate) struct LeafFragment {
    pub collection: String,
    pub source: Arc<dyn QuerySource>,
    pub request: ScanRequest,
    pub plan: AcceptedScan,
    pub residual: Option<Expr>,
    pub pushed_predicate: Option<Expr>,
    pub sole_input: bool,
}

pub(crate) fn combine_filters(filters: impl IntoIterator<Item = Expr>) -> Option<Expr> {
    filters.into_iter().reduce(|left, right| Expr::Binary {
        op: BinaryOp::And,
        left: Box::new(left),
        right: Box::new(right),
    })
}

/// Rewrite just this scan's binding. Subqueries and relation predicates need
/// the host context; another leaf's binding must never be sent to the plugin.
fn entity_relative(expr: &Expr, key: &LeafKey, bindings: &BTreeSet<String>) -> Option<Expr> {
    let mut expr: semantic_data::query::Expr = expr.clone().into();
    let mut supported = true;
    let result = expr.visit_mut(&mut |expr| {
        match expr {
            semantic_data::query::Expr::Operand(semantic_data::query::Operand::Field(path)) => {
                if path.segments().len() > 1
                    && let Some(PathSegment::Field(binding)) = path.segments().first()
                {
                    if Some(binding.as_str()) == key.binding.as_deref()
                        || binding == &key.source_name
                    {
                        path.0.remove(0);
                    } else if bindings.contains(binding) {
                        supported = false;
                    }
                }
            }
            semantic_data::query::Expr::Subquery(_)
            | semantic_data::query::Expr::Exists { .. }
            | semantic_data::query::Expr::RelationExists { .. } => supported = false,
            _ => {}
        }
        Ok::<(), std::convert::Infallible>(())
    });
    match result {
        Ok(()) => {}
        Err(impossible) => match impossible {},
    }
    supported.then(|| expr.into())
}

fn has_host_filter(plan: &LogicalPlan) -> bool {
    match plan {
        LogicalPlan::Filter { .. } => true,
        LogicalPlan::Sort { input, .. }
        | LogicalPlan::Project { input, .. }
        | LogicalPlan::Limit { input, .. } => has_host_filter(input),
        _ => false,
    }
}

pub(crate) async fn negotiate_leaves(
    planned: &mut PlannedSelect,
    query_order: &[OrderBy],
    limit: Option<u64>,
    offset: u64,
    sources: &FederationSources,
) -> Result<BTreeMap<LeafKey, LeafFragment>, FederatedError> {
    let leaves = collect_leaves(&planned.logical);
    let bindings = leaves
        .iter()
        .flat_map(|leaf| {
            std::iter::once(leaf.key.source_name.clone()).chain(leaf.key.binding.clone())
        })
        .collect::<BTreeSet<_>>();
    let host_filter = has_host_filter(&planned.logical);
    let negotiations = leaves.into_iter().map(|leaf| {
        let bindings = &bindings;
        let overlay = &planned.overlay;
        async move {
            let collection = &leaf.key.source_name;
            let schema = overlay.collection_by_name(collection).ok_or_else(|| {
                DbError::UnknownCollectionByName {
                    name: collection.clone(),
                }
            })?;
            let mut filters = Vec::new();
            let mut host_only = Vec::new();
            for filter in leaf
                .pushed_predicate
                .clone()
                .into_iter()
                .flat_map(crate::plan::conjuncts)
            {
                if let Some(filter) = entity_relative(&filter, &leaf.key, bindings) {
                    filters.push(
                        crate::canonical::canonicalize_filter_expr(&filter, overlay, schema)
                            .map_err(DbError::from)?,
                    );
                } else {
                    host_only.push(filter);
                }
            }
            let mut order = Vec::new();
            let mut host_order = false;
            if leaf.sole_input {
                for term in query_order {
                    if let Some(expr) = entity_relative(&term.expr, &leaf.key, bindings) {
                        order.push(OrderBy {
                            expr: crate::canonical::canonicalize_filter_expr(
                                &expr, overlay, schema,
                            )
                            .map_err(DbError::from)?,
                            direction: term.direction,
                        });
                    } else {
                        host_order = true;
                    }
                }
            }
            let can_limit = leaf.sole_input && host_only.is_empty() && !host_filter && !host_order;
            if host_order {
                order.clear();
            }
            let request = ScanRequest {
                filters: filters.iter().cloned().map(Into::into).collect(),
                order_by: order.into_iter().map(Into::into).collect(),
                limit: can_limit.then_some(limit).flatten(),
                offset: if can_limit { offset } else { 0 },
                projection: None,
                parameters: Vec::new(),
                fetch_hint: if can_limit {
                    None
                } else {
                    limit.and_then(|limit| offset.checked_add(limit))
                },
            };
            let virtual_source = sources.virtual_sources.get(collection);
            let source = virtual_source
                .map_or_else(|| sources.local.clone(), |source| source.source.clone());
            let plan = match source.negotiate(collection, &request).await? {
                ScanPlan::Accepted { plan } => plan,
                ScanPlan::Rejected { reason } => {
                    return Err(DbError::InvalidQuery(format!("{collection}: {reason}")).into());
                }
            };
            plan.validate(&request).map_err(|error| {
                DbError::InvalidQuery(format!(
                    "{collection}: protocol violation: {}",
                    error.message
                ))
            })?;
            if virtual_source.is_some_and(|source| source.schema_revision != plan.schema_revision) {
                return Err(FederatedError::SchemaChanged {
                    collection: collection.clone(),
                });
            }
            let residual = combine_filters(
                filters
                    .into_iter()
                    .zip(&plan.filters)
                    .filter_map(|(filter, support)| {
                        (*support != FilterSupport::Exact).then_some(filter)
                    })
                    .chain(host_only),
            );
            let fragment = LeafFragment {
                collection: collection.clone(),
                source,
                request,
                plan,
                residual,
                pushed_predicate: leaf.pushed_predicate,
                sole_input: leaf.sole_input,
            };
            Ok((leaf.key, fragment))
        }
    });
    let mut fragments = BTreeMap::new();
    for (key, fragment) in futures::future::try_join_all(negotiations).await? {
        if fragments.insert(key.clone(), fragment).is_some() {
            return Err(DbError::InvalidQuery(format!(
                "federation: duplicate leaf binding for '{}'",
                key.source_name
            ))
            .into());
        }
    }
    Ok(fragments)
}

pub(crate) fn apply_offset_rewrites(
    mut plan: LogicalPlan,
    fragments: &BTreeMap<LeafKey, LeafFragment>,
) -> LogicalPlan {
    fn visit(plan: &mut LogicalPlan, fragments: &BTreeMap<LeafKey, LeafFragment>) {
        match plan {
            LogicalPlan::Limit { input, offset, .. } => {
                let leaves = collect_leaves(input);
                if leaves.len() == 1
                    && fragments
                        .get(&leaves[0].key)
                        .is_some_and(|fragment| fragment.sole_input && fragment.plan.offset_applied)
                {
                    *offset = Expr::from(0usize);
                }
                visit(input, fragments);
            }
            LogicalPlan::Filter { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Project { input, .. }
            | LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Distinct { input }
            | LogicalPlan::Exchange { input, .. }
            | LogicalPlan::RepartitionHash { input, .. } => visit(input, fragments),
            LogicalPlan::Union { inputs, .. } => {
                for input in inputs {
                    visit(input, fragments);
                }
            }
            LogicalPlan::Join(join) => {
                visit(&mut join.left, fragments);
                visit(&mut join.right, fragments);
            }
            LogicalPlan::ApplyExists {
                input, subquery, ..
            }
            | LogicalPlan::ApplyInSubquery {
                input, subquery, ..
            } => {
                visit(input, fragments);
                visit(subquery, fragments);
            }
            LogicalPlan::Source { .. } | LogicalPlan::Values { .. } => {}
        }
    }
    visit(&mut plan, fragments);
    plan
}

#[cfg(test)]
mod tests {
    use super::super::planner::{overlay_catalog, plan_select};
    use super::super::test_support::{TestSource, accepted};
    use super::*;
    use crate::Operand;
    use semantic_data::query::SortDirection;
    use semantic_data::value::{FieldPath, Value};

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        futures::executor::block_on(future)
    }
    fn field(name: &str) -> Expr {
        Expr::Operand(Operand::Field(FieldPath::from_fields([name])))
    }
    fn eq(name: &str, value: &str) -> Expr {
        Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(name)),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(value.into())))),
        }
    }
    fn setup(
        source: Arc<TestSource>,
        query: crate::SelectQuery,
    ) -> (PlannedSelect, FederationSources) {
        let (local, mut sources) = super::super::planner::tests::setup();
        sources.virtual_sources.get_mut("fx").unwrap().source = source;
        let overlay =
            Arc::new(overlay_catalog(&local, &[("fx", &sources.virtual_sources["fx"])]).unwrap());
        (
            plan_select(query, overlay, &sources, &local).unwrap(),
            sources,
        )
    }

    #[test]
    fn exact_scan_receives_order_limit_and_applies_offset_once() {
        let source = Arc::new(TestSource::new(|request| {
            Ok(ScanPlan::Accepted {
                plan: accepted(request),
            })
        }));
        let order = vec![OrderBy {
            expr: field("id"),
            direction: SortDirection::Asc,
        }];
        let (mut planned, sources) = setup(
            source.clone(),
            crate::SelectQuery::new()
                .with_collection("fx")
                .with_predicate(eq("title", "x"))
                .with_order_by(order.clone())
                .with_limit(3usize)
                .with_offset(2usize),
        );
        let fragments = run(negotiate_leaves(&mut planned, &order, Some(3), 2, &sources)).unwrap();
        let fragment = fragments.values().next().unwrap();
        assert!(fragment.residual.is_none());
        assert_eq!(fragment.request.limit, Some(3));
        assert_eq!(fragment.request.offset, 2);
        assert_eq!(fragment.request.order_by.len(), 1);
        assert!(format!("{:?}", fragment.request.filters).contains("virtual:title"));
        let rewritten = apply_offset_rewrites(planned.logical, &fragments);
        let LogicalPlan::Limit { offset, .. } = rewritten else {
            panic!("limit")
        };
        assert_eq!(crate::evaluate_usize_expr(&offset), Some(0));
        assert_eq!(source.requests().len(), 1);
    }

    #[test]
    fn mixed_filter_support_keeps_only_non_exact_residuals_in_order() {
        let source = Arc::new(TestSource::new(|request| {
            let mut plan = accepted(request);
            plan.filters = vec![
                FilterSupport::Exact,
                FilterSupport::Inexact,
                FilterSupport::Unsupported,
            ];
            plan.limit_applied = false;
            plan.offset_applied = false;
            Ok(ScanPlan::Accepted { plan })
        }));
        let predicate =
            combine_filters([eq("id", "a"), eq("type", "virtual:Item"), eq("title", "x")]).unwrap();
        let (mut planned, sources) = setup(
            source,
            crate::SelectQuery::new()
                .with_collection("fx")
                .with_predicate(predicate),
        );
        let fragments = run(negotiate_leaves(&mut planned, &[], None, 0, &sources)).unwrap();
        let fragment = fragments.values().next().unwrap();
        let residual = crate::plan::conjuncts(fragment.residual.clone().unwrap());
        let expected = fragment.request.filters[1..]
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<Expr>>();
        assert_eq!(residual, expected);
    }

    #[cfg(feature = "sql")]
    #[test]
    fn join_scans_receive_canonical_relative_filters_without_order_or_limit() {
        let source = Arc::new(TestSource::new(|request| {
            Ok(ScanPlan::Accepted {
                plan: accepted(request),
            })
        }));
        let query = crate::sql::parse_sql_query_unbound("SELECT a.id FROM local a JOIN fx b ON a.id = b.id WHERE b.title = 'x' ORDER BY a.id LIMIT 2", crate::sql::SqlDialectKind::Generic).unwrap();
        let semantic_data::query::Query::Select(query) = query else {
            panic!("select")
        };
        let (mut planned, sources) = setup(source.clone(), query.into());
        // The local test placeholder is not executed or negotiated in this test.
        let mut sources = sources;
        sources.local = source.clone();
        let fragments = run(negotiate_leaves(&mut planned, &[], Some(2), 0, &sources)).unwrap();
        assert_eq!(fragments.len(), 2);
        for fragment in fragments.values() {
            assert!(fragment.request.order_by.is_empty());
            assert!(fragment.request.limit.is_none());
        }
        let virtual_fragment = fragments
            .values()
            .find(|fragment| fragment.collection == "fx")
            .unwrap();
        assert_eq!(
            virtual_fragment.request.filters,
            vec![eq("virtual:title", "x").into()]
        );
    }

    #[test]
    fn rejection_and_schema_changes_propagate() {
        let rejected = Arc::new(TestSource::new(|_| {
            Ok(ScanPlan::Rejected {
                reason: "requires a type filter".into(),
            })
        }));
        let (mut planned, sources) =
            setup(rejected, crate::SelectQuery::new().with_collection("fx"));
        let error = run(negotiate_leaves(&mut planned, &[], None, 0, &sources))
            .err()
            .unwrap();
        assert!(error.to_string().contains("requires a type filter"));
        let changed = Arc::new(TestSource::new(|request| {
            let mut plan = accepted(request);
            plan.schema_revision = "2".into();
            Ok(ScanPlan::Accepted { plan })
        }));
        let (mut planned, sources) =
            setup(changed, crate::SelectQuery::new().with_collection("fx"));
        assert!(
            matches!(run(negotiate_leaves(&mut planned, &[], None, 0, &sources)), Err(FederatedError::SchemaChanged { collection }) if collection == "fx")
        );
    }

    #[test]
    fn protocol_violations_fail_the_query() {
        for violation in 0..4 {
            let source = Arc::new(TestSource::new(move |request| {
                let mut plan = accepted(request);
                match violation {
                    0 => plan.filters.clear(),
                    1 => plan.ordered_prefix = 1,
                    2 => {
                        plan.limit_applied = true;
                    }
                    3 => {
                        plan.filters[0] = FilterSupport::Inexact;
                        plan.offset_applied = true;
                    }
                    _ => unreachable!(),
                }
                Ok(ScanPlan::Accepted { plan })
            }));
            let (mut planned, sources) = setup(
                source,
                crate::SelectQuery::new()
                    .with_collection("fx")
                    .with_predicate(eq("id", "a")),
            );
            let error = run(negotiate_leaves(&mut planned, &[], None, 0, &sources))
                .err()
                .unwrap();
            assert!(
                error.to_string().contains("protocol violation"),
                "{violation}: {error}"
            );
        }
    }

    #[cfg(feature = "sql")]
    #[test]
    fn bound_limit_is_offered_as_literal() {
        let source = Arc::new(TestSource::new(|request| {
            Ok(ScanPlan::Accepted {
                plan: accepted(request),
            })
        }));
        let query = crate::sql::parse_sql_query_unbound(
            "SELECT * FROM fx LIMIT :n",
            crate::sql::SqlDialectKind::Generic,
        )
        .unwrap()
        .into_bound(&BTreeMap::from([("n".into(), Value::U64(4))]))
        .unwrap();
        let semantic_data::query::Query::Select(query) = query else {
            panic!("select")
        };
        let query: crate::SelectQuery = query.into();
        let limit = query
            .limit
            .as_ref()
            .and_then(crate::evaluate_usize_expr)
            .map(|limit| limit as u64);
        let (mut planned, sources) = setup(source.clone(), query);
        run(negotiate_leaves(&mut planned, &[], limit, 0, &sources)).unwrap();
        assert_eq!(source.requests()[0].1.limit, Some(4));
    }
}
