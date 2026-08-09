use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Arc;

use futures::{
    FutureExt, StreamExt, TryStreamExt,
    future::BoxFuture,
    stream::{self, BoxStream},
};
use semantic_data::query::{AggregateOp, JoinType};
use semantic_data::value::{FieldPath, Object, PathSegment, Value, ValueRef};

use crate::QueryContext;
use crate::plan::{
    FieldRef, Optimizer, PhysicalJoinAlgorithm, PhysicalJoinCondition, PhysicalJoinKey,
    PhysicalJoinPlan, PhysicalOrderField, PhysicalPlan, PhysicalProjectionField, PhysicalSource,
    SourceRef, source_ref_for_collection,
};
use crate::query::{
    CoreError, CoreResult, Expr, FunctionArg, ObjectAccess as QueryObjectAccess, Operand,
    compare_objects_for_plan, evaluate_expr, evaluate_filter_expr, evaluate_usize_expr,
};

pub type DynObject = Box<dyn QueryObjectAccess>;
pub type RowBatch = Vec<DynObject>;
pub type SendableRecordBatchStream = BoxStream<'static, CoreResult<RowBatch>>;
type RecordBatchStream<'a> = BoxStream<'a, CoreResult<RowBatch>>;

pub const DEFAULT_EXECUTION_BATCH_SIZE: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionOptions {
    pub batch_size: usize,
    pub parallel_union_branches: bool,
    pub parallel_join_inputs: bool,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_EXECUTION_BATCH_SIZE,
            parallel_union_branches: true,
            parallel_join_inputs: true,
        }
    }
}

pub trait AsyncPhysicalDataSource: Send + Sync {
    fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream;

    fn scan_filtered_stream(
        &self,
        source: SourceRef,
        predicate: Expr,
    ) -> SendableRecordBatchStream {
        filter_batch_stream(self.scan_stream(source), predicate)
    }

    fn index_lookup_stream(
        &self,
        source: SourceRef,
        field: FieldRef,
        value: Value,
    ) -> SendableRecordBatchStream {
        filter_batch_stream(
            self.scan_stream(source),
            Expr::Binary {
                op: semantic_data::query::BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(
                    field_path_for_ref(&field).unwrap_or_else(|| FieldPath::from_fields(["id"])),
                ))),
                right: Box::new(Expr::Operand(Operand::Literal(value))),
            },
        )
    }

    fn scan(&self, source: SourceRef) -> BoxFuture<'static, CoreResult<Vec<DynObject>>> {
        collect_dyn_stream(self.scan_stream(source)).boxed()
    }
}

pub fn execute_physical_plan_stream(
    plan: PhysicalPlan,
    source: Arc<dyn AsyncPhysicalDataSource>,
    context: QueryContext,
    options: ExecutionOptions,
) -> SendableRecordBatchStream {
    normalize_record_batch_stream(execute_physical_dyn_stream(
        plan,
        source,
        context,
        normalize_options(options),
    ))
}

pub async fn execute_physical_plan_collect(
    plan: PhysicalPlan,
    source: Arc<dyn AsyncPhysicalDataSource>,
    context: QueryContext,
    options: ExecutionOptions,
) -> CoreResult<Vec<Object>> {
    Ok(
        collect_dyn_stream(execute_physical_plan_stream(plan, source, context, options))
            .await?
            .into_iter()
            .map(|row| row.to_object())
            .collect(),
    )
}

fn normalize_options(mut options: ExecutionOptions) -> ExecutionOptions {
    if options.batch_size == 0 {
        options.batch_size = DEFAULT_EXECUTION_BATCH_SIZE;
    }
    options
}

fn execute_physical_dyn_stream(
    plan: PhysicalPlan,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: QueryContext,
    options: ExecutionOptions,
) -> RecordBatchStream<'_> {
    let stream = match plan {
        PhysicalPlan::Source(PhysicalSource::Scan { source: source_ref }) => {
            source.scan_stream(source_ref)
        }
        PhysicalPlan::Source(PhysicalSource::FilteredScan {
            source: source_ref,
            predicate,
        }) => {
            if !expr_contains_subquery(&predicate) {
                return source.scan_filtered_stream(source_ref, predicate);
            }
            stream::once(async move {
                let predicate =
                    resolve_expr_subqueries_async(&predicate, source.clone(), &context, options)
                        .await?;
                Ok(rows_to_batches(
                    filter_dyn_rows(
                        collect_dyn_stream(source.scan_stream(source_ref)).await?,
                        &predicate,
                    ),
                    options.batch_size,
                ))
            })
            .try_flatten()
            .boxed()
        }
        PhysicalPlan::Source(PhysicalSource::IndexLookup {
            source: source_ref,
            field,
            value,
            residual_predicate,
        }) => {
            let base = source.index_lookup_stream(source_ref, field, value);
            if let Some(residual) = residual_predicate {
                stream::once(async move {
                    let residual =
                        resolve_expr_subqueries_async(&residual, source, &context, options).await?;
                    Ok(rows_to_batches(
                        filter_dyn_rows(collect_dyn_stream(base).await?, &residual),
                        options.batch_size,
                    ))
                })
                .try_flatten()
                .boxed()
            } else {
                base
            }
        }
        PhysicalPlan::Values { values } => rows_to_batches(
            values
                .into_iter()
                .map(|item| Box::new(item) as DynObject)
                .collect(),
            options.batch_size,
        ),
        PhysicalPlan::Filter { input, predicate } => stream::once(async move {
            let predicate =
                resolve_expr_subqueries_async(&predicate, source.clone(), &context, options)
                    .await?;
            Ok(filter_batch_stream(
                execute_physical_dyn_stream(*input, source, context, options),
                predicate,
            ))
        })
        .try_flatten()
        .boxed(),
        PhysicalPlan::Sort { input, order_by } => stream::once(async move {
            let mut out = collect_dyn_stream(execute_physical_dyn_stream(
                *input,
                source.clone(),
                context.clone(),
                options,
            ))
            .await?;
            let order_by =
                resolve_order_by_subqueries_async(&order_by, source, &context, options).await?;
            out.sort_by(|a, b| compare_dyn_objects(a.as_ref(), b.as_ref(), &order_by));
            Ok(rows_to_batches(out, options.batch_size))
        })
        .try_flatten()
        .boxed(),
        PhysicalPlan::Project { input, projection } => stream::once(async move {
            let projection =
                resolve_projection_subqueries_async(&projection, source.clone(), &context, options)
                    .await?;
            Ok(project_batch_stream(
                execute_physical_dyn_stream(*input, source, context, options),
                projection,
            ))
        })
        .try_flatten()
        .boxed(),
        PhysicalPlan::Aggregate {
            input,
            group_by,
            projection,
            having,
        } => stream::once(async move {
            let group_by =
                resolve_expr_list_subqueries_async(&group_by, source.clone(), &context, options)
                    .await?;
            let projection =
                resolve_projection_subqueries_async(&projection, source.clone(), &context, options)
                    .await?;
            let having = match having {
                Some(expr) => Some(
                    resolve_expr_subqueries_async(&expr, source.clone(), &context, options).await?,
                ),
                None => None,
            };
            let rows = collect_dyn_stream(execute_physical_dyn_stream(
                *input, source, context, options,
            ))
            .await?;
            execute_aggregate(rows, &group_by, &projection, &having)
        })
        .map_ok(move |rows| rows_to_batches(rows, options.batch_size))
        .try_flatten()
        .boxed(),
        PhysicalPlan::Limit {
            input,
            offset,
            limit,
        } => stream::once(async move {
            let offset =
                resolve_expr_subqueries_async(&offset, source.clone(), &context, options).await?;
            let limit = match limit {
                Some(expr) => Some(
                    resolve_expr_subqueries_async(&expr, source.clone(), &context, options).await?,
                ),
                None => None,
            };
            let offset = evaluate_usize_expr(&offset)
                .ok_or_else(|| CoreError::new("failed to evaluate OFFSET expression"))?;
            let limit = limit
                .as_ref()
                .map(|expr| {
                    evaluate_usize_expr(expr)
                        .ok_or_else(|| CoreError::new("failed to evaluate LIMIT expression"))
                })
                .transpose()?;
            Ok(limit_batch_stream(
                execute_physical_dyn_stream(*input, source, context, options),
                offset,
                limit,
            ))
        })
        .try_flatten()
        .boxed(),
        PhysicalPlan::Distinct { input } => distinct_batch_stream(execute_physical_dyn_stream(
            *input, source, context, options,
        )),
        PhysicalPlan::Union { inputs, all } => {
            if options.parallel_union_branches {
                let streams = inputs
                    .into_iter()
                    .map(|input| {
                        execute_physical_dyn_stream(input, source.clone(), context.clone(), options)
                    })
                    .collect::<Vec<_>>();
                let merged = stream::select_all(streams).boxed();
                if all {
                    merged.boxed()
                } else {
                    distinct_batch_stream(merged)
                }
            } else {
                let streams = stream::iter(inputs.into_iter().map(move |input| {
                    execute_physical_dyn_stream(input, source.clone(), context.clone(), options)
                }))
                .flatten()
                .boxed();
                if all {
                    streams.boxed()
                } else {
                    distinct_batch_stream(streams)
                }
            }
        }
        PhysicalPlan::Join(join) => execute_join_stream(join, source, context, options),
        PhysicalPlan::ApplyExists {
            input,
            subquery,
            negated,
        } => stream::once(async move {
            let subquery_any = !collect_dyn_stream(execute_physical_dyn_stream(
                *subquery,
                source.clone(),
                context.clone(),
                options,
            ))
            .await?
            .is_empty();
            Ok(filter_bool_batch_stream(
                execute_physical_dyn_stream(*input, source, context, options),
                if negated { !subquery_any } else { subquery_any },
            ))
        })
        .try_flatten()
        .boxed(),
        PhysicalPlan::ApplyInSubquery {
            input,
            left,
            subquery,
            negated,
        } => stream::once(async move {
            let shape = physical_subquery_single_column_shape(&subquery)?;
            let sub_values: BTreeSet<Value> = collect_dyn_stream(execute_physical_dyn_stream(
                *subquery,
                source.clone(),
                context.clone(),
                options,
            ))
            .await?
            .into_iter()
            .map(|row| single_column_subquery_value(row.to_object(), shape))
            .collect::<CoreResult<_>>()?;
            let left =
                resolve_expr_subqueries_async(&left, source.clone(), &context, options).await?;
            Ok(filter_in_subquery_batch_stream(
                execute_physical_dyn_stream(*input, source, context, options),
                left,
                sub_values,
                negated,
            ))
        })
        .try_flatten()
        .boxed(),
        PhysicalPlan::Exchange { input, .. }
        | PhysicalPlan::RepartitionHash { input, .. }
        | PhysicalPlan::Materialize { input } => {
            execute_physical_dyn_stream(*input, source, context, options)
        }
    };

    normalize_record_batch_stream(stream)
}

fn rows_to_batches(rows: Vec<DynObject>, batch_size: usize) -> SendableRecordBatchStream {
    let batch_size = batch_size.max(1);
    stream::unfold(rows.into_iter(), move |mut iter| async move {
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            let Some(row) = iter.next() else {
                break;
            };
            batch.push(row);
        }
        if batch.is_empty() {
            None
        } else {
            Some((Ok(batch), iter))
        }
    })
    .boxed()
}

fn normalize_record_batch_stream<'a>(input: RecordBatchStream<'a>) -> RecordBatchStream<'a> {
    input
        .filter(|item| futures::future::ready(!matches!(item, Ok(batch) if batch.is_empty())))
        .boxed()
}

async fn collect_dyn_stream(stream: RecordBatchStream<'_>) -> CoreResult<Vec<DynObject>> {
    let batches = normalize_record_batch_stream(stream)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(batches.into_iter().flatten().collect())
}

fn filter_dyn_rows(rows: Vec<DynObject>, predicate: &Expr) -> Vec<DynObject> {
    rows.into_iter()
        .filter(|row| evaluate_filter_expr(row.as_ref(), predicate))
        .collect()
}

