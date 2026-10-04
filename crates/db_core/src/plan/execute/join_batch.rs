//! Bounded lookup groups for sources that explicitly opt in.
use super::*;

pub(super) fn execute_batched_index_join(
    join: PhysicalJoinPlan,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: QueryContext,
    options: ExecutionOptions,
    metrics: OperatorMetrics,
    size: NonZeroUsize,
) -> RecordBatchStream<'_> {
    let left = execute_physical_dyn_stream(
        *join.left.clone(),
        source.clone(),
        context.child(0),
        options,
    );
    // Only one group's lookup is active. Existing upstream row batches and
    // per-group right fanout/output still have their own memory requirements.
    let groups: RecordBatchStream<'_> = left
        .map_ok(|rows: RowBatch| stream::iter(rows.into_iter().map(Ok::<DynObject, CoreError>)))
        .try_flatten()
        .try_chunks(size.get())
        .map_err(|error| error.1)
        .boxed();
    stream::try_unfold(groups, move |mut groups| {
        let source = source.clone();
        let join = join.clone();
        let metrics = metrics.clone();
        async move {
            let Some(left_rows) = groups.try_next().await? else {
                return Ok::<_, CoreError>(None);
            };
            metrics.record_join_probe(left_rows.len());
            let probe = join
                .index_probe
                .as_ref()
                .expect("indexed join requires probe");
            let PhysicalJoinCondition::Eq { left, .. } = &join.condition else {
                unreachable!()
            };
            let keys = left_rows
                .iter()
                .filter_map(|row| {
                    value_ref_for_join_key(row.as_ref(), left).map(ValueRef::into_owned)
                })
                .filter(|value| !matches!(value, Value::Null))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let right_rows = if keys.is_empty() {
                Vec::new()
            } else {
                collect_dyn_stream(metrics.record_access(
                    &probe.source,
                    AccessPathKind::IndexProbe,
                    None,
                    source.index_lookup_batch_stream(
                        probe.source.clone(),
                        probe.field.clone(),
                        keys,
                        probe.residual_predicate.clone(),
                    ),
                ))
                .await?
            };
            metrics.record_join_build(right_rows.len());
            let rows = execute_hash_join(&join, left_rows, right_rows)?;
            Ok(Some((rows, groups)))
        }
    })
    .map_ok(move |rows| rows_to_batches(rows, options.batch_size))
    .try_flatten()
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::task::{Context, Poll};

    struct Source {
        outer: Vec<Object>,
        inner: Vec<Object>,
        opt_in: bool,
        keys: Mutex<Vec<Vec<Value>>>,
        many_calls: AtomicUsize,
        pulled: Arc<AtomicUsize>,
        error_at: Option<usize>,
        scan_error_at: Option<usize>,
        pending: bool,
        dropped: Arc<AtomicBool>,
    }
    struct PendingLookup(Arc<AtomicBool>);
    impl futures::Stream for PendingLookup {
        type Item = Result<Vec<DynObject>, CoreError>;
        fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }
    impl Drop for PendingLookup {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    impl Source {
        fn new(count: usize) -> Self {
            Self {
                outer: (0..count)
                    .map(|i| {
                        Object::from_iter([
                            ("fk".into(), Value::I64((i % 70) as i64)),
                            ("ordinal".into(), Value::I64(i as i64)),
                            ("allow".into(), Value::Bool(true)),
                        ])
                    })
                    .collect(),
                inner: (0..70)
                    .flat_map(|id| {
                        ["first", "second", "inactive"].map(|name| {
                            Object::from_iter([
                                ("id".into(), Value::I64(id)),
                                ("name".into(), Value::String(name.into())),
                                ("active".into(), Value::Bool(name != "inactive")),
                            ])
                        })
                    })
                    .collect(),
                opt_in: true,
                keys: Mutex::new(Vec::new()),
                many_calls: AtomicUsize::new(0),
                pulled: Arc::new(AtomicUsize::new(0)),
                error_at: None,
                scan_error_at: None,
                pending: false,
                dropped: Arc::new(AtomicBool::new(false)),
            }
        }
        fn matching(&self, keys: &[Value], residual: Option<Expr>) -> SendableRecordBatchStream {
            let rows = self
                .inner
                .iter()
                .filter(|row| {
                    keys.contains(row.get("id").unwrap())
                        && residual
                            .as_ref()
                            .is_none_or(|predicate| evaluate_filter_expr(*row, predicate))
                })
                .cloned()
                .map(|row| Box::new(row) as DynObject)
                .collect();
            rows_to_batches(rows, 4)
        }
    }
    impl AsyncPhysicalDataSource for Source {
        fn scan_stream(&self, _: SourceRef) -> SendableRecordBatchStream {
            let pulled = self.pulled.clone();
            let error_at = self.scan_error_at;
            stream::iter(self.outer.clone())
                .enumerate()
                .map(move |(i, row)| {
                    pulled.fetch_add(1, Ordering::SeqCst);
                    if error_at == Some(i) {
                        return Err(CoreError::new("outer failed"));
                    }
                    Ok(vec![Box::new(row) as DynObject])
                })
                .boxed()
        }
        fn index_lookup_batch_size(&self, source: &SourceRef) -> Option<NonZeroUsize> {
            (self.opt_in && source.backend_tag.as_deref() == Some("virtual"))
                .then(|| NonZeroUsize::new(64).unwrap())
        }
        fn index_lookup_many_stream(
            &self,
            _: SourceRef,
            _: FieldRef,
            keys: Vec<Value>,
            residual: Option<Expr>,
        ) -> SendableRecordBatchStream {
            self.many_calls.fetch_add(1, Ordering::SeqCst);
            self.matching(&keys, residual)
        }
        fn index_lookup_batch_stream(
            &self,
            _: SourceRef,
            _: FieldRef,
            keys: Vec<Value>,
            residual: Option<Expr>,
        ) -> SendableRecordBatchStream {
            let mut calls = self.keys.lock().unwrap();
            let ordinal = calls.len();
            calls.push(keys.clone());
            drop(calls);
            if self.error_at == Some(ordinal) {
                return stream::once(async { Err(CoreError::new("lookup failed")) }).boxed();
            }
            if self.pending {
                return PendingLookup(self.dropped.clone()).boxed();
            }
            self.matching(&keys, residual)
        }
    }
    fn field(parts: &[&str]) -> Expr {
        Expr::Operand(Operand::Field(FieldPath::from_fields(
            parts.iter().copied(),
        )))
    }
    fn plan(join_type: JoinType, tag: &str) -> PhysicalPlan {
        let source = SourceRef {
            source_name: Some("right".into()),
            binding: Some("r".into()),
            backend_tag: Some(tag.into()),
            occurrence_id: None,
            collection_id: None,
        };
        PhysicalPlan::Join(PhysicalJoinPlan {
            left: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some("left".into()),
                    binding: Some("l".into()),
                    backend_tag: None,
                    occurrence_id: None,
                    collection_id: None,
                },
            })),
            right: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: source.clone(),
            })),
            join_type,
            algorithm: PhysicalJoinAlgorithm::IndexNestedLoop,
            condition: PhysicalJoinCondition::Eq {
                left: PhysicalJoinKey {
                    field: FieldRef::Path(FieldPath::from_fields(["fk"])),
                    source_path: FieldPath::from_fields(["fk"]),
                },
                right: PhysicalJoinKey {
                    field: FieldRef::Path(FieldPath::from_fields(["id"])),
                    source_path: FieldPath::from_fields(["id"]),
                },
                residual_predicate: Some(Expr::Binary {
                    op: semantic_data::query::BinaryOp::Eq,
                    left: Box::new(field(&["l", "allow"])),
                    right: Box::new(field(&["r", "active"])),
                }),
            },
            index_probe: Some(crate::PhysicalIndexProbe {
                source,
                field: FieldRef::Path(FieldPath::from_fields(["id"])),
                residual_predicate: Some(Expr::Binary {
                    op: semantic_data::query::BinaryOp::Eq,
                    left: Box::new(field(&["active"])),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::Bool(true)))),
                }),
            }),
            left_binding: "l".into(),
            right_binding: "r".into(),
        })
    }
    fn collect(plan: PhysicalPlan, source: Arc<Source>) -> Vec<Object> {
        futures::executor::block_on(execute_physical_plan_collect(
            plan,
            source,
            QueryContext::default(),
            ExecutionOptions::default(),
        ))
        .unwrap()
    }
    fn pair(row: &Object) -> (i64, String) {
        let Some(Value::Object(left)) = row.get("l") else {
            panic!("missing left");
        };
        let Some(Value::Object(right)) = row.get("r") else {
            panic!("missing right");
        };
        let Some(Value::String(name)) = right.get("name") else {
            panic!("missing name");
        };
        (left.get("ordinal").unwrap().as_i64().unwrap(), name.clone())
    }
    #[test]
    fn bounded_groups_preserve_outer_order_fanout_nulls_and_residuals() {
        let mut source = Source::new(130);
        source.outer[5].insert("fk", Value::Null);
        source.outer[65].insert("fk", Value::Null);
        source.outer[129].insert("allow", Value::Bool(false));
        let mut mirror = Source::new(0);
        mirror.outer = source.outer.clone();
        mirror.inner = source.inner.clone();
        mirror.opt_in = false;
        let source = Arc::new(source);
        let rows = collect(plan(JoinType::Inner, "virtual"), source.clone());
        let ordinary = collect(plan(JoinType::Inner, "virtual"), Arc::new(mirror));
        assert_eq!(
            rows.iter().map(pair).collect::<Vec<_>>(),
            ordinary.iter().map(pair).collect::<Vec<_>>()
        );
        let expected = (0..130)
            .filter(|i| ![5, 65, 129].contains(i))
            .flat_map(|i| ["first", "second"].map(|name| (i as i64, name.to_owned())))
            .collect::<Vec<_>>();
        assert_eq!(rows.iter().map(pair).collect::<Vec<_>>(), expected);
        let keys = source.keys.lock().unwrap();
        assert_eq!(keys.len(), 3);
        assert!(
            keys.iter()
                .all(|keys| keys.len() <= 64 && !keys.contains(&Value::Null))
        );
        assert_eq!(source.many_calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn zero_and_batch_boundaries_have_exact_lookup_counts() {
        for count in [0, 1, 63, 64, 65] {
            let source = Arc::new(Source::new(count));
            let rows = collect(plan(JoinType::Inner, "virtual"), source.clone());
            let expected = (0..count)
                .flat_map(|i| ["first", "second"].map(|name| (i as i64, name.to_owned())))
                .collect::<Vec<_>>();
            assert_eq!(rows.iter().map(pair).collect::<Vec<_>>(), expected);
            let keys = source.keys.lock().unwrap();
            assert_eq!(keys.len(), count.div_ceil(64));
            assert_eq!(keys.iter().map(Vec::len).sum::<usize>(), count);
            assert!(keys.iter().all(|keys| keys.len() <= 64));
        }
    }
    #[test]
    fn all_null_group_skips_lookup() {
        let mut source = Source::new(64);
        for row in &mut source.outer {
            row.insert("fk", Value::Null);
        }
        let source = Arc::new(source);
        assert!(collect(plan(JoinType::Inner, "virtual"), source.clone()).is_empty());
        assert_eq!(source.pulled.load(Ordering::SeqCst), 64);
        assert!(source.keys.lock().unwrap().is_empty());
        assert_eq!(source.many_calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn repeated_keys_are_deduplicated_per_group_without_global_cache() {
        let mut source = Source::new(130);
        for row in &mut source.outer {
            row.insert("fk", Value::I64(1));
        }
        let source = Arc::new(source);
        assert_eq!(
            collect(plan(JoinType::Inner, "virtual"), source.clone()).len(),
            260
        );
        assert_eq!(*source.keys.lock().unwrap(), vec![vec![Value::I64(1)]; 3]);
    }
    #[test]
    fn local_outer_and_default_policy_retain_existing_many_hook() {
        for (join_type, tag, opt_in) in [
            (JoinType::Inner, "local", true),
            (JoinType::Left, "virtual", true),
            (JoinType::Inner, "virtual", false),
        ] {
            let mut source = Source::new(130);
            source.opt_in = opt_in;
            let source = Arc::new(source);
            assert_eq!(collect(plan(join_type, tag), source.clone()).len(), 260);
            assert_eq!(source.many_calls.load(Ordering::SeqCst), 1);
            assert!(source.keys.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn cancellation_drops_current_lookup_without_consuming_next_group() {
        let mut source = Source::new(130);
        source.pending = true;
        let source = Arc::new(source);
        let mut output = execute_physical_plan_stream(
            plan(JoinType::Inner, "virtual"),
            source.clone(),
            QueryContext::default(),
            ExecutionOptions::default(),
        );
        let mut context = Context::from_waker(futures::task::noop_waker_ref());
        assert!(output.as_mut().poll_next(&mut context).is_pending());
        assert_eq!(source.pulled.load(Ordering::SeqCst), 64);
        assert_eq!(source.keys.lock().unwrap().len(), 1);
        drop(output);
        assert!(source.dropped.load(Ordering::SeqCst));
        assert_eq!(source.pulled.load(Ordering::SeqCst), 64);
    }
    #[test]
    fn lookup_error_is_terminal_even_when_consumer_polls_again() {
        for error_at in [0, 1] {
            let mut source = Source::new(130);
            source.error_at = Some(error_at);
            let source = Arc::new(source);
            futures::executor::block_on(async {
                let mut output = execute_physical_plan_stream(
                    plan(JoinType::Inner, "virtual"),
                    source.clone(),
                    QueryContext::default(),
                    ExecutionOptions {
                        batch_size: 16,
                        ..Default::default()
                    },
                );
                let mut count = 0;
                loop {
                    match output.next().await.unwrap() {
                        Ok(rows) => count += rows.len(),
                        Err(error) => {
                            assert!(error.to_string().contains("lookup failed"));
                            break;
                        }
                    }
                }
                assert_eq!(count, error_at * 128);
                assert!(output.next().await.is_none());
                assert!(output.next().await.is_none());
            });
            assert_eq!(source.pulled.load(Ordering::SeqCst), (error_at + 1) * 64);
            assert_eq!(source.keys.lock().unwrap().len(), error_at + 1);
        }
    }
    #[test]
    fn outer_error_terminates_after_completed_group() {
        let mut source = Source::new(130);
        source.scan_error_at = Some(65);
        let source = Arc::new(source);
        futures::executor::block_on(async {
            let mut output = execute_physical_plan_stream(
                plan(JoinType::Inner, "virtual"),
                source.clone(),
                QueryContext::default(),
                ExecutionOptions::default(),
            );
            assert_eq!(output.next().await.unwrap().unwrap().len(), 128);
            let error = match output.next().await.unwrap() {
                Err(error) => error,
                Ok(_) => panic!("outer error expected"),
            };
            assert!(error.to_string().contains("outer failed"));
            assert!(output.next().await.is_none());
        });
        assert_eq!(source.pulled.load(Ordering::SeqCst), 66);
        assert_eq!(source.keys.lock().unwrap().len(), 1);
    }
    #[test]
    fn limit_stops_before_second_lookup_group() {
        let source = Arc::new(Source::new(130));
        let limited = PhysicalPlan::Limit {
            input: Box::new(plan(JoinType::Inner, "virtual")),
            offset: Expr::Operand(Operand::Literal(Value::U64(0))),
            limit: Some(Expr::Operand(Operand::Literal(Value::U64(1)))),
        };
        assert_eq!(collect(limited, source.clone()).len(), 1);
        assert_eq!(source.pulled.load(Ordering::SeqCst), 64);
        assert_eq!(source.keys.lock().unwrap().len(), 1);
    }
    #[test]
    fn borrowed_adapter_forwards_batch_policy_and_hook() {
        let source = Source::new(130);
        let rows = execute_physical_plan_with_source(
            &plan(JoinType::Inner, "virtual"),
            &source,
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(rows.len(), 260);
        assert_eq!(source.keys.lock().unwrap().len(), 3);
        assert_eq!(source.many_calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn default_batch_hook_delegates_specialized_many_lookup() {
        struct DefaultHook(Source);
        impl AsyncPhysicalDataSource for DefaultHook {
            fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
                self.0.scan_stream(source)
            }
            fn index_lookup_many_stream(
                &self,
                source: SourceRef,
                field: FieldRef,
                keys: Vec<Value>,
                residual: Option<Expr>,
            ) -> SendableRecordBatchStream {
                self.0
                    .index_lookup_many_stream(source, field, keys, residual)
            }
        }
        let source = DefaultHook(Source::new(0));
        let reference = SourceRef::unnamed();
        assert!(source.index_lookup_batch_size(&reference).is_none());
        let rows =
            futures::executor::block_on(collect_dyn_stream(source.index_lookup_batch_stream(
                reference,
                FieldRef::Path(FieldPath::from_fields(["id"])),
                vec![Value::I64(1), Value::I64(1)],
                None,
            )))
            .unwrap();
        assert_eq!(rows.len(), 3); // Specialized many hook deduplicates duplicate probe keys.
        assert_eq!(source.0.many_calls.load(Ordering::SeqCst), 1);
    }
}
