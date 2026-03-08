use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

use futures::{FutureExt, TryStreamExt, future::BoxFuture, stream::BoxStream};
use semantic_data::query::{AggregateOp, JoinType};
use semantic_data::value::{FieldPath, Object, Value, ValueRef};

use crate::QueryContext;
use crate::plan::{
    FieldRef, PhysicalJoinAlgorithm, PhysicalJoinCondition, PhysicalJoinKey, PhysicalJoinPlan,
    PhysicalOrderField, PhysicalPlan, PhysicalProjectionField, PhysicalSource, SourceRef,
};
use crate::query::{
    CoreError, CoreResult, Expr, FunctionArg, ObjectAccess as QueryObjectAccess, Operand,
    compare_objects_for_plan, evaluate_expr, evaluate_filter_expr, evaluate_usize_expr,
};

pub type DynObject = Box<dyn QueryObjectAccess>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionOptions {
    pub parallel_union_branches: bool,
}

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            parallel_union_branches: true,
        }
    }
}

pub trait PhysicalDataSource: Send + Sync {
    fn scan(&self, source: &SourceRef) -> CoreResult<Vec<DynObject>>;

    fn scan_filtered(&self, source: &SourceRef, predicate: &Expr) -> CoreResult<Vec<DynObject>> {
        let items = self.scan(source)?;
        Ok(items
            .into_iter()
            .filter(|item| evaluate_filter_expr(item.as_ref(), predicate))
            .collect())
    }

    fn index_lookup(
        &self,
        source: &SourceRef,
        field: &FieldRef,
        value: &Value,
    ) -> CoreResult<Vec<DynObject>> {
        let items = self.scan(source)?;
        Ok(items
            .into_iter()
            .filter(|item| {
                value_ref_for_field(item.as_ref(), field, None)
                    .map(|v| v.into_owned() == *value)
                    .unwrap_or(false)
            })
            .collect())
    }
}

pub trait AsyncPhysicalDataSource: Send + Sync {
    fn scan_stream<'a>(&'a self, source: &'a SourceRef) -> BoxStream<'a, CoreResult<DynObject>>;

    fn scan<'a>(&'a self, source: &'a SourceRef) -> BoxFuture<'a, CoreResult<Vec<DynObject>>> {
        self.scan_stream(source).try_collect().boxed()
    }
}

pub fn execute_physical_plan_with_source(
    plan: &PhysicalPlan,
    source: &dyn PhysicalDataSource,
    _context: &QueryContext,
) -> CoreResult<Vec<Object>> {
    execute_physical_dyn(plan, source)
        .map(|items| items.into_iter().map(|item| item.to_object()).collect())
}

pub async fn execute_physical_plan_with_source_async(
    plan: &PhysicalPlan,
    source: &dyn AsyncPhysicalDataSource,
    options: ExecutionOptions,
    _context: &QueryContext,
) -> CoreResult<Vec<Object>> {
    let out = execute_physical_dyn_async(plan, source, options).await?;
    Ok(out.into_iter().map(|item| item.to_object()).collect())
}

pub fn execute_physical_plan(
    plan: &PhysicalPlan,
    rows: Vec<Object>,
    context: &QueryContext,
) -> Vec<Object> {
    struct InlineDataSource {
        rows: Vec<Object>,
    }

    impl PhysicalDataSource for InlineDataSource {
        fn scan(&self, source: &SourceRef) -> CoreResult<Vec<DynObject>> {
            if source.source_name.is_some() || source.collection_id.is_some() {
                return Err(CoreError::new(
                    "inline physical executor does not support named sources",
                ));
            }
            Ok(self
                .rows
                .iter()
                .cloned()
                .map(|row| Box::new(row) as DynObject)
                .collect())
        }
    }

    execute_physical_plan_with_source(plan, &InlineDataSource { rows }, context).unwrap_or_default()
}

