use std::collections::BTreeMap;
use std::sync::Arc;

use semantic_data::query::{FieldFormat, Query};
use semantic_data::value::{Object, Value};

use super::composite::CompositeDataSource;
use super::planner::{LeafKey, overlay_catalog, plan_select};
use super::pushdown::{LeafFragment, apply_offset_rewrites, negotiate_leaves_with_bind};
use super::{FederatedError, FederatedExplain, FederationSources, LeafExplain};
use crate::catalog::Catalog;
use crate::{DbError, ExecutionOptions, Expr, LogicalPlan, Optimizer, PhysicalPlan, QueryContext};

pub struct FederatedEngine {
    local_catalog: Arc<Catalog>,
    sources: FederationSources,
}

struct PreparedExecution {
    overlay: Arc<Catalog>,
    logical: LogicalPlan,
    physical: PhysicalPlan,
    fragments: BTreeMap<LeafKey, LeafFragment>,
    field_format: FieldFormat,
}

impl FederatedEngine {
    pub fn new(local_catalog: Arc<Catalog>, sources: FederationSources) -> Self {
        Self {
            local_catalog,
            sources,
        }
    }

    /// Execute a SELECT over an in-memory overlay and the negotiated sources.
    pub async fn select(
        &self,
        query: semantic_data::query::SelectQuery,
        params: &BTreeMap<String, Value>,
    ) -> Result<Vec<Object>, FederatedError> {
        let prepared = self.prepare(query, params).await?;
        let mut rows = crate::execute_physical_plan_collect(
            prepared.physical,
            Arc::new(CompositeDataSource {
                fragments: prepared.fragments,
            }),
            QueryContext::new(prepared.overlay.clone()),
            ExecutionOptions::default(),
        )
        .await
        .map_err(|error| DbError::InvalidQuery(error.to_string()))?;
        for row in &mut rows {
            let _ = crate::inject_computed_attributes(&prepared.overlay, row);
        }
        Ok(crate::format_output_rows(
            &prepared.overlay,
            rows,
            prepared.field_format,
        ))
    }

    pub async fn explain(
        &self,
        query: semantic_data::query::SelectQuery,
        params: &BTreeMap<String, Value>,
    ) -> Result<FederatedExplain, FederatedError> {
        let prepared = self.prepare(query, params).await?;
        let leaves = prepared
            .fragments
            .into_iter()
            .map(|(key, fragment)| {
                let batch_size = fragment
                    .bind_field
                    .as_ref()
                    .map(|_| super::bind::BIND_JOIN_BATCH_SIZE as u64);
                let (request, plan, residual) =
                    (fragment.request, fragment.plan, fragment.residual);
                LeafExplain {
                    collection: fragment.collection,
                    source_tag: key.backend_tag,
                    filters: request.filters.into_iter().zip(plan.filters).collect(),
                    ordered_prefix: plan.ordered_prefix,
                    limit_applied: plan.limit_applied,
                    offset_applied: plan.offset_applied,
                    estimated_rows: plan.estimated_rows,
                    residual: residual.map(Into::into),
                    batch_size,
                }
            })
            .collect();
        Ok(FederatedExplain {
            logical: prepared.logical,
            physical: prepared.physical,
            leaves,
        })
    }