fn filter_batch_stream(input: RecordBatchStream<'_>, predicate: Expr) -> RecordBatchStream<'_> {
    normalize_record_batch_stream(
        input
            .map_ok(move |batch| filter_dyn_rows(batch, &predicate))
            .boxed(),
    )
}

fn filter_bool_batch_stream(input: RecordBatchStream<'_>, keep: bool) -> RecordBatchStream<'_> {
    if keep { input } else { stream::empty().boxed() }
}

fn filter_in_subquery_batch_stream(
    input: RecordBatchStream<'_>,
    left: Expr,
    sub_values: BTreeSet<Value>,
    negated: bool,
) -> RecordBatchStream<'_> {
    if sub_values.is_empty() {
        return filter_bool_batch_stream(input, negated);
    }
    let contains_null = sub_values.iter().any(Value::is_nullish);
    normalize_record_batch_stream(
        input
            .map_ok(move |batch| {
                batch
                    .into_iter()
                    .filter(|row| {
                        let Some(value) = evaluate_expr(row.as_ref(), &left) else {
                            return false;
                        };
                        if value.is_nullish() {
                            return false;
                        }
                        let contains = sub_values.contains(&value);
                        if negated {
                            !contains && !contains_null
                        } else {
                            contains
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .boxed(),
    )
}

fn project_batch_stream(
    input: RecordBatchStream<'_>,
    projection: Vec<PhysicalProjectionField>,
) -> RecordBatchStream<'_> {
    input
        .map_ok(move |batch| {
            batch
                .into_iter()
                .map(|row| Box::new(project_dyn_object(row.as_ref(), &projection)) as DynObject)
                .collect::<Vec<_>>()
        })
        .boxed()
}

fn limit_batch_stream(
    input: RecordBatchStream<'_>,
    offset: usize,
    limit: Option<usize>,
) -> RecordBatchStream<'_> {
    stream::unfold(
        (input, offset, limit, false),
        |(mut input, mut offset, mut remaining, done)| async move {
            if done {
                return None;
            }
            loop {
                let item = input.next().await?;
                let mut batch = match item {
                    Ok(batch) => batch,
                    Err(err) => return Some((Err(err), (input, offset, remaining, true))),
                };
                if offset >= batch.len() {
                    offset -= batch.len();
                    continue;
                }
                if offset > 0 {
                    batch = batch.into_iter().skip(offset).collect();
                    offset = 0;
                }
                if let Some(left) = remaining {
                    if left == 0 {
                        return None;
                    }
                    if batch.len() > left {
                        batch.truncate(left);
                        remaining = Some(0);
                        return Some((Ok(batch), (input, offset, remaining, true)));
                    }
                    remaining = Some(left - batch.len());
                }
                if batch.is_empty() {
                    continue;
                }
                return Some((Ok(batch), (input, offset, remaining, false)));
            }
        },
    )
    .boxed()
}

fn distinct_batch_stream(input: RecordBatchStream<'_>) -> RecordBatchStream<'_> {
    stream::unfold(
        (input, BTreeSet::<Object>::new()),
        |(mut input, mut dedup)| async move {
            loop {
                let item = input.next().await?;
                let batch = match item {
                    Ok(batch) => batch,
                    Err(err) => return Some((Err(err), (input, dedup))),
                };
                let out = batch
                    .into_iter()
                    .filter(|item| dedup.insert(item.to_object()))
                    .collect::<Vec<_>>();
                if out.is_empty() {
                    continue;
                }
                return Some((Ok(out), (input, dedup)));
            }
        },
    )
    .boxed()
}

fn execute_join_stream(
    mut join: PhysicalJoinPlan,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: QueryContext,
    options: ExecutionOptions,
) -> RecordBatchStream<'_> {
    stream::once(async move {
        if let PhysicalJoinCondition::Predicate(predicate) = &join.condition {
            join.condition = PhysicalJoinCondition::Predicate(
                resolve_expr_subqueries_async(predicate, source.clone(), &context, options).await?,
            );
        }
        let out = match join.algorithm {
            PhysicalJoinAlgorithm::Hash
                if matches!(join.condition, PhysicalJoinCondition::Eq { .. }) =>
            {
                execute_hash_join_stream(join, source.clone(), context.clone(), options)
            }
            PhysicalJoinAlgorithm::Hash
            | PhysicalJoinAlgorithm::NestedLoop
            | PhysicalJoinAlgorithm::Merge => {
                execute_nested_loop_join_stream(join, source.clone(), context.clone(), options)
            }
        };
        Ok(out)
    })
    .try_flatten()
    .boxed()
}

fn execute_hash_join_stream(
    join: PhysicalJoinPlan,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: QueryContext,
    options: ExecutionOptions,
) -> RecordBatchStream<'_> {
    stream::once(async move {
        let left_stream = execute_physical_dyn_stream(
            *join.left.clone(),
            source.clone(),
            context.clone(),
            options,
        );
        let right_stream = execute_physical_dyn_stream(
            *join.right.clone(),
            source.clone(),
            context.clone(),
            options,
        );
        let (left_rows, right_rows) = if options.parallel_join_inputs {
            futures::try_join!(
                collect_dyn_stream(left_stream),
                collect_dyn_stream(right_stream)
            )?
        } else {
            let left_rows = collect_dyn_stream(left_stream).await?;
            let right_rows = collect_dyn_stream(right_stream).await?;
            (left_rows, right_rows)
        };
        execute_hash_join(&join, left_rows, right_rows)
    })
    .map_ok(move |rows| rows_to_batches(rows, options.batch_size))
    .try_flatten()
    .boxed()
}

fn execute_nested_loop_join_stream(
    join: PhysicalJoinPlan,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: QueryContext,
    options: ExecutionOptions,
) -> RecordBatchStream<'_> {
    stream::once(async move {
        let right_rows = collect_dyn_stream(execute_physical_dyn_stream(
            *join.right.clone(),
            source.clone(),
            context.clone(),
            options,
        ))
        .await?;
        let left_stream = execute_physical_dyn_stream(*join.left.clone(), source, context, options);
        Ok(nested_loop_join_left_stream(
            join,
            left_stream,
            right_rows,
            options.batch_size,
        ))
    })
    .try_flatten()
    .boxed()
}

fn nested_loop_join_left_stream(
    join: PhysicalJoinPlan,
    left_stream: RecordBatchStream<'_>,
    right_rows: Vec<DynObject>,
    batch_size: usize,
) -> RecordBatchStream<'_> {
    let batch_size = batch_size.max(1);
    stream::unfold(
        (
            join,
            left_stream,
            right_rows,
            Vec::<bool>::new(),
            VecDeque::<DynObject>::new(),
            false,
            false,
        ),
        move |(
            join,
            mut left_stream,
            right_rows,
            mut right_matched,
            mut pending,
            mut left_done,
            mut unmatched_right_emitted,
        )| async move {
            if right_matched.is_empty() {
                right_matched = vec![false; right_rows.len()];
            }

            loop {
                if pending.len() >= batch_size {
                    let batch = take_pending_batch(&mut pending, batch_size);
                    return Some((
                        Ok(batch),
                        (
                            join,
                            left_stream,
                            right_rows,
                            right_matched,
                            pending,
                            left_done,
                            unmatched_right_emitted,
                        ),
                    ));
                }

                if left_done {
                    if !unmatched_right_emitted {
                        append_unmatched_right_rows(
                            &join,
                            &right_rows,
                            &right_matched,
                            &mut pending,
                        );
                        unmatched_right_emitted = true;
                    }
                    if pending.is_empty() {
                        return None;
                    }
                    let batch = take_pending_batch(&mut pending, batch_size);
                    return Some((
                        Ok(batch),
                        (
                            join,
                            left_stream,
                            right_rows,
                            right_matched,
                            pending,
                            true,
                            unmatched_right_emitted,
                        ),
                    ));
                }

                let Some(item) = left_stream.next().await else {
                    left_done = true;
                    continue;
                };

                let left_batch = match item {
                    Ok(batch) => batch,
                    Err(err) => {
                        return Some((
                            Err(err),
                            (
                                join,
                                left_stream,
                                right_rows,
                                right_matched,
                                pending,
                                true,
                                unmatched_right_emitted,
                            ),
                        ));
                    }
                };

                for left_row in left_batch {
                    append_nested_loop_left_row(
                        &join,
                        left_row.as_ref(),
                        &right_rows,
                        &mut right_matched,
                        &mut pending,
                    );
                    if pending.len() >= batch_size {
                        let batch = take_pending_batch(&mut pending, batch_size);
                        return Some((
                            Ok(batch),
                            (
                                join,
                                left_stream,
                                right_rows,
                                right_matched,
                                pending,
                                false,
                                unmatched_right_emitted,
                            ),
                        ));
                    }
                }
            }
        },
    )
    .boxed()
}

fn take_pending_batch(pending: &mut VecDeque<DynObject>, batch_size: usize) -> Vec<DynObject> {
    let mut batch = Vec::with_capacity(batch_size);
    for _ in 0..batch_size {
        let Some(row) = pending.pop_front() else {
            break;
        };
        batch.push(row);
    }
    batch
}

fn append_nested_loop_left_row(
    join: &PhysicalJoinPlan,
    left_row: &dyn QueryObjectAccess,
    right_rows: &[DynObject],
    right_matched: &mut [bool],
    pending: &mut VecDeque<DynObject>,
) {
    let mut matched_any = false;
    for (right_idx, right_row) in right_rows.iter().enumerate() {
        if join_pair_matches(join, left_row, right_row.as_ref()) {
            matched_any = true;
            right_matched[right_idx] = true;
            pending.push_back(Box::new(bind_join_result(
                Some(left_row),
                Some(right_row.as_ref()),
                &join.left_binding,
                &join.right_binding,
            )) as DynObject);
        }
    }

    if !matched_any && matches!(join.join_type, JoinType::Left | JoinType::Full) {
        pending.push_back(Box::new(bind_join_result(
            Some(left_row),
            None,
            &join.left_binding,
            &join.right_binding,
        )) as DynObject);
    }
}

fn append_unmatched_right_rows(
    join: &PhysicalJoinPlan,
    right_rows: &[DynObject],
    right_matched: &[bool],
    pending: &mut VecDeque<DynObject>,
) {
    if !matches!(join.join_type, JoinType::Right | JoinType::Full) {
        return;
    }

    for (idx, right_row) in right_rows.iter().enumerate() {
        if right_matched[idx] {
            continue;
        }
        pending.push_back(Box::new(bind_join_result(
            None,
            Some(right_row.as_ref()),
            &join.left_binding,
            &join.right_binding,
        )) as DynObject);
    }
}

fn field_path_for_ref(field: &FieldRef) -> Option<FieldPath> {
    match field {
        FieldRef::CanonicalName(name) => Some(FieldPath::from_fields([name])),
        FieldRef::Path(path) => Some(path.clone()),
        FieldRef::AttrId(_) | FieldRef::FieldId(_) => None,
    }
}

pub async fn execute_physical_plan_with_source_async(
    plan: &PhysicalPlan,
    source: &dyn AsyncPhysicalDataSource,
    options: ExecutionOptions,
    context: &QueryContext,
) -> CoreResult<Vec<Object>> {
    let source = Arc::new(BorrowedAsyncPhysicalDataSource { inner: source });
    Ok(collect_dyn_stream(execute_physical_dyn_stream(
        plan.clone(),
        source,
        context.clone(),
        normalize_options(options),
    ))
    .await?
    .into_iter()
    .map(|row| row.to_object())
    .collect())
}

pub fn execute_physical_plan_with_source(
    plan: &PhysicalPlan,
    source: &dyn AsyncPhysicalDataSource,
    context: &QueryContext,
) -> CoreResult<Vec<Object>> {
    run_future_to_completion(execute_physical_plan_with_source_async(
        plan,
        source,
        ExecutionOptions::default(),
        context,
    ))
}

fn run_future_to_completion<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut pool = futures::executor::LocalPool::new();
    pool.run_until(future)
}

pub fn execute_physical_plan(
    plan: &PhysicalPlan,
    rows: Vec<Object>,
    context: &QueryContext,
) -> Vec<Object> {
    struct InlineDataSource {
        rows: Vec<Object>,
    }

    impl AsyncPhysicalDataSource for InlineDataSource {
        fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
            if source.source_name.is_some() || source.collection_id.is_some() {
                return stream::once(async {
                    Err(CoreError::new(
                        "inline physical executor does not support named sources",
                    ))
                })
                .boxed();
            }
            rows_to_batches(
                self.rows
                    .iter()
                    .cloned()
                    .map(|row| Box::new(row) as DynObject)
                    .collect(),
                DEFAULT_EXECUTION_BATCH_SIZE,
            )
        }
    }

    execute_physical_plan_with_source(plan, &InlineDataSource { rows }, context).unwrap_or_default()
}

struct BorrowedAsyncPhysicalDataSource<'a> {
    inner: &'a dyn AsyncPhysicalDataSource,
}

impl AsyncPhysicalDataSource for BorrowedAsyncPhysicalDataSource<'_> {
    fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
        self.inner.scan_stream(source)
    }

    fn scan_filtered_stream(
        &self,
        source: SourceRef,
        predicate: Expr,
    ) -> SendableRecordBatchStream {
        self.inner.scan_filtered_stream(source, predicate)
    }

    fn index_lookup_stream(
        &self,
        source: SourceRef,
        field: FieldRef,
        value: Value,
    ) -> SendableRecordBatchStream {
        self.inner.index_lookup_stream(source, field, value)
    }
}

fn expr_contains_subquery(expr: &Expr) -> bool {
    match expr {
        Expr::Operand(_) => false,
        Expr::Unary { expr, .. } => expr_contains_subquery(expr),
        Expr::Binary { left, right, .. } => {
            expr_contains_subquery(left) || expr_contains_subquery(right)
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_subquery(cond)
                || expr_contains_subquery(then_expr)
                || expr_contains_subquery(else_expr)
        }
        Expr::Coalesce(items) => items.iter().any(expr_contains_subquery),
        Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            FunctionArg::Expr(expr) => expr_contains_subquery(expr),
            FunctionArg::Wildcard => false,
        }),
        Expr::Aggregate { arg, .. } => match arg.as_ref() {
            FunctionArg::Expr(expr) => expr_contains_subquery(expr),
            FunctionArg::Wildcard => false,
        },
        Expr::InList { expr, list, .. } => {
            expr_contains_subquery(expr) || list.iter().any(expr_contains_subquery)
        }
        Expr::Subquery(_) | Expr::Exists { .. } => true,
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_subquery(expr)
                || expr_contains_subquery(low)
                || expr_contains_subquery(high)
        }
        Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
            expr_contains_subquery(expr) || expr_contains_subquery(pattern)
        }
        Expr::IsNull { expr, .. } => expr_contains_subquery(expr),
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            expr_contains_subquery(relation)
                || expr_contains_subquery(source)
                || expr_contains_subquery(target)
                || max_depth
                    .as_ref()
                    .is_some_and(|depth| expr_contains_subquery(depth))
        }
    }
}