fn execute_physical_dyn(
    plan: &PhysicalPlan,
    source: &dyn PhysicalDataSource,
) -> CoreResult<Vec<DynObject>> {
    match plan {
        PhysicalPlan::Source(PhysicalSource::Scan { source: source_ref }) => {
            source.scan(source_ref)
        }
        PhysicalPlan::Source(PhysicalSource::FilteredScan {
            source: source_ref,
            predicate,
        }) => source.scan_filtered(source_ref, predicate),
        PhysicalPlan::Source(PhysicalSource::IndexLookup {
            source: source_ref,
            field,
            value,
            residual_predicate,
        }) => {
            let items = source.index_lookup(source_ref, field, value)?;
            if let Some(residual) = residual_predicate {
                Ok(items
                    .into_iter()
                    .filter(|item| evaluate_filter_expr(item.as_ref(), residual))
                    .collect())
            } else {
                Ok(items)
            }
        }
        PhysicalPlan::Values { values } => Ok(values
            .iter()
            .cloned()
            .map(|item| Box::new(item) as DynObject)
            .collect()),
        PhysicalPlan::Filter { input, predicate } => Ok(execute_physical_dyn(input, source)?
            .into_iter()
            .filter(|row| evaluate_filter_expr(row.as_ref(), predicate))
            .collect()),
        PhysicalPlan::Sort { input, order_by } => {
            let mut out = execute_physical_dyn(input, source)?;
            out.sort_by(|a, b| compare_dyn_objects(a.as_ref(), b.as_ref(), order_by));
            Ok(out)
        }
        PhysicalPlan::Project { input, projection } => Ok(execute_physical_dyn(input, source)?
            .into_iter()
            .map(|row| Box::new(project_dyn_object(row.as_ref(), projection)) as DynObject)
            .collect()),
        PhysicalPlan::Aggregate {
            input,
            group_by,
            projection,
            having,
        } => execute_aggregate(
            execute_physical_dyn(input, source)?,
            group_by,
            projection,
            having,
        ),
        PhysicalPlan::Limit {
            input,
            offset,
            limit,
        } => {
            let offset = evaluate_usize_expr(offset)
                .ok_or_else(|| CoreError::new("failed to evaluate OFFSET expression"))?;
            let limit = limit
                .as_ref()
                .map(|expr| {
                    evaluate_usize_expr(expr)
                        .ok_or_else(|| CoreError::new("failed to evaluate LIMIT expression"))
                })
                .transpose()?;
            let iter = execute_physical_dyn(input, source)?
                .into_iter()
                .skip(offset);
            if let Some(limit) = limit {
                Ok(iter.take(limit).collect())
            } else {
                Ok(iter.collect())
            }
        }
        PhysicalPlan::Distinct { input } => {
            let mut dedup = BTreeSet::<Object>::new();
            Ok(execute_physical_dyn(input, source)?
                .into_iter()
                .filter(|item| dedup.insert(item.to_object()))
                .collect())
        }
        PhysicalPlan::Union { inputs, all } => {
            let mut merged = Vec::<DynObject>::new();
            for input in inputs {
                merged.extend(execute_physical_dyn(input, source)?);
            }
            if *all {
                return Ok(merged);
            }
            let mut dedup = BTreeSet::<Object>::new();
            Ok(merged
                .into_iter()
                .filter(|item| dedup.insert(item.to_object()))
                .collect())
        }
        PhysicalPlan::Join(join) => execute_join(join, source),
        PhysicalPlan::ApplyExists {
            input,
            subquery,
            negated,
        } => {
            let subquery_any = !execute_physical_dyn(subquery, source)?.is_empty();
            Ok(execute_physical_dyn(input, source)?
                .into_iter()
                .filter(|_| {
                    if *negated {
                        !subquery_any
                    } else {
                        subquery_any
                    }
                })
                .collect())
        }
        PhysicalPlan::ApplyInSubquery {
            input,
            left,
            subquery,
            negated,
        } => {
            let sub_values: BTreeSet<Value> = execute_physical_dyn(subquery, source)?
                .into_iter()
                .filter_map(|row| row.to_object().into_btree().into_values().next())
                .collect();
            Ok(execute_physical_dyn(input, source)?
                .into_iter()
                .filter(|row| {
                    let contains = evaluate_expr(row.as_ref(), left)
                        .map(|v| sub_values.contains(&v))
                        .unwrap_or(false);
                    if *negated { !contains } else { contains }
                })
                .collect())
        }
        PhysicalPlan::Exchange { input, .. }
        | PhysicalPlan::RepartitionHash { input, .. }
        | PhysicalPlan::Materialize { input } => execute_physical_dyn(input, source),
    }
}