    async fn prepare(
        &self,
        query: semantic_data::query::SelectQuery,
        params: &BTreeMap<String, Value>,
    ) -> Result<PreparedExecution, FederatedError> {
        // Use the existing core binder, which also resolves SQL projection
        // references in GROUP BY and ORDER BY after parameter substitution.
        let bound = crate::Query::Select(query.into()).into_bound(params)?;
        let mut query: Query = bound.into();
        super::normalize_virtual_joins(
            &mut query,
            &self.local_catalog,
            &self.sources.virtual_sources.keys().cloned().collect(),
        );
        let referenced = super::referenced_collections(&query);
        let virtual_sources = self
            .sources
            .virtual_sources
            .iter()
            .filter(|(name, _)| referenced.contains(*name))
            .map(|(name, source)| (name.as_str(), source))
            .collect::<Vec<_>>();
        let overlay = Arc::new(overlay_catalog(&self.local_catalog, &virtual_sources)?);
        let Query::Select(query) = query else {
            unreachable!()
        };
        let query: crate::SelectQuery = query.into();
        let order = query.order_by.clone();
        let offset = literal_limit(&query.offset);
        // Truncation is unsafe when the host still has to evaluate its offset.
        let limit = offset.and_then(|_| query.limit.as_ref().and_then(literal_limit));
        let offset = offset.unwrap_or(0);
        let mut planned = plan_select(query, overlay.clone(), &self.sources)?;
        let baseline = Optimizer::core().lower_to_physical(
            &planned.logical,
            None,
            &QueryContext::new(overlay.clone()),
        );
        let candidates = super::bind::candidates(&baseline, &self.sources, &overlay)?;
        let fragments = negotiate_leaves_with_bind(
            &mut planned,
            &order,
            limit,
            offset,
            &self.sources,
            &candidates,
        )
        .await?;
        // Metadata only for the selected exact lookup, after ordinary index
        // removal. Core lowering stays index-free and is rewritten below.
        let mut lookup_overlay = (*overlay).clone();
        for (key, fragment) in &fragments {
            if fragment.bind_field.is_some() {
                let candidate = &candidates[key];
                let collection = lookup_overlay
                    .collection_by_name(&key.source_name)
                    .unwrap()
                    .lid;
                lookup_overlay
                    .upsert_index(
                        format!("__vdb_bind_{}_{}", key.source_name, candidate.index_field),
                        collection,
                        &candidate.index_field,
                        false,
                    )
                    .map_err(|error| DbError::InvalidQuery(error.to_string()))?;
            }
        }
        let lookup_overlay = Arc::new(lookup_overlay);
        let logical = apply_offset_rewrites(planned.logical, &fragments);
        let mut physical = Optimizer::core().lower_to_physical(
            &logical,
            None,
            &QueryContext::new(overlay.clone()),
        );
        super::bind::apply(&mut physical, &fragments);
        Ok(PreparedExecution {
            overlay: lookup_overlay,
            logical,
            physical,
            fragments,
            field_format: planned.field_format,
        })
    }
}