fn resolve_projection_subqueries_async<'a>(
    projection: &'a [PhysicalProjectionField],
    source: Arc<dyn AsyncPhysicalDataSource + 'a>,
    context: &'a QueryContext,
    options: ExecutionOptions,
) -> BoxFuture<'a, CoreResult<Vec<PhysicalProjectionField>>> {
    async move {
        let mut out = Vec::with_capacity(projection.len());
        for field in projection {
            out.push(PhysicalProjectionField {
                expr: resolve_expr_subqueries_async(&field.expr, source.clone(), context, options)
                    .await?,
                field: field.field.clone(),
                source_path: field.source_path.clone(),
                alias: field.alias.clone(),
                wildcard: field.wildcard.clone(),
            });
        }
        Ok(out)
    }
    .boxed()
}

fn resolve_order_by_subqueries_async<'a>(
    order_by: &'a [PhysicalOrderField],
    source: Arc<dyn AsyncPhysicalDataSource + 'a>,
    context: &'a QueryContext,
    options: ExecutionOptions,
) -> BoxFuture<'a, CoreResult<Vec<PhysicalOrderField>>> {
    async move {
        let mut out = Vec::with_capacity(order_by.len());
        for item in order_by {
            out.push(PhysicalOrderField {
                expr: resolve_expr_subqueries_async(&item.expr, source.clone(), context, options)
                    .await?,
                direction: item.direction,
            });
        }
        Ok(out)
    }
    .boxed()
}

fn resolve_expr_list_subqueries_async<'a>(
    exprs: &'a [Expr],
    source: Arc<dyn AsyncPhysicalDataSource + 'a>,
    context: &'a QueryContext,
    options: ExecutionOptions,
) -> BoxFuture<'a, CoreResult<Vec<Expr>>> {
    async move {
        let mut out = Vec::with_capacity(exprs.len());
        for expr in exprs {
            out.push(resolve_expr_subqueries_async(expr, source.clone(), context, options).await?);
        }
        Ok(out)
    }
    .boxed()
}

fn resolve_expr_subqueries_async<'a>(
    expr: &'a Expr,
    source: Arc<dyn AsyncPhysicalDataSource + 'a>,
    context: &'a QueryContext,
    options: ExecutionOptions,
) -> BoxFuture<'a, CoreResult<Expr>> {
    async move {
        match expr {
            Expr::Operand(_) => Ok(expr.clone()),
            Expr::Unary { op, expr }
                if *op == semantic_data::query::UnaryOp::Not
                    && matches!(
                        expr.as_ref(),
                        Expr::Binary {
                            op: semantic_data::query::BinaryOp::In,
                            right,
                            ..
                        } if matches!(right.as_ref(), Expr::Subquery(_))
                    ) =>
            {
                let Expr::Binary { left, right, .. } = expr.as_ref() else {
                    unreachable!("guard requires binary IN expression");
                };
                let Expr::Subquery(query) = right.as_ref() else {
                    unreachable!("guard requires IN subquery");
                };
                resolve_in_subquery_expr_async(left, query, true, source, context, options).await
            }
            Expr::Unary { op, expr } => Ok(Expr::Unary {
                op: *op,
                expr: Box::new(
                    resolve_expr_subqueries_async(expr, source, context, options).await?,
                ),
            }),
            Expr::Binary { op, left, right } => {
                if *op == semantic_data::query::BinaryOp::In
                    && let Expr::Subquery(query) = right.as_ref()
                {
                    return resolve_in_subquery_expr_async(
                        left, query, false, source, context, options,
                    )
                    .await;
                }
                Ok(Expr::Binary {
                    op: *op,
                    left: Box::new(
                        resolve_expr_subqueries_async(left, source.clone(), context, options)
                            .await?,
                    ),
                    right: Box::new(
                        resolve_expr_subqueries_async(right, source, context, options).await?,
                    ),
                })
            }
            Expr::IfElse {
                cond,
                then_expr,
                else_expr,
            } => Ok(Expr::IfElse {
                cond: Box::new(
                    resolve_expr_subqueries_async(cond, source.clone(), context, options).await?,
                ),
                then_expr: Box::new(
                    resolve_expr_subqueries_async(then_expr, source.clone(), context, options)
                        .await?,
                ),
                else_expr: Box::new(
                    resolve_expr_subqueries_async(else_expr, source, context, options).await?,
                ),
            }),
            Expr::Coalesce(items) => Ok(Expr::Coalesce(
                resolve_expr_list_subqueries_async(items, source, context, options).await?,
            )),
            Expr::Function { name, args } => {
                let mut resolved = Vec::with_capacity(args.len());
                for arg in args {
                    resolved.push(match arg {
                        FunctionArg::Expr(expr) => FunctionArg::Expr(
                            resolve_expr_subqueries_async(expr, source.clone(), context, options)
                                .await?,
                        ),
                        FunctionArg::Wildcard => FunctionArg::Wildcard,
                    });
                }
                Ok(Expr::Function {
                    name: name.clone(),
                    args: resolved,
                })
            }
            Expr::Aggregate { op, distinct, arg } => Ok(Expr::Aggregate {
                op: *op,
                distinct: *distinct,
                arg: Box::new(match arg.as_ref() {
                    FunctionArg::Expr(expr) => FunctionArg::Expr(
                        resolve_expr_subqueries_async(expr, source, context, options).await?,
                    ),
                    FunctionArg::Wildcard => FunctionArg::Wildcard,
                }),
            }),
            Expr::InList {
                expr,
                list,
                negated,
            } => Ok(Expr::InList {
                expr: Box::new(
                    resolve_expr_subqueries_async(expr, source.clone(), context, options).await?,
                ),
                list: resolve_expr_list_subqueries_async(list, source, context, options).await?,
                negated: *negated,
            }),
            Expr::Subquery(query) => Ok(Expr::Operand(Operand::Literal(
                execute_scalar_subquery_async(query, source, context, options).await?,
            ))),
            Expr::Between {
                expr,
                low,
                high,
                negated,
            } => Ok(Expr::Between {
                expr: Box::new(
                    resolve_expr_subqueries_async(expr, source.clone(), context, options).await?,
                ),
                low: Box::new(
                    resolve_expr_subqueries_async(low, source.clone(), context, options).await?,
                ),
                high: Box::new(
                    resolve_expr_subqueries_async(high, source, context, options).await?,
                ),
                negated: *negated,
            }),
            Expr::PatternMatch {
                kind,
                expr,
                pattern,
                case_insensitive,
                negated,
            } => Ok(Expr::PatternMatch {
                kind: *kind,
                expr: Box::new(
                    resolve_expr_subqueries_async(expr, source.clone(), context, options).await?,
                ),
                pattern: Box::new(
                    resolve_expr_subqueries_async(pattern, source, context, options).await?,
                ),
                case_insensitive: *case_insensitive,
                negated: *negated,
            }),
            Expr::RegexMatch {
                expr,
                pattern,
                case_insensitive,
                negated,
            } => Ok(Expr::RegexMatch {
                expr: Box::new(
                    resolve_expr_subqueries_async(expr, source.clone(), context, options).await?,
                ),
                pattern: Box::new(
                    resolve_expr_subqueries_async(pattern, source, context, options).await?,
                ),
                case_insensitive: *case_insensitive,
                negated: *negated,
            }),
            Expr::IsNull { expr, negated } => Ok(Expr::IsNull {
                expr: Box::new(
                    resolve_expr_subqueries_async(expr, source, context, options).await?,
                ),
                negated: *negated,
            }),
            Expr::Exists { query, negated } => {
                let exists =
                    !execute_select_subquery_async(query, source, context, options, Some(1))
                        .await?
                        .is_empty();
                Ok(Expr::Operand(Operand::Literal(Value::Bool(if *negated {
                    !exists
                } else {
                    exists
                }))))
            }
            Expr::RelationExists {
                relation,
                source: relation_source,
                target,
                transitive,
                max_depth,
            } => Ok(Expr::RelationExists {
                relation: Box::new(
                    resolve_expr_subqueries_async(relation, source.clone(), context, options)
                        .await?,
                ),
                source: Box::new(
                    resolve_expr_subqueries_async(
                        relation_source,
                        source.clone(),
                        context,
                        options,
                    )
                    .await?,
                ),
                target: Box::new(
                    resolve_expr_subqueries_async(target, source.clone(), context, options).await?,
                ),
                transitive: *transitive,
                max_depth: match max_depth {
                    Some(expr) => Some(Box::new(
                        resolve_expr_subqueries_async(expr, source, context, options).await?,
                    )),
                    None => None,
                },
            }),
        }
    }
    .boxed()
}

fn resolve_in_subquery_expr_async<'a>(
    left: &'a Expr,
    query: &'a crate::query::SelectQuery,
    negated: bool,
    source: Arc<dyn AsyncPhysicalDataSource + 'a>,
    context: &'a QueryContext,
    options: ExecutionOptions,
) -> BoxFuture<'a, CoreResult<Expr>> {
    async move {
        let values = execute_list_subquery_async(query, source.clone(), context, options).await?;
        if values.is_empty() {
            return Ok(Expr::Operand(Operand::Literal(Value::Bool(negated))));
        }
        let left = resolve_expr_subqueries_async(left, source, context, options).await?;
        let contains_null = values.iter().any(Value::is_nullish);
        let list = values
            .into_iter()
            .filter(|value| !value.is_nullish())
            .map(|value| Expr::Operand(Operand::Literal(value)))
            .collect();
        let membership = Expr::InList {
            expr: Box::new(left.clone()),
            list,
            negated: negated && !contains_null,
        };
        let null = Expr::Operand(Operand::Literal(Value::Null));

        if contains_null {
            Ok(Expr::IfElse {
                cond: Box::new(membership),
                then_expr: Box::new(Expr::Operand(Operand::Literal(Value::Bool(!negated)))),
                else_expr: Box::new(null),
            })
        } else {
            Ok(Expr::IfElse {
                cond: Box::new(Expr::IsNull {
                    expr: Box::new(left),
                    negated: false,
                }),
                then_expr: Box::new(null),
                else_expr: Box::new(membership),
            })
        }
    }
    .boxed()
}

async fn execute_select_subquery_async(
    query: &crate::query::SelectQuery,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: &QueryContext,
    options: ExecutionOptions,
    row_limit: Option<usize>,
) -> CoreResult<Vec<Object>> {
    let collection_id = query
        .collection
        .as_ref()
        .filter(|name| !crate::is_all_collection_alias(name))
        .and_then(|name| context.catalog().collection_by_name(name))
        .map(|collection| collection.lid);
    let source_ref = source_ref_for_collection(
        query.collection.clone(),
        query.source_alias.clone(),
        collection_id,
    );
    let plan = Optimizer::core().optimize_query_with_source(query, source_ref, None, context);
    let physical = match row_limit {
        Some(limit) => PhysicalPlan::Limit {
            input: Box::new(plan.physical),
            offset: Expr::from(0usize),
            limit: Some(Expr::from(limit)),
        },
        None => plan.physical,
    };
    Ok(collect_dyn_stream(execute_physical_dyn_stream(
        physical,
        source,
        context.clone(),
        options,
    ))
    .await?
    .into_iter()
    .map(|row| row.to_object())
    .collect())
}

async fn execute_scalar_subquery_async(
    query: &crate::query::SelectQuery,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: &QueryContext,
    options: ExecutionOptions,
) -> CoreResult<Value> {
    let shape = select_subquery_single_column_shape(query)?;
    let mut rows = execute_select_subquery_async(query, source, context, options, Some(2)).await?;
    if rows.len() > 1 {
        return Err(CoreError::new(format!(
            "scalar subquery returned {} rows; expected at most one",
            rows.len()
        )));
    }
    let Some(row) = rows.pop() else {
        return Ok(Value::Null);
    };
    single_column_subquery_value(row, shape)
}

async fn execute_list_subquery_async(
    query: &crate::query::SelectQuery,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: &QueryContext,
    options: ExecutionOptions,
) -> CoreResult<Vec<Value>> {
    let shape = select_subquery_single_column_shape(query)?;
    execute_select_subquery_async(query, source, context, options, None)
        .await?
        .into_iter()
        .map(|row| single_column_subquery_value(row, shape))
        .collect()
}

#[derive(Clone, Copy)]
enum SingleColumnShape {
    Exact,
    Dynamic,
}

fn select_subquery_single_column_shape(
    query: &crate::query::SelectQuery,
) -> CoreResult<SingleColumnShape> {
    if query.projection.is_empty() {
        return Ok(SingleColumnShape::Dynamic);
    }
    reject_duplicate_projection_aliases(
        query
            .projection
            .iter()
            .filter_map(|field| field.alias.as_deref()),
    )?;
    if query.projection.len() != 1 {
        return Err(CoreError::new(format!(
            "subquery projects {} columns; expected exactly one",
            query.projection.len()
        )));
    }
    if query.projection[0].wildcard.is_some() {
        return Err(CoreError::new(
            "wildcard subquery projection cannot be proven to contain exactly one column",
        ));
    }
    Ok(SingleColumnShape::Exact)
}