fn execute_physical_dyn_async<'a>(
    plan: &'a PhysicalPlan,
    source: &'a dyn AsyncPhysicalDataSource,
    options: ExecutionOptions,
) -> BoxFuture<'a, CoreResult<Vec<DynObject>>> {
    async move {
        match plan {
            PhysicalPlan::Source(PhysicalSource::Scan { source: source_ref }) => {
                source.scan(source_ref).await
            }
            PhysicalPlan::Source(PhysicalSource::FilteredScan {
                source: source_ref,
                predicate,
            }) => source.scan(source_ref).await.map(|items| {
                items
                    .into_iter()
                    .filter(|item| evaluate_filter_expr(item.as_ref(), predicate))
                    .collect()
            }),
            PhysicalPlan::Source(PhysicalSource::IndexLookup {
                source: source_ref, ..
            }) => source.scan(source_ref).await,
            PhysicalPlan::Values { values } => Ok(values
                .iter()
                .cloned()
                .map(|item| Box::new(item) as DynObject)
                .collect()),
            PhysicalPlan::Filter { input, predicate } => {
                Ok(execute_physical_dyn_async(input, source, options)
                    .await?
                    .into_iter()
                    .filter(|row| evaluate_filter_expr(row.as_ref(), predicate))
                    .collect())
            }
            PhysicalPlan::Sort { input, order_by } => {
                let mut out = execute_physical_dyn_async(input, source, options).await?;
                out.sort_by(|a, b| compare_dyn_objects(a.as_ref(), b.as_ref(), order_by));
                Ok(out)
            }
            PhysicalPlan::Project { input, projection } => {
                Ok(execute_physical_dyn_async(input, source, options)
                    .await?
                    .into_iter()
                    .map(|row| Box::new(project_dyn_object(row.as_ref(), projection)) as DynObject)
                    .collect())
            }
            PhysicalPlan::Aggregate {
                input,
                group_by,
                projection,
                having,
            } => execute_aggregate(
                execute_physical_dyn_async(input, source, options).await?,
                group_by,
                projection,
                having,
            ),
            PhysicalPlan::Limit {
                input,
                offset,
                limit,
            } => {
                let offset = evaluate_usize_expr(offset)
                    .ok_or_else(|| CoreError::new("failed to evaluate OFFSET expression"))?;
                let limit = limit
                    .as_ref()
                    .map(|expr| {
                        evaluate_usize_expr(expr)
                            .ok_or_else(|| CoreError::new("failed to evaluate LIMIT expression"))
                    })
                    .transpose()?;
                let iter = execute_physical_dyn_async(input, source, options)
                    .await?
                    .into_iter()
                    .skip(offset);
                if let Some(limit) = limit {
                    Ok(iter.take(limit).collect())
                } else {
                    Ok(iter.collect())
                }
            }
            PhysicalPlan::Distinct { input } => {
                let mut dedup = BTreeSet::<Object>::new();
                Ok(execute_physical_dyn_async(input, source, options)
                    .await?
                    .into_iter()
                    .filter(|item| dedup.insert(item.to_object()))
                    .collect())
            }
            PhysicalPlan::Union { inputs, all } => {
                let mut merged = Vec::<DynObject>::new();
                if options.parallel_union_branches {
                    let futures = inputs
                        .iter()
                        .map(|input| execute_physical_dyn_async(input, source, options));
                    for out in futures::future::try_join_all(futures).await? {
                        merged.extend(out);
                    }
                } else {
                    for input in inputs {
                        merged.extend(execute_physical_dyn_async(input, source, options).await?);
                    }
                }
                if *all {
                    return Ok(merged);
                }
                let mut dedup = BTreeSet::<Object>::new();
                Ok(merged
                    .into_iter()
                    .filter(|item| dedup.insert(item.to_object()))
                    .collect())
            }
            PhysicalPlan::Join(_)
            | PhysicalPlan::ApplyExists { .. }
            | PhysicalPlan::ApplyInSubquery { .. } => Err(CoreError::new(
                "async execution currently supports source/pipe operators only",
            )),
            PhysicalPlan::Exchange { input, .. }
            | PhysicalPlan::RepartitionHash { input, .. }
            | PhysicalPlan::Materialize { input } => {
                execute_physical_dyn_async(input, source, options).await
            }
        }
    }
    .boxed()
}

fn execute_join(
    join: &PhysicalJoinPlan,
    source: &dyn PhysicalDataSource,
) -> CoreResult<Vec<DynObject>> {
    let left_rows = execute_physical_dyn(&join.left, source)?;
    let right_rows = execute_physical_dyn(&join.right, source)?;
    match join.algorithm {
        PhysicalJoinAlgorithm::Hash => execute_hash_join(join, left_rows, right_rows),
        PhysicalJoinAlgorithm::NestedLoop | PhysicalJoinAlgorithm::Merge => {
            execute_nested_loop_join(join, left_rows, right_rows)
        }
    }
}

fn execute_hash_join(
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
        Expr::InSubquery { .. } | Expr::Exists { .. } | Expr::Operand(_) => false,
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
                .or_else(|| fallback_path.and_then(|path| row.value_at_path_ref(path)))
        }
        FieldRef::Path(path) => row.value_at_path_ref(path),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ValueKey(Value);

impl ValueKey {
    fn from_ref(value: ValueRef<'_>) -> Self {
        Self(value.into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::Operand;

    struct InlineSource {
        left: Vec<Object>,
        right: Vec<Object>,
    }

    impl PhysicalDataSource for InlineSource {
        fn scan(&self, source: &SourceRef) -> CoreResult<Vec<DynObject>> {
            let values = match source.source_name.as_deref() {
                Some("left") => &self.left,
                Some("right") => &self.right,
                _ => &self.left,
            };
            Ok(values
                .iter()
                .cloned()
                .map(|item| Box::new(item) as DynObject)
                .collect())
        }
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
}
