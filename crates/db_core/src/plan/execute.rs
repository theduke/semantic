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
            let sub_values: BTreeSet<Value> = collect_dyn_stream(execute_physical_dyn_stream(
                *subquery,
                source.clone(),
                context.clone(),
                options,
            ))
            .await?
            .into_iter()
            .filter_map(|row| row.to_object().into_btree().into_values().next())
            .collect();
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
    normalize_record_batch_stream(
        input
            .map_ok(move |batch| {
                batch
                    .into_iter()
                    .filter(|row| {
                        let contains = evaluate_expr(row.as_ref(), &left)
                            .map(|value| sub_values.contains(&value))
                            .unwrap_or(false);
                        if negated { !contains } else { contains }
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
    futures::executor::block_on(execute_physical_plan_with_source_async(
        plan,
        source,
        ExecutionOptions::default(),
        context,
    ))
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
                    return Ok(Expr::InList {
                        expr: Box::new(
                            resolve_expr_subqueries_async(left, source.clone(), context, options)
                                .await?,
                        ),
                        list: execute_list_subquery_async(query, source, context, options)
                            .await?
                            .into_iter()
                            .map(|value| Expr::Operand(Operand::Literal(value)))
                            .collect(),
                        negated: false,
                    });
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
                let exists = !execute_select_subquery_async(query, source, context, options)
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

async fn execute_select_subquery_async(
    query: &crate::query::SelectQuery,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: &QueryContext,
    options: ExecutionOptions,
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
    Ok(collect_dyn_stream(execute_physical_dyn_stream(
        plan.physical,
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
    let rows = execute_select_subquery_async(query, source, context, options).await?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(Value::Null);
    };
    Ok(row.into_btree().into_values().next().unwrap_or(Value::Null))
}

async fn execute_list_subquery_async(
    query: &crate::query::SelectQuery,
    source: Arc<dyn AsyncPhysicalDataSource + '_>,
    context: &QueryContext,
    options: ExecutionOptions,
) -> CoreResult<Vec<Value>> {
    Ok(
        execute_select_subquery_async(query, source, context, options)
            .await?
            .into_iter()
            .filter_map(|row| row.into_btree().into_values().next())
            .collect(),
    )
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
            evaluate_filter_expr(&merged, predicate)
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
    value_ref_for_field(row, &key.field, Some(&key.source_path))
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
            let one = rows.first()?;
            evaluate_expr(
                one,
                &Expr::Unary {
                    op: *op,
                    expr: Box::new(rewrite_group_expr(rows, expr)?),
                },
            )
        }
        Expr::Binary { op, left, right } => {
            let one = rows.first()?;
            evaluate_expr(
                one,
                &Expr::Binary {
                    op: *op,
                    left: Box::new(rewrite_group_expr(rows, left)?),
                    right: Box::new(rewrite_group_expr(rows, right)?),
                },
            )
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            let one = rows.first()?;
            evaluate_expr(
                one,
                &Expr::IfElse {
                    cond: Box::new(rewrite_group_expr(rows, cond)?),
                    then_expr: Box::new(rewrite_group_expr(rows, then_expr)?),
                    else_expr: Box::new(rewrite_group_expr(rows, else_expr)?),
                },
            )
        }
        Expr::Coalesce(items) => {
            let mut rewritten = Vec::with_capacity(items.len());
            for item in items {
                rewritten.push(rewrite_group_expr(rows, item)?);
            }
            evaluate_expr(rows.first()?, &Expr::Coalesce(rewritten))
        }
        Expr::Function { name, args } => {
            let mut rewritten = Vec::with_capacity(args.len());
            for arg in args {
                rewritten.push(match arg {
                    FunctionArg::Expr(expr) => FunctionArg::Expr(rewrite_group_expr(rows, expr)?),
                    FunctionArg::Wildcard => FunctionArg::Wildcard,
                });
            }
            evaluate_expr(
                rows.first()?,
                &Expr::Function {
                    name: name.clone(),
                    args: rewritten,
                },
            )
        }
        _ => rows.first().and_then(|row| evaluate_expr(row, expr)),
    }
}

fn rewrite_group_expr(rows: &[Object], expr: &Expr) -> Option<Expr> {
    if expr_contains_aggregate(expr) {
        evaluate_group_expr(rows, expr).map(|value| Expr::Operand(Operand::Literal(value)))
    } else {
        Some(expr.clone())
    }
}

fn expr_contains_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Aggregate { .. } => true,
        Expr::Unary { expr, .. } => expr_contains_aggregate(expr),
        Expr::Binary { left, right, .. } => {
            expr_contains_aggregate(left) || expr_contains_aggregate(right)
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_aggregate(cond)
                || expr_contains_aggregate(then_expr)
                || expr_contains_aggregate(else_expr)
        }
        Expr::Coalesce(items) => items.iter().any(expr_contains_aggregate),
        Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            FunctionArg::Expr(expr) => expr_contains_aggregate(expr),
            FunctionArg::Wildcard => false,
        }),
        Expr::InList { expr, list, .. } => {
            expr_contains_aggregate(expr) || list.iter().any(expr_contains_aggregate)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_aggregate(expr)
                || expr_contains_aggregate(low)
                || expr_contains_aggregate(high)
        }
        Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
            expr_contains_aggregate(expr) || expr_contains_aggregate(pattern)
        }
        Expr::IsNull { expr, .. } => expr_contains_aggregate(expr),
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            expr_contains_aggregate(relation)
                || expr_contains_aggregate(source)
                || expr_contains_aggregate(target)
                || max_depth
                    .as_ref()
                    .is_some_and(|depth| expr_contains_aggregate(depth))
        }
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::Operand(_) => false,
    }
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
        expr::Expr::Literal(lit) => Ok(literal_to_value(&lit.value)),
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

fn literal_to_value(lit: &semantic_data::schema::core::literal_value::LiteralValue) -> Value {
    use semantic_data::schema::core::literal_value::LiteralValue as LV;
    use semantic_data::value::Map;
    match lit {
        LV::Null => Value::Null,
        LV::Bool(b) => Value::Bool(*b),
        LV::Int(v) => Value::I128(*v),
        LV::UInt(v) => Value::U128(*v),
        LV::Float(s) => Value::String(s.clone()),
        LV::String(s) => Value::String(s.clone()),
        LV::Bytes(b) => Value::Bytes(b.clone().into()),
        LV::List(items) => Value::List(items.iter().map(literal_to_value).collect()),
        LV::Map(entries) => {
            let mut map = Map::new();
            for (k, v) in entries {
                map.insert(Value::String(k.clone()), literal_to_value(v));
            }
            Value::Map(map)
        }
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
    use futures::executor;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

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
        let batches = executor::block_on(
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
        let batches = executor::block_on(
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
                }],
            }),
            offset: Expr::from(1usize),
            limit: Some(Expr::from(2usize)),
        };

        let out = executor::block_on(execute_physical_plan_collect(
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

        let filtered = executor::block_on(execute_physical_plan_collect(
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
        let indexed = executor::block_on(execute_physical_plan_collect(
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
        let mut r1 = Object::new();
        r1.insert("rid", Value::I64(2));

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
                left: vec![l1, l2],
                right: vec![r1],
            },
            &QueryContext::default(),
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].contains_key("l"));
        assert!(out[0].contains_key("r"));
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
                        }]),
                )),
                field: None,
                source_path: None,
                alias: Some("sv".to_string()),
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
}