fn physical_subquery_single_column_shape(plan: &PhysicalPlan) -> CoreResult<SingleColumnShape> {
    match plan {
        PhysicalPlan::Project { projection, .. } | PhysicalPlan::Aggregate { projection, .. } => {
            reject_duplicate_projection_aliases(
                projection.iter().filter_map(|field| field.alias.as_deref()),
            )?;
            if projection.len() != 1 {
                return Err(CoreError::new(format!(
                    "subquery projects {} columns; expected exactly one",
                    projection.len()
                )));
            }
            if projection[0].wildcard.is_some() {
                return Err(CoreError::new(
                    "wildcard subquery projection cannot be proven to contain exactly one column",
                ));
            }
            Ok(SingleColumnShape::Exact)
        }
        PhysicalPlan::Sort { input, .. }
        | PhysicalPlan::Filter { input, .. }
        | PhysicalPlan::Limit { input, .. }
        | PhysicalPlan::Distinct { input }
        | PhysicalPlan::ApplyExists { input, .. }
        | PhysicalPlan::ApplyInSubquery { input, .. }
        | PhysicalPlan::Exchange { input, .. }
        | PhysicalPlan::RepartitionHash { input, .. }
        | PhysicalPlan::Materialize { input } => physical_subquery_single_column_shape(input),
        PhysicalPlan::Union { inputs, .. } => {
            let mut shape = SingleColumnShape::Exact;
            for input in inputs {
                if matches!(
                    physical_subquery_single_column_shape(input)?,
                    SingleColumnShape::Dynamic
                ) {
                    shape = SingleColumnShape::Dynamic;
                }
            }
            Ok(shape)
        }
        PhysicalPlan::Source(_) | PhysicalPlan::Values { .. } | PhysicalPlan::Join(_) => {
            Ok(SingleColumnShape::Dynamic)
        }
    }
}

fn reject_duplicate_projection_aliases<'a>(
    aliases: impl IntoIterator<Item = &'a str>,
) -> CoreResult<()> {
    let mut seen = BTreeSet::new();
    for alias in aliases {
        if !seen.insert(alias) {
            return Err(CoreError::new(format!(
                "subquery projection contains duplicate alias '{alias}'"
            )));
        }
    }
    Ok(())
}

fn single_column_subquery_value(row: Object, shape: SingleColumnShape) -> CoreResult<Value> {
    let fields = row.into_btree();
    if fields.is_empty() && matches!(shape, SingleColumnShape::Exact) {
        return Ok(Value::Null);
    }
    if fields.len() != 1 {
        return Err(CoreError::new(format!(
            "subquery returned {} columns; expected exactly one",
            fields.len()
        )));
    }
    Ok(fields
        .into_values()
        .next()
        .expect("single-column row must contain one value"))
}

fn execute_hash_join(
    join: &PhysicalJoinPlan,
    left_rows: Vec<DynObject>,
    right_rows: Vec<DynObject>,
) -> CoreResult<Vec<DynObject>> {
    let PhysicalJoinCondition::Eq { .. } = &join.condition else {
        return execute_nested_loop_join(join, left_rows, right_rows);
    };

    // Building the smaller side is semantics-preserving for inner joins. Outer joins keep the
    // existing right-build path so unmatched-row handling and ordering stay unchanged.
    if matches!(join.join_type, JoinType::Inner) && left_rows.len() < right_rows.len() {
        return execute_hash_join_build_left(join, left_rows, right_rows);
    }

    execute_hash_join_build_right(join, left_rows, right_rows)
}

fn execute_hash_join_build_right(
    join: &PhysicalJoinPlan,
    left_rows: Vec<DynObject>,
    right_rows: Vec<DynObject>,
) -> CoreResult<Vec<DynObject>> {
    let PhysicalJoinCondition::Eq { left, right } = &join.condition else {
        return execute_nested_loop_join(join, left_rows, right_rows);
    };

    let mut right_index: HashMap<ValueKey, Vec<(usize, Object)>> = HashMap::new();
    for (idx, right_row) in right_rows.iter().enumerate() {
        let Some(key) = value_ref_for_join_key(right_row.as_ref(), right).map(ValueKey::from_ref)
        else {
            continue;
        };
        right_index
            .entry(key)
            .or_default()
            .push((idx, right_row.to_object()));
    }

    let mut right_matched = vec![false; right_rows.len()];
    let mut out = Vec::<DynObject>::new();

    for left_row in &left_rows {
        let Some(left_key) =
            value_ref_for_join_key(left_row.as_ref(), left).map(ValueKey::from_ref)
        else {
            if matches!(join.join_type, JoinType::Left | JoinType::Full) {
                out.push(Box::new(bind_join_result(
                    Some(left_row.as_ref()),
                    None,
                    &join.left_binding,
                    &join.right_binding,
                )) as DynObject);
            }
            continue;
        };

        if let Some(matches) = right_index.get(&left_key) {
            for (right_idx, right_obj) in matches {
                right_matched[*right_idx] = true;
                out.push(Box::new(bind_join_result_obj(
                    Some(left_row.as_ref()),
                    Some(right_obj),
                    &join.left_binding,
                    &join.right_binding,
                )) as DynObject);
            }
        } else if matches!(join.join_type, JoinType::Left | JoinType::Full) {
            out.push(Box::new(bind_join_result(
                Some(left_row.as_ref()),
                None,
                &join.left_binding,
                &join.right_binding,
            )) as DynObject);
        }
    }

    if matches!(join.join_type, JoinType::Right | JoinType::Full) {
        for (idx, right_row) in right_rows.iter().enumerate() {
            if right_matched[idx] {
                continue;
            }
            out.push(Box::new(bind_join_result(
                None,
                Some(right_row.as_ref()),
                &join.left_binding,
                &join.right_binding,
            )) as DynObject);
        }
    }

    Ok(out)
}

fn execute_hash_join_build_left(
    join: &PhysicalJoinPlan,
    left_rows: Vec<DynObject>,
    right_rows: Vec<DynObject>,
) -> CoreResult<Vec<DynObject>> {
    let PhysicalJoinCondition::Eq { left, right } = &join.condition else {
        return execute_nested_loop_join(join, left_rows, right_rows);
    };

    let mut left_index: HashMap<ValueKey, Vec<(usize, Object)>> = HashMap::new();
    for (idx, left_row) in left_rows.iter().enumerate() {
        let Some(key) = value_ref_for_join_key(left_row.as_ref(), left).map(ValueKey::from_ref)
        else {
            continue;
        };
        left_index
            .entry(key)
            .or_default()
            .push((idx, left_row.to_object()));
    }

    let mut out_by_left = (0..left_rows.len())
        .map(|_| Vec::<DynObject>::new())
        .collect::<Vec<_>>();
    for right_row in &right_rows {
        let Some(right_key) =
            value_ref_for_join_key(right_row.as_ref(), right).map(ValueKey::from_ref)
        else {
            continue;
        };

        if let Some(matches) = left_index.get(&right_key) {
            let right_obj = right_row.to_object();
            for (left_idx, left_obj) in matches {
                out_by_left[*left_idx].push(Box::new(bind_join_result_obj(
                    Some(left_obj),
                    Some(&right_obj),
                    &join.left_binding,
                    &join.right_binding,
                )) as DynObject);
            }
        }
    }

    Ok(out_by_left.into_iter().flatten().collect())
}

fn execute_nested_loop_join(
    join: &PhysicalJoinPlan,
    left_rows: Vec<DynObject>,
    right_rows: Vec<DynObject>,
) -> CoreResult<Vec<DynObject>> {
    let mut out = Vec::<DynObject>::new();
    let mut right_matched = vec![false; right_rows.len()];

    for left_row in &left_rows {
        let mut matched_any = false;
        for (right_idx, right_row) in right_rows.iter().enumerate() {
            if join_pair_matches(join, left_row.as_ref(), right_row.as_ref()) {
                matched_any = true;
                right_matched[right_idx] = true;
                out.push(Box::new(bind_join_result(
                    Some(left_row.as_ref()),
                    Some(right_row.as_ref()),
                    &join.left_binding,
                    &join.right_binding,
                )) as DynObject);
            }
        }

        if !matched_any && matches!(join.join_type, JoinType::Left | JoinType::Full) {
            out.push(Box::new(bind_join_result(
                Some(left_row.as_ref()),
                None,
                &join.left_binding,
                &join.right_binding,
            )) as DynObject);
        }
    }

    if matches!(join.join_type, JoinType::Right | JoinType::Full) {
        for (idx, right_row) in right_rows.iter().enumerate() {
            if right_matched[idx] {
                continue;
            }
            out.push(Box::new(bind_join_result(
                None,
                Some(right_row.as_ref()),
                &join.left_binding,
                &join.right_binding,
            )) as DynObject);
        }
    }

    Ok(out)
}