fn literal_limit(expr: &Expr) -> Option<u64> {
    matches!(expr, Expr::Operand(crate::Operand::Literal(_)))
        .then(|| crate::evaluate_usize_expr(expr).map(|value| value as u64))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{MemorySource, NegotiationMode, accepted};
    use super::super::{QuerySource, SourceScan};
    use super::*;
    use crate::catalog::{CollectionKind, IntegrityMode};
    use crate::{DynObject, SendableRecordBatchStream};
    use async_trait::async_trait;
    use futures::{StreamExt, stream};
    use semantic_data::value::{IntoValue, SemanticType};
    use semantic_data::vdb::{FilterSupport, ScanPlan, ScanRequest};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        futures::executor::block_on(future)
    }
    fn row(id: &str, title: &str, owner: &str) -> Object {
        Object::from_iter([
            ("id".into(), Value::String(id.into())),
            ("type".into(), Value::String("virtual:Item".into())),
            ("virtual:title".into(), Value::String(title.into())),
            ("shared:owner".into(), Value::String(owner.into())),
        ])
    }

    fn engines(mode: NegotiationMode) -> (FederatedEngine, FederatedEngine) {
        let (local, mut sources) = super::super::planner::tests::setup();
        let rows = vec![
            row("a", "Alpha", "one"),
            row("b", "Beta", "one"),
            row("c", "Gamma", "two"),
            row("d", "Delta", "two"),
        ];
        let local_rows = vec![
            Object::from_iter([("id".into(), Value::String("a".into()))]),
            Object::from_iter([("id".into(), Value::String("c".into()))]),
        ];
        sources.local = Arc::new(MemorySource {
            rows: BTreeMap::from([("local".into(), local_rows.clone())]),
            mode: NegotiationMode::Exact,
            revision: "local".into(),
        });
        let source = sources.virtual_sources.get_mut("fx").unwrap();
        source.source = Arc::new(MemorySource {
            rows: BTreeMap::from([("fx".into(), rows.clone())]),
            mode,
            revision: "1".into(),
        });
        let mut second = source.clone();
        second.source = Arc::new(MemorySource {
            rows: BTreeMap::from([("fx2".into(), rows.clone())]),
            mode,
            revision: "1".into(),
        });
        sources.virtual_sources.insert("fx2".into(), second);
        let mut oracle_catalog =
            crate::apply_ddl_batch(&local, &sources.virtual_sources["fx"].schema)
                .unwrap()
                .0;
        for collection in ["mirror", "mirror2"] {
            oracle_catalog
                .upsert_collection(
                    collection,
                    CollectionKind::Polymorphic,
                    IntegrityMode::Permissive,
                )
                .unwrap();
        }
        let oracle = FederatedEngine::new(
            Arc::new(oracle_catalog),
            FederationSources {
                local: Arc::new(MemorySource {
                    rows: BTreeMap::from([
                        ("local".into(), local_rows),
                        ("mirror".into(), rows.clone()),
                        ("mirror2".into(), rows),
                    ]),
                    mode: NegotiationMode::Exact,
                    revision: "local".into(),
                }),
                virtual_sources: BTreeMap::new(),
            },
        );
        (FederatedEngine::new(local, sources), oracle)
    }

    #[cfg(feature = "sql")]
    fn select(sql: &str) -> semantic_data::query::SelectQuery {
        let Query::Select(query) =
            crate::sql::parse_sql_query_unbound(sql, crate::sql::SqlDialectKind::Generic).unwrap()
        else {
            panic!("select")
        };
        query
    }

    #[cfg(feature = "sql")]
    #[test]
    fn differential_corpus_across_negotiation_modes() {
        let corpus = [
            "SELECT * FROM {v}",
            "SELECT id FROM {v} WHERE id = 'a'",
            "SELECT id FROM {v} WHERE type = 'virtual:Item' AND title LIKE 'A%'",
            "SELECT id FROM {v} WHERE id IN ('a', 'c')",
            "SELECT id FROM {v} WHERE id BETWEEN 'b' AND 'd'",
            "SELECT id FROM {v} WHERE title IS NOT NULL",
            "SELECT * FROM {v} ORDER BY title DESC",
            "SELECT * FROM {v} ORDER BY owner, id DESC LIMIT 2 OFFSET 1",
            "SELECT DISTINCT owner FROM {v}",
            "SELECT owner, COUNT(*) AS count FROM {v} GROUP BY owner ORDER BY owner",
            "SELECT a.id FROM local a JOIN {v}._ b ON a.id = b.id ORDER BY a.id",
            "SELECT a.id FROM {v} a JOIN {w}._ b ON a.id = b.id WHERE b.title = 'Alpha'",
            "SELECT id FROM local WHERE id IN (SELECT v.id FROM {v} v WHERE v.title = 'Alpha')",
            "SELECT id FROM local WHERE EXISTS (SELECT v.id FROM {v} v WHERE v.id = 'a')",
            "SELECT id FROM {v} WHERE owner = 'one' AND title LIKE '%a' ORDER BY id LIMIT 1 OFFSET 1",
            "SELECT a.id FROM {v} a WHERE EXISTS (SELECT l.id FROM local l WHERE l.id = 'a') AND a.id IN (SELECT l.id FROM local l)",
            "SELECT a.id FROM {v} a WHERE EXISTS (SELECT v.title FROM {w} v WHERE v.id = 'a') AND a.id IN (SELECT v.id FROM {w} v WHERE v.id = 'c')",
            "SELECT a.id FROM {v} a WHERE EXISTS (SELECT {w}.title FROM {w} WHERE {w}.id = 'a') AND a.id IN (SELECT {w}.id FROM {w} WHERE {w}.id = 'c')",
        ];
        for mode in [
            NegotiationMode::Exact,
            NegotiationMode::Inexact,
            NegotiationMode::Unsupported,
            NegotiationMode::Alternating,
        ] {
            let (engine, oracle) = engines(mode);
            for template in corpus {
                let virtual_sql = template.replace("{v}", "fx").replace("{w}", "fx2");
                let local_sql = template.replace("{v}", "mirror").replace("{w}", "mirror2");
                let mut actual = run(engine.select(select(&virtual_sql), &BTreeMap::new()))
                    .unwrap_or_else(|error| panic!("{mode:?}: {virtual_sql}: {error}"));
                let mut expected = run(oracle.select(select(&local_sql), &BTreeMap::new()))
                    .unwrap_or_else(|error| panic!("oracle: {local_sql}: {error}"));
                if !template.contains("ORDER BY") {
                    actual.sort_by_key(|row| format!("{row:?}"));
                    expected.sort_by_key(|row| format!("{row:?}"));
                }
                assert_eq!(actual, expected, "{mode:?}: {virtual_sql}");
            }
        }
    }

    #[cfg(feature = "sql")]
    #[test]
    fn sibling_subqueries_keep_distinct_leaf_plans_and_tokens() {
        struct TokenSource {
            inner: Arc<dyn QuerySource>,
            scans: Arc<std::sync::Mutex<Vec<SourceScan>>>,
        }
        fn token(collection: &str, request: &ScanRequest) -> Value {
            Value::String(format!("{collection}:{:?}", request.filters))
        }
        #[async_trait]
        impl QuerySource for TokenSource {
            async fn negotiate(
                &self,
                collection: &str,
                request: &ScanRequest,
            ) -> Result<ScanPlan, DbError> {
                let mut plan = self.inner.negotiate(collection, request).await?;
                if let ScanPlan::Accepted { plan } = &mut plan {
                    plan.token = Some(token(collection, request));
                }
                Ok(plan)
            }
            fn scan(self: Arc<Self>, scan: SourceScan) -> SendableRecordBatchStream {
                assert_eq!(
                    scan.plan.token,
                    Some(token(&scan.collection, &scan.request))
                );
                self.scans.lock().unwrap().push(scan.clone());
                self.inner.clone().scan(scan)
            }
        }
        for mode in [
            NegotiationMode::Exact,
            NegotiationMode::Inexact,
            NegotiationMode::Unsupported,
        ] {
            for (collection, query) in [
                (
                    "local",
                    "SELECT a.id FROM fx a WHERE EXISTS (SELECT l.id FROM local l WHERE l.id = 'a') AND a.id IN (SELECT l.id FROM local l WHERE l.id = 'c')",
                ),
                (
                    "fx2",
                    "SELECT a.id FROM fx a WHERE EXISTS (SELECT v.title FROM fx2 v WHERE v.id = 'a') AND a.id IN (SELECT v.id FROM fx2 v WHERE v.id = 'c')",
                ),
                (
                    "fx2",
                    "SELECT a.id FROM fx a WHERE EXISTS (SELECT fx2.title FROM fx2 WHERE fx2.id = 'a') AND a.id IN (SELECT fx2.id FROM fx2 WHERE fx2.id = 'c')",
                ),
            ] {
                let (mut engine, _) = engines(mode);
                let scans = Arc::new(std::sync::Mutex::new(Vec::new()));
                engine.sources.local = Arc::new(TokenSource {
                    inner: engine.sources.local.clone(),
                    scans: scans.clone(),
                });
                for source in engine.sources.virtual_sources.values_mut() {
                    source.source = Arc::new(TokenSource {
                        inner: source.source.clone(),
                        scans: scans.clone(),
                    });
                }
                let explanation = run(engine.explain(select(query), &BTreeMap::new())).unwrap();
                let leaves = super::super::planner::collect_leaves(&explanation.logical);
                assert_eq!(leaves.len(), 3);
                let occurrences = leaves
                    .iter()
                    .map(|leaf| leaf.key.occurrence_id)
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(occurrences.len(), 3);
                assert!(!occurrences.contains(&None));
                assert_eq!(
                    run(engine.select(select(query), &BTreeMap::new())).unwrap(),
                    vec![Object::from_iter([(
                        "id".into(),
                        Value::String("c".into())
                    )])],
                    "{mode:?}: {query}"
                );
                let scans = scans.lock().unwrap();
                let nested = scans
                    .iter()
                    .filter(|scan| scan.collection == collection)
                    .collect::<Vec<_>>();
                assert_eq!(nested.len(), 2);
                assert_ne!(nested[0].plan.token, nested[1].plan.token);
                assert_ne!(nested[0].request.filters, nested[1].request.filters);
            }
        }
        assert!(!format!("{:?}", crate::SourceRef::unnamed()).contains("occurrence_id"));
    }

    #[cfg(feature = "sql")]
    #[test]
    fn plain_field_format_and_explain_wire_preserve_negotiation_details() {
        let (engine, _) = engines(NegotiationMode::Inexact);
        let query = select("SELECT * FROM fx WHERE title = 'Alpha'");
        let explain = run(engine.explain(query.clone(), &BTreeMap::new())).unwrap();
        assert_eq!(explain.leaves.len(), 1);
        assert_eq!(explain.leaves[0].filters[0].1, FilterSupport::Inexact);
        assert!(explain.leaves[0].residual.is_some());
        assert_eq!(explain.leaves[0].estimated_rows, Some(4));
        assert!(matches!(
            FederatedExplain::semantic_type().kind,
            semantic_data::schema::TypeKind::Record(_)
        ));
        let Value::Object(wire) = explain.into_value() else {
            panic!("object")
        };
        assert!(matches!(wire.get("logical"), Some(Value::String(_))));
        assert!(
            matches!(wire.get("physical"), Some(Value::String(tree)) if tree.to_ascii_lowercase().contains("scan"))
        );
        let Some(Value::List(leaves)) = wire.get("leaves") else {
            panic!("leaves")
        };
        let Value::Object(leaf) = &leaves[0] else {
            panic!("leaf")
        };
        assert_eq!(leaf.get("estimated_rows"), Some(&Value::U64(4)));
        let mut query = query;
        query.field_format = FieldFormat::Plain;
        let rows = run(engine.select(query, &BTreeMap::new())).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("title"), Some(&Value::String("Alpha".into())));
        assert!(!rows[0].contains_key("virtual:title"));
    }

    #[cfg(feature = "sql")]
    #[test]
    fn plain_join_attribute_projection_returns_values() {
        let (engine, _) = engines(NegotiationMode::Exact);
        let query =
            select("SELECT b.title AS title FROM local a JOIN fx b ON a.id = b.id ORDER BY a.id");
        let rows = run(engine.select(query, &BTreeMap::new())).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.get("title").cloned())
                .collect::<Vec<_>>(),
            vec![
                Some(Value::String("Alpha".into())),
                Some(Value::String("Gamma".into()))
            ]
        );
    }

    #[test]
    fn engine_propagates_schema_revision_changes() {
        let (mut engine, _) = engines(NegotiationMode::Exact);
        engine.sources.virtual_sources.get_mut("fx").unwrap().source = Arc::new(MemorySource {
            rows: BTreeMap::new(),
            mode: NegotiationMode::Exact,
            revision: "2".into(),
        });
        assert!(
            matches!(run(engine.select(semantic_data::query::SelectQuery::new().with_collection("fx"), &BTreeMap::new())), Err(FederatedError::SchemaChanged { collection }) if collection == "fx")
        );
    }

    struct HostOffsetSource {
        inner: Arc<MemorySource>,
        requests: std::sync::Mutex<Vec<ScanRequest>>,
    }
    #[async_trait]
    impl QuerySource for HostOffsetSource {
        async fn negotiate(&self, name: &str, request: &ScanRequest) -> Result<ScanPlan, DbError> {
            self.requests.lock().unwrap().push(request.clone());
            let ScanPlan::Accepted { mut plan } = self.inner.negotiate(name, request).await? else {
                unreachable!()
            };
            // Reporting an applied zero offset is a valid no-op. Nonzero offsets
            // are deliberately left to the host while limit remains accepted.
            plan.offset_applied = request.offset == 0;
            Ok(ScanPlan::Accepted { plan })
        }
        fn scan(self: Arc<Self>, scan: SourceScan) -> SendableRecordBatchStream {
            self.inner.clone().scan(scan)
        }
    }
    #[test]
    fn host_offsets_keep_sufficient_rows_and_arithmetic_offsets() {
        let (mut engine, _) = engines(NegotiationMode::Exact);
        let source = Arc::new(HostOffsetSource {
            inner: Arc::new(MemorySource {
                rows: BTreeMap::from([(
                    "fx".into(),
                    vec![
                        row("a", "Alpha", "one"),
                        row("b", "Beta", "one"),
                        row("c", "Gamma", "two"),
                        row("d", "Delta", "two"),
                    ],
                )]),
                mode: NegotiationMode::Exact,
                revision: "1".into(),
            }),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        engine.sources.virtual_sources.get_mut("fx").unwrap().source = source.clone();
        for arithmetic in [false, true] {
            let mut query = semantic_data::query::SelectQuery::new()
                .with_collection("fx")
                .with_limit(1usize)
                .with_offset(2usize);
            if arithmetic {
                query.offset = semantic_data::query::Expr::Binary {
                    op: semantic_data::query::BinaryOp::Add,
                    left: Box::new(1usize.into()),
                    right: Box::new(1usize.into()),
                };
            }
            let rows = run(engine.select(query, &BTreeMap::new())).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get("id"), Some(&Value::String("c".into())));
        }
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests[0].limit, Some(3));
        assert_eq!(requests[0].offset, 2);
        assert_eq!(requests[1].limit, None);
        assert_eq!(requests[1].offset, 0);
    }

    struct EndlessSource(Arc<AtomicBool>);
    struct DropGuard(Arc<AtomicBool>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    #[async_trait]
    impl QuerySource for EndlessSource {
        async fn negotiate(&self, _: &str, request: &ScanRequest) -> Result<ScanPlan, DbError> {
            let mut plan = accepted(request);
            plan.limit_applied = false;
            plan.offset_applied = false;
            Ok(ScanPlan::Accepted { plan })
        }
        fn scan(self: Arc<Self>, _: SourceScan) -> SendableRecordBatchStream {
            stream::unfold(DropGuard(self.0.clone()), |guard| async move {
                Some((
                    Ok(vec![Box::new(row("a", "Alpha", "one")) as DynObject]),
                    guard,
                ))
            })
            .boxed()
        }
    }

    #[test]
    fn limit_cancels_an_endless_source() {
        let (mut engine, _) = engines(NegotiationMode::Exact);
        let dropped = Arc::new(AtomicBool::new(false));
        engine.sources.virtual_sources.get_mut("fx").unwrap().source =
            Arc::new(EndlessSource(dropped.clone()));
        let rows = run(engine.select(
            semantic_data::query::SelectQuery::new()
                .with_collection("fx")
                .with_limit(1usize),
            &BTreeMap::new(),
        ))
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(dropped.load(Ordering::SeqCst));
    }
}