fn join_pair_matches(
    join: &PhysicalJoinPlan,
    left: &dyn QueryObjectAccess,
    right: &dyn QueryObjectAccess,
) -> bool {
    match &join.condition {
        PhysicalJoinCondition::True => true,
        PhysicalJoinCondition::Predicate(predicate) => {
            let merged = bind_join_result(
                Some(left),
                Some(right),
                &join.left_binding,
                &join.right_binding,
            );
            matches!(evaluate_join_predicate(&merged, predicate), SqlTruth::True)
        }
        PhysicalJoinCondition::Eq { left: l, right: r } => {
            let lv = value_ref_for_join_key(left, l);
            let rv = value_ref_for_join_key(right, r);
            match (lv, rv) {
                (Some(lv), Some(rv)) => lv.into_owned() == rv.into_owned(),
                _ => false,
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SqlTruth {
    True,
    False,
    Unknown,
}

fn evaluate_join_predicate(row: &dyn QueryObjectAccess, expr: &Expr) -> SqlTruth {
    match expr {
        Expr::Binary { op, left, right } if *op == semantic_data::query::BinaryOp::And => {
            match (
                evaluate_join_predicate(row, left),
                evaluate_join_predicate(row, right),
            ) {
                (SqlTruth::False, _) | (_, SqlTruth::False) => SqlTruth::False,
                (SqlTruth::True, SqlTruth::True) => SqlTruth::True,
                _ => SqlTruth::Unknown,
            }
        }
        Expr::Binary { op, left, right } if *op == semantic_data::query::BinaryOp::Or => {
            match (
                evaluate_join_predicate(row, left),
                evaluate_join_predicate(row, right),
            ) {
                (SqlTruth::True, _) | (_, SqlTruth::True) => SqlTruth::True,
                (SqlTruth::False, SqlTruth::False) => SqlTruth::False,
                _ => SqlTruth::Unknown,
            }
        }
        Expr::Binary { op, left, right }
            if matches!(
                op,
                semantic_data::query::BinaryOp::Eq
                    | semantic_data::query::BinaryOp::NotEq
                    | semantic_data::query::BinaryOp::Lt
                    | semantic_data::query::BinaryOp::Lte
                    | semantic_data::query::BinaryOp::Gt
                    | semantic_data::query::BinaryOp::Gte
            ) =>
        {
            let Some(left) = evaluate_expr(row, left) else {
                return SqlTruth::Unknown;
            };
            let Some(right) = evaluate_expr(row, right) else {
                return SqlTruth::Unknown;
            };
            if left.is_nullish() || right.is_nullish() {
                return SqlTruth::Unknown;
            }
            match evaluate_expr(row, expr) {
                Some(Value::Bool(true)) => SqlTruth::True,
                Some(Value::Bool(false)) => SqlTruth::False,
                _ => SqlTruth::Unknown,
            }
        }
        Expr::Unary {
            op: semantic_data::query::UnaryOp::Not,
            expr,
        } => match evaluate_join_predicate(row, expr) {
            SqlTruth::True => SqlTruth::False,
            SqlTruth::False => SqlTruth::True,
            SqlTruth::Unknown => SqlTruth::Unknown,
        },
        _ => {
            if evaluate_filter_expr(row, expr) {
                SqlTruth::True
            } else {
                SqlTruth::False
            }
        }
    }
}

fn bind_join_result(
    left: Option<&dyn QueryObjectAccess>,
    right: Option<&dyn QueryObjectAccess>,
    left_binding: &str,
    right_binding: &str,
) -> Object {
    let mut out = match left {
        Some(left) => left.to_object(),
        None => Object::new(),
    };
    if !out.contains_key(left_binding) {
        match left {
            Some(left) => {
                out.insert(left_binding.to_string(), Value::Object(left.to_object()));
            }
            None => {
                out.insert(left_binding.to_string(), Value::Null);
            }
        }
    }
    match right {
        Some(right) => {
            out.insert(right_binding.to_string(), Value::Object(right.to_object()));
        }
        None => {
            out.insert(right_binding.to_string(), Value::Null);
        }
    }
    out
}

fn bind_join_result_obj(
    left: Option<&dyn QueryObjectAccess>,
    right: Option<&Object>,
    left_binding: &str,
    right_binding: &str,
) -> Object {
    let mut out = match left {
        Some(left) => left.to_object(),
        None => Object::new(),
    };
    if !out.contains_key(left_binding) {
        match left {
            Some(left) => out.insert(left_binding.to_string(), Value::Object(left.to_object())),
            None => out.insert(left_binding.to_string(), Value::Null),
        };
    }
    match right {
        Some(right) => out.insert(right_binding.to_string(), Value::Object(right.clone())),
        None => out.insert(right_binding.to_string(), Value::Null),
    };
    out
}

fn compare_dyn_objects(
    a: &dyn QueryObjectAccess,
    b: &dyn QueryObjectAccess,
    order_by: &[PhysicalOrderField],
) -> Ordering {
    let legacy: Vec<_> = order_by
        .iter()
        .map(|item| crate::query::OrderBy {
            expr: item.expr.clone(),
            direction: item.direction,
        })
        .collect();
    compare_objects_for_plan(a, b, &legacy)
}

fn project_dyn_object(
    value: &dyn QueryObjectAccess,
    projection: &[PhysicalProjectionField],
) -> Object {
    let mut out = Object::new();
    for project in projection {
        if let Some(path) = &project.wildcard {
            if path.segments().is_empty() {
                out.extend(value.to_object());
                continue;
            }
            let Some(Value::Object(object)) = value.value_at_path_ref(path).map(|v| v.into_owned())
            else {
                continue;
            };
            out.extend(object);
            continue;
        }
        let projected = if let Some(field) = &project.field {
            value_ref_for_field(value, field, project.source_path.as_ref()).map(|v| v.into_owned())
        } else {
            evaluate_expr(value, &project.expr)
        };
        let Some(v) = projected else {
            continue;
        };
        let key = project.alias.clone().unwrap_or_else(|| {
            project
                .source_path
                .as_ref()
                .map(infer_project_key)
                .unwrap_or_else(|| infer_expr_key(&project.expr))
        });
        out.insert(key, v);
    }
    out
}

fn infer_project_key(path: &FieldPath) -> String {
    for segment in path.segments().iter().rev() {
        if let semantic_data::value::PathSegment::Field(name) = segment {
            return name.clone();
        }
    }
    "value".to_string()
}

fn value_ref_for_join_key<'a>(
    row: &'a dyn QueryObjectAccess,
    key: &PhysicalJoinKey,
) -> Option<ValueRef<'a>> {
    let value = value_ref_for_field(row, &key.field, Some(&key.source_path))?;
    let is_nullish = match &value {
        ValueRef::Owned(value) => value.is_nullish(),
        ValueRef::Ref(value) => value.is_nullish(),
        ValueRef::Void | ValueRef::Null => true,
        _ => false,
    };
    (!is_nullish).then_some(value)
}

fn execute_aggregate(
    input_rows: Vec<DynObject>,
    group_by: &[crate::query::Expr],
    projection: &[PhysicalProjectionField],
    having: &Option<Expr>,
) -> CoreResult<Vec<DynObject>> {
    let owned_rows = input_rows
        .into_iter()
        .map(|row| row.to_object())
        .collect::<Vec<_>>();
    let mut groups = std::collections::BTreeMap::<Vec<Value>, Vec<Object>>::new();
    if group_by.is_empty() {
        groups.insert(Vec::new(), owned_rows);
    } else {
        for row in owned_rows {
            let key = group_by
                .iter()
                .map(|expr| evaluate_expr(&row, expr).unwrap_or(Value::Null))
                .collect::<Vec<_>>();
            groups.entry(key).or_default().push(row);
        }
    }

    let mut out = Vec::<DynObject>::new();
    for (_key, rows) in groups {
        if let Some(having) = having
            && !evaluate_group_predicate(&rows, having)
        {
            continue;
        }
        let mut projected = Object::new();
        for field in projection {
            let value = evaluate_group_expr(&rows, &field.expr);
            let Some(value) = value else {
                continue;
            };
            let key = field.alias.clone().unwrap_or_else(|| {
                field
                    .source_path
                    .as_ref()
                    .map(infer_project_key)
                    .unwrap_or_else(|| infer_expr_key(&field.expr))
            });
            projected.insert(key, value);
        }
        out.push(Box::new(projected) as DynObject);
    }
    Ok(out)
}

fn evaluate_group_predicate(rows: &[Object], predicate: &Expr) -> bool {
    evaluate_group_expr(rows, predicate)
        .as_ref()
        .is_some_and(|value| match value {
            Value::Bool(v) => *v,
            Value::Null | Value::Void => false,
            _ => true,
        })
}

fn evaluate_group_expr(rows: &[Object], expr: &Expr) -> Option<Value> {
    match expr {
        Expr::Aggregate { op, distinct, arg } => evaluate_aggregate_expr(rows, *op, *distinct, arg),
        Expr::Unary { op, expr } => {
            let value = evaluate_group_expr(rows, expr)?;
            if value.is_nullish() {
                return Some(Value::Null);
            }
            let empty = Object::new();
            let one = rows.first().unwrap_or(&empty);
            evaluate_expr(
                one,
                &Expr::Unary {
                    op: *op,
                    expr: Box::new(Expr::Operand(Operand::Literal(value))),
                },
            )
        }
        Expr::Binary { op, left, right } => {
            let left = evaluate_group_expr(rows, left)?;
            let right = evaluate_group_expr(rows, right)?;
            if left.is_nullish() || right.is_nullish() {
                return match op {
                    semantic_data::query::BinaryOp::And
                        if matches!(left, Value::Bool(false))
                            || matches!(right, Value::Bool(false)) =>
                    {
                        Some(Value::Bool(false))
                    }
                    semantic_data::query::BinaryOp::Or
                        if matches!(left, Value::Bool(true))
                            || matches!(right, Value::Bool(true)) =>
                    {
                        Some(Value::Bool(true))
                    }
                    _ => Some(Value::Null),
                };
            }
            let empty = Object::new();
            let one = rows.first().unwrap_or(&empty);
            evaluate_expr(
                one,
                &Expr::Binary {
                    op: *op,
                    left: Box::new(Expr::Operand(Operand::Literal(left))),
                    right: Box::new(Expr::Operand(Operand::Literal(right))),
                },
            )
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            let condition = evaluate_group_expr(rows, cond)?;
            if matches!(condition, Value::Bool(true)) {
                evaluate_group_expr(rows, then_expr)
            } else {
                evaluate_group_expr(rows, else_expr)
            }
        }
        Expr::Coalesce(items) => {
            for item in items {
                let Some(value) = evaluate_group_expr(rows, item) else {
                    continue;
                };
                if !value.is_nullish() {
                    return Some(value);
                }
            }
            Some(Value::Null)
        }
        Expr::Function { name, args } => {
            let mut rewritten = Vec::with_capacity(args.len());
            let mut contains_null = false;
            for arg in args {
                rewritten.push(match arg {
                    FunctionArg::Expr(expr) => {
                        let value = evaluate_group_expr(rows, expr)?;
                        contains_null |= value.is_nullish();
                        FunctionArg::Expr(Expr::Operand(Operand::Literal(value)))
                    }
                    FunctionArg::Wildcard => FunctionArg::Wildcard,
                });
            }
            let empty = Object::new();
            let one = rows.first().unwrap_or(&empty);
            evaluate_expr(
                one,
                &Expr::Function {
                    name: name.clone(),
                    args: rewritten,
                },
            )
            .or_else(|| contains_null.then_some(Value::Null))
        }
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let target = evaluate_group_expr(rows, expr)?;
            if list.is_empty() {
                return Some(Value::Bool(*negated));
            }
            if target.is_nullish() {
                return Some(Value::Null);
            }
            let mut contains_null = false;
            for item in list {
                let Some(candidate) = evaluate_group_expr(rows, item) else {
                    contains_null = true;
                    continue;
                };
                if candidate.is_nullish() {
                    contains_null = true;
                } else if candidate == target {
                    return Some(Value::Bool(!*negated));
                }
            }
            if contains_null {
                Some(Value::Null)
            } else {
                Some(Value::Bool(*negated))
            }
        }
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let value = evaluate_group_expr(rows, expr)?;
            let low = evaluate_group_expr(rows, low)?;
            let high = evaluate_group_expr(rows, high)?;
            if value.is_nullish() || low.is_nullish() || high.is_nullish() {
                return Some(Value::Null);
            }
            Some(Value::Bool(if *negated {
                value < low || value > high
            } else {
                value >= low && value <= high
            }))
        }
        Expr::PatternMatch {
            kind,
            expr,
            pattern,
            case_insensitive,
            negated,
        } => {
            let value = evaluate_group_expr(rows, expr)?;
            let pattern = evaluate_group_expr(rows, pattern)?;
            if value.is_nullish() || pattern.is_nullish() {
                return Some(Value::Null);
            }
            evaluate_group_scalar_expr(
                rows,
                Expr::PatternMatch {
                    kind: *kind,
                    expr: Box::new(Expr::Operand(Operand::Literal(value))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(pattern))),
                    case_insensitive: *case_insensitive,
                    negated: *negated,
                },
            )
        }
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive,
            negated,
        } => {
            let value = evaluate_group_expr(rows, expr)?;
            let pattern = evaluate_group_expr(rows, pattern)?;
            if value.is_nullish() || pattern.is_nullish() {
                return Some(Value::Null);
            }
            evaluate_group_scalar_expr(
                rows,
                Expr::RegexMatch {
                    expr: Box::new(Expr::Operand(Operand::Literal(value))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(pattern))),
                    case_insensitive: *case_insensitive,
                    negated: *negated,
                },
            )
        }
        Expr::IsNull { expr, negated } => {
            let is_null = evaluate_group_expr(rows, expr).is_none_or(|value| value.is_nullish());
            Some(Value::Bool(if *negated { !is_null } else { is_null }))
        }
        Expr::Operand(Operand::Literal(value)) => Some(value.clone()),
        Expr::Operand(Operand::Field(_))
        | Expr::Subquery(_)
        | Expr::Exists { .. }
        | Expr::RelationExists { .. } => rows.first().and_then(|row| evaluate_expr(row, expr)),
    }
}

fn evaluate_group_scalar_expr(rows: &[Object], expr: Expr) -> Option<Value> {
    let empty = Object::new();
    evaluate_expr(rows.first().unwrap_or(&empty), &expr)
}

fn evaluate_aggregate_expr(
    rows: &[Object],
    op: AggregateOp,
    distinct: bool,
    arg: &FunctionArg,
) -> Option<Value> {
    match op {
        AggregateOp::Count => {
            if matches!(arg, FunctionArg::Wildcard) {
                return Some(Value::I64(rows.len() as i64));
            }
            let FunctionArg::Expr(expr) = arg else {
                return Some(Value::I64(0));
            };
            let mut seen = BTreeSet::new();
            let mut count = 0i64;
            for row in rows {
                let Some(value) = evaluate_expr(row, expr) else {
                    continue;
                };
                if value.is_nullish() {
                    continue;
                }
                if distinct && !seen.insert(value.clone()) {
                    continue;
                }
                count += 1;
            }
            Some(Value::I64(count))
        }
        AggregateOp::Sum => {
            let FunctionArg::Expr(expr) = arg else {
                return None;
            };
            let mut seen = BTreeSet::new();
            let mut sum = 0.0f64;
            let mut found = false;
            for row in rows {
                let Some(value) = evaluate_expr(row, expr) else {
                    continue;
                };
                if value.is_nullish() {
                    continue;
                }
                if distinct && !seen.insert(value.clone()) {
                    continue;
                }
                let number = value.as_f64()?;
                sum += number;
                found = true;
            }
            if found {
                Some(Value::F64(sum.into()))
            } else {
                Some(Value::Null)
            }
        }
        AggregateOp::Avg => {
            let FunctionArg::Expr(expr) = arg else {
                return None;
            };
            let mut seen = BTreeSet::new();
            let mut sum = 0.0f64;
            let mut count = 0usize;
            for row in rows {
                let Some(value) = evaluate_expr(row, expr) else {
                    continue;
                };
                if value.is_nullish() {
                    continue;
                }
                if distinct && !seen.insert(value.clone()) {
                    continue;
                }
                sum += value.as_f64()?;
                count += 1;
            }
            if count == 0 {
                Some(Value::Null)
            } else {
                Some(Value::F64((sum / count as f64).into()))
            }
        }
        AggregateOp::Min | AggregateOp::Max => {
            let FunctionArg::Expr(expr) = arg else {
                return None;
            };
            let mut seen = BTreeSet::new();
            let mut best: Option<Value> = None;
            for row in rows {
                let Some(value) = evaluate_expr(row, expr) else {
                    continue;
                };
                if value.is_nullish() {
                    continue;
                }
                if distinct && !seen.insert(value.clone()) {
                    continue;
                }
                match &best {
                    None => best = Some(value),
                    Some(current) => {
                        let replace = match op {
                            AggregateOp::Min => value < *current,
                            AggregateOp::Max => value > *current,
                            _ => false,
                        };
                        if replace {
                            best = Some(value);
                        }
                    }
                }
            }
            Some(best.unwrap_or(Value::Null))
        }
    }
}

fn infer_expr_key(expr: &Expr) -> String {
    match expr {
        Expr::Operand(Operand::Field(path)) => infer_project_key(path),
        _ => "value".to_string(),
    }
}

fn value_ref_for_field<'a>(
    row: &'a dyn QueryObjectAccess,
    field: &FieldRef,
    fallback_path: Option<&FieldPath>,
) -> Option<ValueRef<'a>> {
    match field {
        FieldRef::AttrId(attr_id) => row
            .value_at_attr_ref(*attr_id)
            .or_else(|| fallback_path.and_then(|path| row.value_at_path_ref(path))),
        FieldRef::FieldId(field_id) => row
            .value_at_field_ref(*field_id)
            .or_else(|| fallback_path.and_then(|path| row.value_at_path_ref(path))),
        FieldRef::CanonicalName(name) => {
            let path = FieldPath::from_fields([name.as_str()]);
            row.value_at_path_ref(&path)
                .or_else(|| value_ref_for_builtin_alias_path(row, &path))
                .or_else(|| fallback_path.and_then(|path| row.value_at_path_ref(path)))
        }
        FieldRef::Path(path) => row
            .value_at_path_ref(path)
            .or_else(|| value_ref_for_builtin_alias_path(row, path)),
    }
}

fn value_ref_for_builtin_alias_path<'a>(
    row: &'a dyn QueryObjectAccess,
    path: &FieldPath,
) -> Option<ValueRef<'a>> {
    let [PathSegment::Field(field)] = path.segments() else {
        return None;
    };
    for alias in builtin_field_aliases(field) {
        if alias == field {
            continue;
        }
        let alias_path = FieldPath::from_fields([*alias]);
        if let Some(value) = row.value_at_path_ref(&alias_path) {
            return Some(value);
        }
    }
    None
}

fn builtin_field_aliases(field: &str) -> &'static [&'static str] {
    match field {
        "id" | "semantic:id" | "semantic:catalog:id" => {
            &["id", "semantic:id", "semantic:catalog:id"]
        }
        _ => &[],
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ValueKey(Value);

impl ValueKey {
    fn from_ref(value: ValueRef<'_>) -> Self {
        Self(value.into_owned())
    }
}

/// Evaluate a computed attribute (data model `Expr`) against an object.
pub fn evaluate_computed_expr(
    obj: &semantic_data::value::Object,
    expr: &semantic_data::expr::Expr,
) -> CoreResult<semantic_data::value::Value> {
    use semantic_data::expr::{self, BinaryOperator};
    use semantic_data::value::Value;

    match expr {
        expr::Expr::Literal(lit) => Ok(lit.value.clone()),
        expr::Expr::Ref(expr::RefExpr::Identifier(name)) if name == "self" => Err(CoreError::new(
            "bare self reference not allowed in computed expression",
        )),
        expr::Expr::FieldAccess(fa) => {
            if !matches!(
                &fa.target,
                expr::Expr::Ref(expr::RefExpr::Identifier(name)) if name == "self"
            ) {
                return Err(CoreError::new(
                    "field access target must be 'self' in computed expression",
                ));
            }
            obj.get(&fa.field).cloned().ok_or_else(|| {
                CoreError::new(format!(
                    "unknown field '{}' in computed expression",
                    fa.field
                ))
            })
        }
        expr::Expr::Binary(bin) if bin.op == BinaryOperator::Concat => {
            let left = evaluate_computed_expr(obj, &bin.left)?;
            let right = evaluate_computed_expr(obj, &bin.right)?;
            let left_str = value_to_display_string(&left);
            let right_str = value_to_display_string(&right);
            Ok(Value::String(left_str + &right_str))
        }
        expr::Expr::Call(call) => match &call.callee {
            expr::Callee::Name(name) if name.as_slice() == ["stringify"] => {
                if call.args.len() != 1 {
                    return Err(CoreError::new("stringify expects exactly 1 argument"));
                }
                if let expr::CallArg::Positional(arg) = &call.args[0] {
                    let value = evaluate_computed_expr(obj, arg)?;
                    Ok(Value::String(value_to_display_string(&value)))
                } else {
                    Err(CoreError::new(
                        "named arguments not supported in computed expressions",
                    ))
                }
            }
            _ => Err(CoreError::new(
                "unsupported function in computed expression",
            )),
        },
        expr::Expr::Unary(unary) => {
            let operand = evaluate_computed_expr(obj, &unary.operand)?;
            match unary.op {
                expr::UnaryOperator::Not => match operand {
                    Value::Bool(b) => Ok(Value::Bool(!b)),
                    _ => Err(CoreError::new(
                        "NOT requires boolean operand in computed expression",
                    )),
                },
                expr::UnaryOperator::Minus => match operand {
                    Value::I64(v) => Ok(Value::I64(-v)),
                    Value::F64(v) => Ok(Value::F64((-v.into_inner()).into())),
                    _ => Err(CoreError::new(
                        "negation requires numeric operand in computed expression",
                    )),
                },
                _ => Err(CoreError::new(
                    "unsupported unary operator in computed expression",
                )),
            }
        }
        expr::Expr::Cast(cast) => evaluate_computed_expr(obj, &cast.expr),
        expr::Expr::If(if_expr) => {
            let cond = evaluate_computed_expr(obj, &if_expr.condition)?;
            if is_value_truthy(&cond) {
                evaluate_computed_expr(obj, &if_expr.then_expr)
            } else {
                evaluate_computed_expr(obj, &if_expr.else_expr)
            }
        }
        _ => Err(CoreError::new(format!(
            "unsupported expression in computed attribute: {:?}",
            std::mem::discriminant(expr)
        ))),
    }
}

fn is_value_truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Void => false,
        Value::Bool(b) => *b,
        _ => true,
    }
}

fn value_to_display_string(value: &Value) -> String {
    match value {
        Value::Null | Value::Void => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::I8(v) => v.to_string(),
        Value::I16(v) => v.to_string(),
        Value::I32(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::I128(v) => v.to_string(),
        Value::U8(v) => v.to_string(),
        Value::U16(v) => v.to_string(),
        Value::U32(v) => v.to_string(),
        Value::U64(v) => v.to_string(),
        Value::U128(v) => v.to_string(),
        Value::F32(v) => v.into_inner().to_string(),
        Value::F64(v) => v.into_inner().to_string(),
        Value::String(s) => s.clone(),
        Value::Bytes(b) => format!("{:02x?}", b),
        Value::Date(d) => format!("{:?}", d),
        Value::Time(t) => format!("{:?}", t),
        Value::DateTime(dt) => format!("{:?}", dt),
        Value::Duration(d) => format!("{:?}", d),
        Value::Uuid(u) => format!("{:?}", u),
        Value::IpAddr(ip) => ip.to_string(),
        Value::List(items) => {
            let strs: Vec<String> = items.iter().map(value_to_display_string).collect();
            format!("[{}]", strs.join(", "))
        }
        Value::Map(map) => {
            let strs: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{:?}: {}", k, value_to_display_string(v)))
                .collect();
            format!("{{{}}}", strs.join(", "))
        }
        Value::Object(o) => {
            let strs: Vec<String> = o
                .iter()
                .map(|(k, v)| format!("{}: {}", k, value_to_display_string(v)))
                .collect();
            format!("{{{}}}", strs.join(", "))
        }
        Value::Variant(v) => format!("<{}:{}>", v.variant, value_to_display_string(&v.value)),
    }
}

/// Inject computed attributes for a row based on its class type.
pub fn inject_computed_attributes(
    catalog: &crate::catalog::Catalog,
    obj: &mut semantic_data::value::Object,
) -> CoreResult<()> {
    use semantic_data::value::Value;

    let object_type = match obj
        .get(crate::catalog::OBJECT_TYPE_FIELD)
        .and_then(Value::as_str)
    {
        Some(ty) => ty,
        None => return Ok(()),
    };

    let class_lid = match catalog.class_id(object_type) {
        Some(lid) => lid,
        None => return Ok(()),
    };

    let class_schema = match catalog.class_by_lid(class_lid) {
        Some(c) => c,
        None => return Ok(()),
    };

    for (_attr_alias, attr) in &class_schema.class.attributes {
        if let Some(ref expr) = attr.computed {
            let canon = &attr.attribute.id;
            if !obj.contains_key(canon) {
                let value = evaluate_computed_expr(obj, expr)?;
                obj.insert(canon.clone(), value);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Expr, Operand, QueryField, SelectQuery};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn run_async<T>(future: impl std::future::Future<Output = T>) -> T {
        super::run_future_to_completion(future)
    }

    struct InlineSource {
        left: Vec<Object>,
        right: Vec<Object>,
    }

    impl AsyncPhysicalDataSource for InlineSource {
        fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
            let values = match source.source_name.as_deref() {
                Some("left") => &self.left,
                Some("right") => &self.right,
                _ => &self.left,
            };
            rows_to_batches(
                values
                    .iter()
                    .cloned()
                    .map(|item| Box::new(item) as DynObject)
                    .collect(),
                2,
            )
        }
    }

    struct AsyncInlineSource {
        rows: Vec<Object>,
        filtered_calls: AtomicUsize,
        index_calls: AtomicUsize,
    }

    impl AsyncInlineSource {
        fn new(rows: Vec<Object>) -> Self {
            Self {
                rows,
                filtered_calls: AtomicUsize::new(0),
                index_calls: AtomicUsize::new(0),
            }
        }
    }

    impl AsyncPhysicalDataSource for AsyncInlineSource {
        fn scan_stream(&self, _source: SourceRef) -> SendableRecordBatchStream {
            rows_to_batches(
                self.rows
                    .iter()
                    .cloned()
                    .map(|row| Box::new(row) as DynObject)
                    .collect(),
                2,
            )
        }

        fn scan_filtered_stream(
            &self,
            source: SourceRef,
            predicate: Expr,
        ) -> SendableRecordBatchStream {
            self.filtered_calls.fetch_add(1, AtomicOrdering::Relaxed);
            filter_batch_stream(self.scan_stream(source), predicate)
        }

        fn index_lookup_stream(
            &self,
            source: SourceRef,
            field: FieldRef,
            value: Value,
        ) -> SendableRecordBatchStream {
            self.index_calls.fetch_add(1, AtomicOrdering::Relaxed);
            <Self as AsyncPhysicalDataSource>::scan_filtered_stream(
                self,
                source,
                Expr::Binary {
                    op: semantic_data::query::BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(
                        field_path_for_ref(&field)
                            .unwrap_or_else(|| FieldPath::from_fields(["id"])),
                    ))),
                    right: Box::new(Expr::Operand(Operand::Literal(value))),
                },
            )
        }
    }

    struct EmptyBatchSource;

    impl AsyncPhysicalDataSource for EmptyBatchSource {
        fn scan_stream(&self, _source: SourceRef) -> SendableRecordBatchStream {
            stream::iter([
                Ok(Vec::new()),
                Ok(vec![Box::new(obj_i64("id", 1)) as DynObject]),
                Ok(Vec::new()),
                Ok(vec![Box::new(obj_i64("id", 2)) as DynObject]),
            ])
            .boxed()
        }
    }

    fn obj_i64(field: &str, value: i64) -> Object {
        let mut row = Object::new();
        row.insert(field, Value::I64(value));
        row
    }

    #[test]
    fn async_values_emit_configured_batch_sizes() {
        let values = (0..5).map(|idx| obj_i64("id", idx)).collect::<Vec<_>>();
        let batches = run_async(
            execute_physical_plan_stream(
                PhysicalPlan::Values { values },
                Arc::new(AsyncInlineSource::new(Vec::new())),
                QueryContext::default(),
                ExecutionOptions {
                    batch_size: 2,
                    ..ExecutionOptions::default()
                },
            )
            .try_collect::<Vec<_>>(),
        )
        .unwrap();

        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
    }

    #[test]
    fn async_stream_boundary_drops_empty_batches() {
        let batches = run_async(
            execute_physical_plan_stream(
                PhysicalPlan::Source(PhysicalSource::Scan {
                    source: SourceRef {
                        source_name: Some("items".to_string()),
                        collection_id: None,
                        binding: None,
                        backend_tag: None,
                    },
                }),
                Arc::new(EmptyBatchSource),
                QueryContext::default(),
                ExecutionOptions::default(),
            )
            .try_collect::<Vec<_>>(),
        )
        .unwrap();

        assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), vec![1, 1]);
    }

    #[test]
    fn async_filter_project_limit_streams_batches() {
        let rows = (0..6).map(|idx| obj_i64("id", idx)).collect::<Vec<_>>();
        let plan = PhysicalPlan::Limit {
            input: Box::new(PhysicalPlan::Project {
                input: Box::new(PhysicalPlan::Filter {
                    input: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                        source: SourceRef {
                            source_name: Some("items".to_string()),
                            collection_id: None,
                            binding: None,
                            backend_tag: None,
                        },
                    })),
                    predicate: Expr::Binary {
                        op: semantic_data::query::BinaryOp::Gt,
                        left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "id",
                        ])))),
                        right: Box::new(Expr::Operand(Operand::Literal(Value::I64(1)))),
                    },
                }),
                projection: vec![PhysicalProjectionField {
                    expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["id"]))),
                    field: Some(FieldRef::Path(FieldPath::from_fields(["id"]))),
                    source_path: Some(FieldPath::from_fields(["id"])),
                    alias: Some("out".to_string()),
                    wildcard: None,
                }],
            }),
            offset: Expr::from(1usize),
            limit: Some(Expr::from(2usize)),
        };

        let out = run_async(execute_physical_plan_collect(
            plan,
            Arc::new(AsyncInlineSource::new(rows)),
            QueryContext::default(),
            ExecutionOptions::default(),
        ))
        .unwrap();

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].get("out"), Some(&Value::I64(3)));
        assert_eq!(out[1].get("out"), Some(&Value::I64(4)));
    }

    #[test]
    fn physical_projection_flattens_current_row_wildcard() {
        let rows = vec![obj_i64("id", 1)];
        let plan = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some("items".to_string()),
                    collection_id: None,
                    binding: Some("i".to_string()),
                    backend_tag: None,
                },
            })),
            projection: vec![PhysicalProjectionField {
                expr: Expr::Operand(Operand::Field(FieldPath::from_fields(["i"]))),
                field: None,
                source_path: Some(FieldPath::from_fields(["i"])),
                alias: None,
                wildcard: Some(FieldPath::new()),
            }],
        };

        let out = run_async(execute_physical_plan_collect(
            plan,
            Arc::new(AsyncInlineSource::new(rows)),
            QueryContext::default(),
            ExecutionOptions::default(),
        ))
        .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("id"), Some(&Value::I64(1)));
        assert!(!out[0].contains_key("i"));
    }

    #[test]
    fn mixed_bare_wildcard_projection_preserves_ordered_map_overwrite_semantics() {
        let mut row = obj_i64("id", 1);
        row.insert("title", Value::String("original".to_string()));
        let plan = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Values { values: vec![row] }),
            projection: vec![
                PhysicalProjectionField {
                    expr: Expr::Operand(Operand::Literal(Value::Null)),
                    field: None,
                    source_path: None,
                    alias: None,
                    wildcard: Some(FieldPath::new()),
                },
                PhysicalProjectionField {
                    expr: Expr::Operand(Operand::Literal(Value::String("override".to_string()))),
                    field: None,
                    source_path: None,
                    alias: Some("title".to_string()),
                    wildcard: None,
                },
            ],
        };

        let out = run_async(execute_physical_plan_collect(
            plan,
            Arc::new(AsyncInlineSource::new(Vec::new())),
            QueryContext::default(),
            ExecutionOptions::default(),
        ))
        .unwrap();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("id"), Some(&Value::I64(1)));
        assert_eq!(
            out[0].get("title"),
            Some(&Value::String("override".to_string()))
        );
    }

    #[test]
    fn global_aggregate_expressions_evaluate_over_empty_input() {
        let count = || Expr::Aggregate {
            op: AggregateOp::Count,
            distinct: false,
            arg: Box::new(FunctionArg::Wildcard),
        };
        let sum = || Expr::Aggregate {
            op: AggregateOp::Sum,
            distinct: false,
            arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                FieldPath::from_fields(["score"]),
            )))),
        };
        let projection = vec![
            PhysicalProjectionField {
                expr: Expr::Binary {
                    op: semantic_data::query::BinaryOp::Add,
                    left: Box::new(count()),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(1)))),
                },
                field: None,
                source_path: None,
                alias: Some("adjusted".to_string()),
                wildcard: None,
            },
            PhysicalProjectionField {
                expr: Expr::Operand(Operand::Literal(Value::String("empty".to_string()))),
                field: None,
                source_path: None,
                alias: Some("label".to_string()),
                wildcard: None,
            },
            PhysicalProjectionField {
                expr: Expr::Coalesce(vec![sum(), Expr::Operand(Operand::Literal(Value::I64(0)))]),
                field: None,
                source_path: None,
                alias: Some("sum_or_zero".to_string()),
                wildcard: None,
            },
            PhysicalProjectionField {
                expr: Expr::Binary {
                    op: semantic_data::query::BinaryOp::Add,
                    left: Box::new(sum()),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(1)))),
                },
                field: None,
                source_path: None,
                alias: Some("nullable_sum".to_string()),
                wildcard: None,
            },
            PhysicalProjectionField {
                expr: Expr::IsNull {
                    expr: Box::new(sum()),
                    negated: false,
                },
                field: None,
                source_path: None,
                alias: Some("sum_is_null".to_string()),
                wildcard: None,
            },
        ];
        let having = Some(Expr::Binary {
            op: semantic_data::query::BinaryOp::Eq,
            left: Box::new(count()),
            right: Box::new(Expr::Operand(Operand::Literal(Value::I64(0)))),
        });

        let rows = execute_aggregate(Vec::new(), &[], &projection, &having).unwrap();
        assert_eq!(rows.len(), 1);
        let row = rows[0].to_object();
        assert_eq!(row.get("adjusted"), Some(&Value::F64(1.0.into())));
        assert_eq!(row.get("label"), Some(&Value::String("empty".to_string())));
        assert_eq!(row.get("sum_or_zero"), Some(&Value::I64(0)));
        assert_eq!(row.get("nullable_sum"), Some(&Value::Null));
        assert_eq!(row.get("sum_is_null"), Some(&Value::Bool(true)));
    }

    #[test]
    fn grouped_aggregate_does_not_invent_a_group_for_empty_input() {
        let projection = vec![PhysicalProjectionField {
            expr: Expr::Aggregate {
                op: AggregateOp::Count,
                distinct: false,
                arg: Box::new(FunctionArg::Wildcard),
            },
            field: None,
            source_path: None,
            alias: Some("count".to_string()),
            wildcard: None,
        }];

        let rows = execute_aggregate(
            Vec::new(),
            &[Expr::Operand(Operand::Field(FieldPath::from_fields([
                "kind",
            ])))],
            &projection,
            &None,
        )
        .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn aggregate_group_expressions_cover_range_membership_and_patterns() {
        let sum = || Expr::Aggregate {
            op: AggregateOp::Sum,
            distinct: false,
            arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                FieldPath::from_fields(["score"]),
            )))),
        };
        let count = || Expr::Aggregate {
            op: AggregateOp::Count,
            distinct: false,
            arg: Box::new(FunctionArg::Wildcard),
        };
        let max_name = || Expr::Aggregate {
            op: AggregateOp::Max,
            distinct: false,
            arg: Box::new(FunctionArg::Expr(Expr::Operand(Operand::Field(
                FieldPath::from_fields(["name"]),
            )))),
        };
        let between = |negated| Expr::Between {
            expr: Box::new(sum()),
            low: Box::new(Expr::Operand(Operand::Literal(Value::F64(4.0.into())))),
            high: Box::new(Expr::Operand(Operand::Literal(Value::F64(6.0.into())))),
            negated,
        };
        let in_with_null = |negated| Expr::InList {
            expr: Box::new(count()),
            list: vec![
                Expr::Operand(Operand::Literal(Value::I64(2))),
                Expr::Operand(Operand::Literal(Value::Null)),
            ],
            negated,
        };
        let in_empty = |negated| Expr::InList {
            expr: Box::new(count()),
            list: Vec::new(),
            negated,
        };
        let like = |negated| Expr::PatternMatch {
            kind: semantic_data::query::PatternMatchKind::Like,
            expr: Box::new(max_name()),
            pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                "b%".to_string(),
            )))),
            case_insensitive: false,
            negated,
        };
        let regex = |negated| Expr::RegexMatch {
            expr: Box::new(max_name()),
            pattern: Box::new(Expr::Operand(Operand::Literal(Value::String(
                "^b".to_string(),
            )))),
            case_insensitive: false,
            negated,
        };
        let field = |alias: &str, expr| PhysicalProjectionField {
            expr,
            field: None,
            source_path: None,
            alias: Some(alias.to_string()),
            wildcard: None,
        };
        let projection = vec![
            field("between", between(false)),
            field("not_between", between(true)),
            field("in", in_with_null(false)),
            field("not_in", in_with_null(true)),
            field("in_empty", in_empty(false)),
            field("not_in_empty", in_empty(true)),
            field("like", like(false)),
            field("not_like", like(true)),
            field("regex", regex(false)),
            field("not_regex", regex(true)),
        ];
        let mut first = Object::new();
        first.insert("score", Value::I64(2));
        first.insert("name", Value::String("alpha".to_string()));
        let mut second = Object::new();
        second.insert("score", Value::I64(3));
        second.insert("name", Value::String("beta".to_string()));

        let nonempty = execute_aggregate(
            vec![Box::new(first), Box::new(second)],
            &[],
            &projection,
            &Some(between(false)),
        )
        .unwrap();
        assert_eq!(nonempty.len(), 1);
        let row = nonempty[0].to_object();
        for key in ["between", "in", "like", "regex", "not_in_empty"] {
            assert_eq!(row.get(key), Some(&Value::Bool(true)), "{key}");
        }
        for key in ["not_between", "not_in", "in_empty", "not_like", "not_regex"] {
            assert_eq!(row.get(key), Some(&Value::Bool(false)), "{key}");
        }

        let empty = execute_aggregate(Vec::new(), &[], &projection, &None).unwrap();
        assert_eq!(empty.len(), 1);
        let row = empty[0].to_object();
        for key in [
            "between",
            "not_between",
            "in",
            "not_in",
            "like",
            "not_like",
            "regex",
            "not_regex",
        ] {
            assert_eq!(row.get(key), Some(&Value::Null), "{key}");
        }
        assert_eq!(row.get("in_empty"), Some(&Value::Bool(false)));
        assert_eq!(row.get("not_in_empty"), Some(&Value::Bool(true)));

        let filtered_empty =
            execute_aggregate(Vec::new(), &[], &projection, &Some(between(false))).unwrap();
        assert!(filtered_empty.is_empty());
    }

    #[test]
    fn async_filtered_scan_and_index_lookup_use_source_hooks() {
        let source = Arc::new(AsyncInlineSource::new(vec![
            obj_i64("id", 1),
            obj_i64("id", 2),
        ]));
        let scan_ref = SourceRef {
            source_name: Some("items".to_string()),
            collection_id: None,
            binding: None,
            backend_tag: None,
        };

        let filtered = run_async(execute_physical_plan_collect(
            PhysicalPlan::Source(PhysicalSource::FilteredScan {
                source: scan_ref.clone(),
                predicate: Expr::Binary {
                    op: semantic_data::query::BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "id",
                    ])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(2)))),
                },
            }),
            source.clone(),
            QueryContext::default(),
            ExecutionOptions::default(),
        ))
        .unwrap();
        let indexed = run_async(execute_physical_plan_collect(
            PhysicalPlan::Source(PhysicalSource::IndexLookup {
                source: scan_ref,
                field: FieldRef::Path(FieldPath::from_fields(["id"])),
                value: Value::I64(1),
                residual_predicate: None,
            }),
            source.clone(),
            QueryContext::default(),
            ExecutionOptions::default(),
        ))
        .unwrap();

        assert_eq!(filtered.len(), 1);
        assert_eq!(indexed.len(), 1);
        assert_eq!(source.filtered_calls.load(AtomicOrdering::Relaxed), 2);
        assert_eq!(source.index_calls.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn executes_hash_join() {
        let mut l1 = Object::new();
        l1.insert("id", Value::I64(1));
        let mut l2 = Object::new();
        l2.insert("id", Value::I64(2));
        let mut lnull = Object::new();
        lnull.insert("id", Value::Null);
        let lmissing = Object::new();
        let mut r1 = Object::new();
        r1.insert("rid", Value::I64(2));
        let mut rnull = Object::new();
        rnull.insert("rid", Value::Null);
        let rmissing = Object::new();

        let plan = PhysicalPlan::Join(PhysicalJoinPlan {
            left: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some("left".to_string()),
                    collection_id: None,
                    binding: None,
                    backend_tag: None,
                },
            })),
            right: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some("right".to_string()),
                    collection_id: None,
                    binding: None,
                    backend_tag: None,
                },
            })),
            join_type: JoinType::Inner,
            algorithm: PhysicalJoinAlgorithm::Hash,
            condition: PhysicalJoinCondition::Eq {
                left: PhysicalJoinKey {
                    field: FieldRef::Path(FieldPath::from_fields(["id"])),
                    source_path: FieldPath::from_fields(["id"]),
                },
                right: PhysicalJoinKey {
                    field: FieldRef::Path(FieldPath::from_fields(["rid"])),
                    source_path: FieldPath::from_fields(["rid"]),
                },
            },
            left_binding: "l".to_string(),
            right_binding: "r".to_string(),
        });

        let out = execute_physical_plan_with_source(
            &plan,
            &InlineSource {
                left: vec![l1, l2, lnull, lmissing],
                right: vec![r1, rnull, rmissing],
            },
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].contains_key("l"));
        assert!(out[0].contains_key("r"));
    }

    #[test]
    fn residual_equality_join_matches_hash_null_semantics_for_all_join_types() {
        let source_ref = |name: &str| {
            PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some(name.to_string()),
                    collection_id: None,
                    binding: None,
                    backend_tag: None,
                },
            })
        };
        let plan = |join_type, residual| {
            let equality = Expr::Binary {
                op: semantic_data::query::BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "l", "id",
                ])))),
                right: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "r", "id",
                ])))),
            };
            PhysicalPlan::Join(PhysicalJoinPlan {
                left: Box::new(source_ref("left")),
                right: Box::new(source_ref("right")),
                join_type,
                algorithm: if residual {
                    PhysicalJoinAlgorithm::NestedLoop
                } else {
                    PhysicalJoinAlgorithm::Hash
                },
                condition: if residual {
                    PhysicalJoinCondition::Predicate(Expr::Binary {
                        op: semantic_data::query::BinaryOp::And,
                        left: Box::new(equality),
                        right: Box::new(Expr::Operand(Operand::Literal(Value::Bool(true)))),
                    })
                } else {
                    PhysicalJoinCondition::Eq {
                        left: PhysicalJoinKey {
                            field: FieldRef::Path(FieldPath::from_fields(["id"])),
                            source_path: FieldPath::from_fields(["id"]),
                        },
                        right: PhysicalJoinKey {
                            field: FieldRef::Path(FieldPath::from_fields(["id"])),
                            source_path: FieldPath::from_fields(["id"]),
                        },
                    }
                },
                left_binding: "l".to_string(),
                right_binding: "r".to_string(),
            })
        };
        let rows = || {
            let mut null = Object::new();
            null.insert("id", Value::Null);
            vec![obj_i64("id", 1), null, Object::new()]
        };

        for join_type in [
            JoinType::Inner,
            JoinType::Left,
            JoinType::Right,
            JoinType::Full,
        ] {
            let hash = execute_physical_plan_with_source(
                &plan(join_type, false),
                &InlineSource {
                    left: rows(),
                    right: rows(),
                },
                &QueryContext::default(),
            )
            .unwrap();
            let residual = execute_physical_plan_with_source(
                &plan(join_type, true),
                &InlineSource {
                    left: rows(),
                    right: rows(),
                },
                &QueryContext::default(),
            )
            .unwrap();
            assert_eq!(residual, hash, "join type {join_type:?}");
        }
    }

    #[test]
    fn executes_exists_subquery_apply() {
        let mut l = Object::new();
        l.insert("v", Value::I64(1));
        let mut s = Object::new();
        s.insert("x", Value::I64(1));
        let plan = PhysicalPlan::ApplyExists {
            input: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some("left".to_string()),
                    collection_id: None,
                    binding: None,
                    backend_tag: None,
                },
            })),
            subquery: Box::new(PhysicalPlan::Source(PhysicalSource::FilteredScan {
                source: SourceRef {
                    source_name: Some("right".to_string()),
                    collection_id: None,
                    binding: None,
                    backend_tag: None,
                },
                predicate: Expr::Binary {
                    op: semantic_data::query::BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["x"])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::I64(1)))),
                },
            })),
            negated: false,
        };

        let out = execute_physical_plan_with_source(
            &plan,
            &InlineSource {
                left: vec![l],
                right: vec![s],
            },
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn executes_scalar_subquery_in_projection() {
        let mut l = Object::new();
        l.insert("id", Value::I64(1));
        let mut s = Object::new();
        s.insert("x", Value::I64(7));

        let plan = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some("left".to_string()),
                    collection_id: None,
                    binding: None,
                    backend_tag: None,
                },
            })),
            projection: vec![PhysicalProjectionField {
                expr: Expr::Subquery(Box::new(
                    SelectQuery::new()
                        .with_collection("right")
                        .with_projection(vec![QueryField {
                            expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(
                                ["x"],
                            )))),
                            alias: None,
                            wildcard: None,
                        }]),
                )),
                field: None,
                source_path: None,
                alias: Some("sv".to_string()),
                wildcard: None,
            }],
        };

        let out = execute_physical_plan_with_source(
            &plan,
            &InlineSource {
                left: vec![l],
                right: vec![s],
            },
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("sv"), Some(&Value::I64(7)));
    }

    #[test]
    fn executes_binary_in_with_subquery_in_filter() {
        let mut a = Object::new();
        a.insert("v", Value::I64(1));
        let mut b = Object::new();
        b.insert("v", Value::I64(3));
        let mut s1 = Object::new();
        s1.insert("x", Value::I64(1));
        let mut s2 = Object::new();
        s2.insert("x", Value::I64(2));

        let plan = PhysicalPlan::Filter {
            input: Box::new(PhysicalPlan::Source(PhysicalSource::Scan {
                source: SourceRef {
                    source_name: Some("left".to_string()),
                    collection_id: None,
                    binding: None,
                    backend_tag: None,
                },
            })),
            predicate: Expr::Binary {
                op: semantic_data::query::BinaryOp::In,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["v"])))),
                right: Box::new(Expr::Subquery(Box::new(
                    SelectQuery::new()
                        .with_collection("right")
                        .with_projection(vec![QueryField {
                            expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(
                                ["x"],
                            )))),
                            alias: None,
                            wildcard: None,
                        }]),
                ))),
            },
        };

        let out = execute_physical_plan_with_source(
            &plan,
            &InlineSource {
                left: vec![a, b],
                right: vec![s1, s2],
            },
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].get("v"), Some(&Value::I64(1)));
    }

    #[test]
    fn scalar_subquery_enforces_row_cardinality_and_empty_is_null() {
        let scalar_plan = || PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("id", 1)],
            }),
            projection: vec![PhysicalProjectionField {
                expr: Expr::Subquery(Box::new(
                    SelectQuery::new()
                        .with_collection("right")
                        .with_projection(vec![QueryField {
                            expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(
                                ["x"],
                            )))),
                            alias: None,
                            wildcard: None,
                        }]),
                )),
                field: None,
                source_path: None,
                alias: Some("scalar".to_string()),
                wildcard: None,
            }],
        };

        let empty = execute_physical_plan_with_source(
            &scalar_plan(),
            &InlineSource {
                left: Vec::new(),
                right: Vec::new(),
            },
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(empty[0].get("scalar"), Some(&Value::Null));

        let err = execute_physical_plan_with_source(
            &scalar_plan(),
            &InlineSource {
                left: Vec::new(),
                right: vec![obj_i64("x", 1), obj_i64("x", 2)],
            },
            &QueryContext::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("expected at most one"));
    }

    #[test]
    fn scalar_and_in_subqueries_require_exactly_one_column() {
        let mut right = Object::new();
        right.insert("x", Value::I64(1));
        right.insert("y", Value::I64(2));
        let two_columns = || {
            SelectQuery::new()
                .with_collection("right")
                .with_projection(vec![
                    QueryField {
                        expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "x",
                        ])))),
                        alias: None,
                        wildcard: None,
                    },
                    QueryField {
                        expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "y",
                        ])))),
                        alias: None,
                        wildcard: None,
                    },
                ])
        };
        let source = InlineSource {
            left: Vec::new(),
            right: vec![right],
        };

        let scalar = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("id", 1)],
            }),
            projection: vec![PhysicalProjectionField {
                expr: Expr::Subquery(Box::new(two_columns())),
                field: None,
                source_path: None,
                alias: Some("scalar".to_string()),
                wildcard: None,
            }],
        };
        let scalar_err =
            execute_physical_plan_with_source(&scalar, &source, &QueryContext::default())
                .unwrap_err();
        assert!(scalar_err.to_string().contains("expected exactly one"));

        let in_filter = PhysicalPlan::Filter {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("v", 1)],
            }),
            predicate: Expr::Binary {
                op: semantic_data::query::BinaryOp::In,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["v"])))),
                right: Box::new(Expr::Subquery(Box::new(two_columns()))),
            },
        };
        let in_err =
            execute_physical_plan_with_source(&in_filter, &source, &QueryContext::default())
                .unwrap_err();
        assert!(in_err.to_string().contains("expected exactly one"));
    }

    #[test]
    fn missing_single_projected_field_is_null_and_duplicate_aliases_fail() {
        let missing_field_query = || {
            SelectQuery::new()
                .with_collection("right")
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "missing",
                    ])))),
                    alias: Some("value".to_string()),
                    wildcard: None,
                }])
        };
        let scalar = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("id", 1)],
            }),
            projection: vec![PhysicalProjectionField {
                expr: Expr::Subquery(Box::new(missing_field_query())),
                field: None,
                source_path: None,
                alias: Some("scalar".to_string()),
                wildcard: None,
            }],
        };
        let rows = execute_physical_plan_with_source(
            &scalar,
            &InlineSource {
                left: Vec::new(),
                right: vec![obj_i64("present", 1)],
            },
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(rows[0].get("scalar"), Some(&Value::Null));

        let duplicate_alias_query =
            SelectQuery::new()
                .with_collection("right")
                .with_projection(vec![
                    QueryField {
                        expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "x",
                        ])))),
                        alias: Some("duplicate".to_string()),
                        wildcard: None,
                    },
                    QueryField {
                        expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                            "y",
                        ])))),
                        alias: Some("duplicate".to_string()),
                        wildcard: None,
                    },
                ]);
        let duplicate_alias = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("id", 1)],
            }),
            projection: vec![PhysicalProjectionField {
                expr: Expr::Subquery(Box::new(duplicate_alias_query)),
                field: None,
                source_path: None,
                alias: Some("scalar".to_string()),
                wildcard: None,
            }],
        };
        let err = execute_physical_plan_with_source(
            &duplicate_alias,
            &InlineSource {
                left: Vec::new(),
                right: vec![obj_i64("x", 1)],
            },
            &QueryContext::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate alias 'duplicate'"));
    }

    #[test]
    fn in_subquery_preserves_null_semantics_in_expressions() {
        let query = || {
            SelectQuery::new()
                .with_collection("right")
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["x"])))),
                    alias: None,
                    wildcard: None,
                }])
        };
        let in_expr = |negated| {
            let expr = Expr::Binary {
                op: semantic_data::query::BinaryOp::In,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["v"])))),
                right: Box::new(Expr::Subquery(Box::new(query()))),
            };
            if negated {
                Expr::Unary {
                    op: semantic_data::query::UnaryOp::Not,
                    expr: Box::new(expr),
                }
            } else {
                expr
            }
        };
        let project = |expr| PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("v", 2)],
            }),
            projection: vec![PhysicalProjectionField {
                expr,
                field: None,
                source_path: None,
                alias: Some("result".to_string()),
                wildcard: None,
            }],
        };
        let mut null_row = Object::new();
        null_row.insert("x", Value::Null);
        let source = InlineSource {
            left: Vec::new(),
            right: vec![obj_i64("x", 1), null_row],
        };

        let in_rows = execute_physical_plan_with_source(
            &project(in_expr(false)),
            &source,
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(in_rows[0].get("result"), Some(&Value::Null));

        let not_in_rows = execute_physical_plan_with_source(
            &project(in_expr(true)),
            &source,
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(not_in_rows[0].get("result"), Some(&Value::Null));
    }

    #[test]
    fn empty_in_subquery_ignores_null_or_missing_left_operand() {
        let query = || {
            SelectQuery::new()
                .with_collection("right")
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["x"])))),
                    alias: None,
                    wildcard: None,
                }])
        };
        let in_expr = |negated| {
            let expr = Expr::Binary {
                op: semantic_data::query::BinaryOp::In,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["v"])))),
                right: Box::new(Expr::Subquery(Box::new(query()))),
            };
            if negated {
                Expr::Unary {
                    op: semantic_data::query::UnaryOp::Not,
                    expr: Box::new(expr),
                }
            } else {
                expr
            }
        };
        let mut null_left = Object::new();
        null_left.insert("v", Value::Null);
        let missing_left = Object::new();
        let source = InlineSource {
            left: Vec::new(),
            right: Vec::new(),
        };

        let projection = PhysicalPlan::Project {
            input: Box::new(PhysicalPlan::Values {
                values: vec![null_left.clone()],
            }),
            projection: vec![
                PhysicalProjectionField {
                    expr: in_expr(false),
                    field: None,
                    source_path: None,
                    alias: Some("in_result".to_string()),
                    wildcard: None,
                },
                PhysicalProjectionField {
                    expr: in_expr(true),
                    field: None,
                    source_path: None,
                    alias: Some("not_in_result".to_string()),
                    wildcard: None,
                },
            ],
        };
        let projected =
            execute_physical_plan_with_source(&projection, &source, &QueryContext::default())
                .unwrap();
        assert_eq!(projected[0].get("in_result"), Some(&Value::Bool(false)));
        assert_eq!(projected[0].get("not_in_result"), Some(&Value::Bool(true)));

        for (negated, expected_len) in [(false, 0), (true, 2)] {
            let filter = PhysicalPlan::Filter {
                input: Box::new(PhysicalPlan::Values {
                    values: vec![null_left.clone(), missing_left.clone()],
                }),
                predicate: in_expr(negated),
            };
            let filtered =
                execute_physical_plan_with_source(&filter, &source, &QueryContext::default())
                    .unwrap();
            assert_eq!(filtered.len(), expected_len);

            let apply = PhysicalPlan::ApplyInSubquery {
                input: Box::new(PhysicalPlan::Values {
                    values: vec![null_left.clone(), missing_left.clone()],
                }),
                left: Expr::Operand(Operand::Field(FieldPath::from_fields(["v"]))),
                subquery: Box::new(PhysicalPlan::Values { values: Vec::new() }),
                negated,
            };
            let applied =
                execute_physical_plan_with_source(&apply, &source, &QueryContext::default())
                    .unwrap();
            assert_eq!(applied.len(), expected_len);
        }
    }

    #[test]
    fn apply_in_subquery_filters_unknown_null_results() {
        let mut left_null = Object::new();
        left_null.insert("v", Value::Null);
        let mut subquery_null = Object::new();
        subquery_null.insert("x", Value::Null);
        let plan = |negated| PhysicalPlan::ApplyInSubquery {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("v", 1), obj_i64("v", 2), left_null.clone()],
            }),
            left: Expr::Operand(Operand::Field(FieldPath::from_fields(["v"]))),
            subquery: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("x", 1), subquery_null.clone()],
            }),
            negated,
        };

        let source = InlineSource {
            left: Vec::new(),
            right: Vec::new(),
        };
        let in_rows =
            execute_physical_plan_with_source(&plan(false), &source, &QueryContext::default())
                .unwrap();
        assert_eq!(in_rows.len(), 1);
        assert_eq!(in_rows[0].get("v"), Some(&Value::I64(1)));

        let not_in_rows =
            execute_physical_plan_with_source(&plan(true), &source, &QueryContext::default())
                .unwrap();
        assert!(not_in_rows.is_empty());
    }

    #[test]
    fn apply_in_subquery_requires_exactly_one_column() {
        let mut subquery_row = Object::new();
        subquery_row.insert("x", Value::I64(1));
        subquery_row.insert("y", Value::I64(2));
        let plan = PhysicalPlan::ApplyInSubquery {
            input: Box::new(PhysicalPlan::Values {
                values: vec![obj_i64("v", 1)],
            }),
            left: Expr::Operand(Operand::Field(FieldPath::from_fields(["v"]))),
            subquery: Box::new(PhysicalPlan::Values {
                values: vec![subquery_row],
            }),
            negated: false,
        };

        let err = execute_physical_plan_with_source(
            &plan,
            &InlineSource {
                left: Vec::new(),
                right: Vec::new(),
            },
            &QueryContext::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("expected exactly one"));
    }
}
