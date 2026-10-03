//! Streaming grouped aggregation.
//!
//! [`GroupedAggregation`] consumes input rows one at a time and keeps, per
//! group, one accumulator per distinct aggregate call of the projection and
//! `HAVING` clause. Input rows are not retained: `count`, `sum` and `avg`
//! keep running totals, `min` and `max` the best value so far, and only
//! `DISTINCT` aggregates keep the set of values they have seen. When a
//! projection or `HAVING` expression reads row fields outside of an
//! aggregate, those are evaluated on the first row of the group, so that row
//! (and only that row) is kept per group.
//!
//! Every expression shape streams; there is no collecting fallback. Group
//! keys and aggregate arguments are evaluated on the materialized row
//! (`to_object`), exactly like the former collect-then-aggregate operator.

use std::collections::{BTreeMap, BTreeSet};

use semantic_data::query::AggregateOp;
use semantic_data::value::{Object, Value};

use super::{DynObject, aggregate_output_key};
use crate::plan::PhysicalProjectionField;
use crate::query::{Expr, FunctionArg, ObjectAccess, Operand, evaluate_expr};

/// The rows of one group as seen by group-level expression evaluation.
pub(super) trait GroupRows {
    /// The first input row of the group, if the group has one and it is
    /// needed.
    fn first_row(&self) -> Option<&Object>;

    /// Result of an aggregate call over the group; `None` when the aggregate
    /// is undefined (for example `sum` over a non-numeric value).
    fn aggregate(&self, op: AggregateOp, distinct: bool, arg: &FunctionArg) -> Option<Value>;
}

/// One aggregate call of an aggregate plan.
#[derive(Debug, Clone, PartialEq)]
struct AggregateCall {
    op: AggregateOp,
    distinct: bool,
    arg: FunctionArg,
}

/// Aggregate calls and first-row needs of group-level expressions.
#[derive(Debug, Default)]
struct GroupExprInfo {
    calls: Vec<AggregateCall>,
    needs_first_row: bool,
}

impl GroupExprInfo {
    /// Visit `expr` the way [`evaluate_group_expr`] does.
    fn visit(&mut self, expr: &Expr) {
        match expr {
            Expr::Aggregate { op, distinct, arg } => {
                let call = AggregateCall {
                    op: *op,
                    distinct: *distinct,
                    arg: arg.as_ref().clone(),
                };
                if !self.calls.contains(&call) {
                    self.calls.push(call);
                }
            }
            Expr::Operand(Operand::Literal(_) | Operand::Parameter(_)) => {}
            Expr::Operand(Operand::Field(_))
            | Expr::Subquery(_)
            | Expr::Exists { .. }
            | Expr::RelationExists { .. } => self.needs_first_row = true,
            Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } | Expr::ProjectionRef(expr) => {
                self.visit(expr)
            }
            Expr::Binary { left, right, .. }
            | Expr::PatternMatch {
                expr: left,
                pattern: right,
                ..
            }
            | Expr::RegexMatch {
                expr: left,
                pattern: right,
                ..
            } => {
                self.visit(left);
                self.visit(right);
            }
            Expr::IfElse {
                cond,
                then_expr,
                else_expr,
            } => {
                self.visit(cond);
                self.visit(then_expr);
                self.visit(else_expr);
            }
            Expr::Coalesce(items) => items.iter().for_each(|item| self.visit(item)),
            Expr::TextMatch { exprs, query, .. } => {
                exprs.iter().for_each(|item| self.visit(item));
                self.visit(query);
            }
            Expr::InList { expr, list, .. } => {
                self.visit(expr);
                list.iter().for_each(|item| self.visit(item));
            }
            Expr::Between {
                expr, low, high, ..
            } => {
                self.visit(expr);
                self.visit(low);
                self.visit(high);
            }
            Expr::Function { args, .. } => {
                for arg in args {
                    if let FunctionArg::Expr(expr) = arg {
                        self.visit(expr);
                    }
                }
            }
        }
    }

    fn position(&self, op: AggregateOp, distinct: bool, arg: &FunctionArg) -> Option<usize> {
        self.calls
            .iter()
            .position(|call| call.op == op && call.distinct == distinct && &call.arg == arg)
    }
}

/// Running state of one aggregate call within one group.
#[derive(Debug)]
struct Accumulator {
    state: AccumulatorState,
    /// Values already aggregated, for `DISTINCT` calls only.
    seen: Option<BTreeSet<Value>>,
}

#[derive(Debug)]
enum AccumulatorState {
    /// `count(*)`.
    Rows(i64),
    /// `count(expr)`.
    Values(i64),
    Sum {
        sum: f64,
        found: bool,
    },
    Avg {
        sum: f64,
        count: usize,
    },
    Min(Option<Value>),
    Max(Option<Value>),
    /// The call has no value: a wildcard argument of a non-count aggregate,
    /// or a non-numeric input of `sum` / `avg`.
    Undefined,
}

impl Accumulator {
    fn new(call: &AggregateCall) -> Self {
        let state = match (call.op, &call.arg) {
            (AggregateOp::Count, FunctionArg::Wildcard) => AccumulatorState::Rows(0),
            (_, FunctionArg::Wildcard) => AccumulatorState::Undefined,
            (AggregateOp::Count, FunctionArg::Expr(_)) => AccumulatorState::Values(0),
            (AggregateOp::Sum, FunctionArg::Expr(_)) => AccumulatorState::Sum {
                sum: 0.0,
                found: false,
            },
            (AggregateOp::Avg, FunctionArg::Expr(_)) => {
                AccumulatorState::Avg { sum: 0.0, count: 0 }
            }
            (AggregateOp::Min, FunctionArg::Expr(_)) => AccumulatorState::Min(None),
            (AggregateOp::Max, FunctionArg::Expr(_)) => AccumulatorState::Max(None),
        };
        let aggregates_values = !matches!(
            state,
            AccumulatorState::Rows(_) | AccumulatorState::Undefined
        );
        Self {
            state,
            seen: (call.distinct && aggregates_values).then(BTreeSet::new),
        }
    }

    fn update(&mut self, arg: &FunctionArg, row: &Object) {
        let expr = match (&mut self.state, arg) {
            (AccumulatorState::Rows(count), _) => {
                *count += 1;
                return;
            }
            (AccumulatorState::Undefined, _) | (_, FunctionArg::Wildcard) => return,
            (_, FunctionArg::Expr(expr)) => expr,
        };
        let Some(value) = evaluate_expr(row, expr) else {
            return;
        };
        if value.is_nullish() {
            return;
        }
        if let Some(seen) = &mut self.seen
            && !seen.insert(value.clone())
        {
            return;
        }
        let mut undefined = false;
        match &mut self.state {
            AccumulatorState::Values(count) => *count += 1,
            AccumulatorState::Sum { sum, found } => match value.as_f64() {
                Some(number) => {
                    *sum += number;
                    *found = true;
                }
                None => undefined = true,
            },
            AccumulatorState::Avg { sum, count } => match value.as_f64() {
                Some(number) => {
                    *sum += number;
                    *count += 1;
                }
                None => undefined = true,
            },
            AccumulatorState::Min(best) => {
                if best.as_ref().is_none_or(|current| value < *current) {
                    *best = Some(value);
                }
            }
            AccumulatorState::Max(best) => {
                if best.as_ref().is_none_or(|current| value > *current) {
                    *best = Some(value);
                }
            }
            AccumulatorState::Rows(_) | AccumulatorState::Undefined => {}
        }
        if undefined {
            self.state = AccumulatorState::Undefined;
            self.seen = None;
        }
    }

    fn finish(self) -> Option<Value> {
        match self.state {
            AccumulatorState::Rows(count) | AccumulatorState::Values(count) => {
                Some(Value::I64(count))
            }
            AccumulatorState::Sum { sum, found } => Some(if found {
                Value::F64(sum.into())
            } else {
                Value::Null
            }),
            AccumulatorState::Avg { sum, count } => Some(if count == 0 {
                Value::Null
            } else {
                Value::F64((sum / count as f64).into())
            }),
            AccumulatorState::Min(best) | AccumulatorState::Max(best) => {
                Some(best.unwrap_or(Value::Null))
            }
            AccumulatorState::Undefined => None,
        }
    }
}

/// Accumulated state of one group.
#[derive(Debug)]
struct GroupState {
    first_row: Option<Object>,
    accumulators: Vec<Accumulator>,
}

/// A finished group, ready for projection and `HAVING`.
struct FinishedGroup<'a> {
    info: &'a GroupExprInfo,
    first_row: Option<Object>,
    values: Vec<Option<Value>>,
}

impl GroupRows for FinishedGroup<'_> {
    fn first_row(&self) -> Option<&Object> {
        self.first_row.as_ref()
    }

    fn aggregate(&self, op: AggregateOp, distinct: bool, arg: &FunctionArg) -> Option<Value> {
        let index = self.info.position(op, distinct, arg)?;
        self.values[index].clone()
    }
}

/// Streaming `GROUP BY` / aggregate operator.
///
/// Groups are emitted in ascending group-key order. Without `GROUP BY` all
/// rows form one group, which exists (and is emitted) even for empty input.
pub(super) struct GroupedAggregation {
    group_by: Vec<Expr>,
    projection: Vec<PhysicalProjectionField>,
    having: Option<Expr>,
    info: GroupExprInfo,
    groups: BTreeMap<Vec<Value>, GroupState>,
}

impl GroupedAggregation {
    pub(super) fn new(
        group_by: Vec<Expr>,
        projection: Vec<PhysicalProjectionField>,
        having: Option<Expr>,
    ) -> Self {
        let mut info = GroupExprInfo::default();
        for field in &projection {
            info.visit(&field.expr);
        }
        if let Some(having) = &having {
            info.visit(having);
        }
        let mut groups = BTreeMap::new();
        if group_by.is_empty() {
            groups.insert(Vec::new(), GroupState::new(&info));
        }
        Self {
            group_by,
            projection,
            having,
            info,
            groups,
        }
    }

    /// Add one input row to its group.
    pub(super) fn push(&mut self, row: &dyn ObjectAccess) {
        let row = row.to_object();
        let key = self
            .group_by
            .iter()
            .map(|expr| evaluate_expr(&row, expr).unwrap_or(Value::Null))
            .collect::<Vec<_>>();
        let info = &self.info;
        let group = self
            .groups
            .entry(key)
            .or_insert_with(|| GroupState::new(info));
        for (accumulator, call) in group.accumulators.iter_mut().zip(&info.calls) {
            accumulator.update(&call.arg, &row);
        }
        if info.needs_first_row && group.first_row.is_none() {
            group.first_row = Some(row);
        }
    }

    /// Number of groups built so far.
    pub(super) fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Evaluate `HAVING` and the projection of every group.
    pub(super) fn finish(self) -> Vec<DynObject> {
        let Self {
            projection,
            having,
            info,
            groups,
            ..
        } = self;
        let mut out = Vec::<DynObject>::new();
        for group in groups.into_values() {
            let group = FinishedGroup {
                info: &info,
                first_row: group.first_row,
                values: group
                    .accumulators
                    .into_iter()
                    .map(Accumulator::finish)
                    .collect(),
            };
            if let Some(having) = &having
                && !evaluate_group_predicate(&group, having)
            {
                continue;
            }
            let mut projected = Object::new();
            for field in &projection {
                let Some(value) = evaluate_group_expr(&group, &field.expr) else {
                    continue;
                };
                projected.insert(aggregate_output_key(field), value);
            }
            out.push(Box::new(projected) as DynObject);
        }
        out
    }
}

impl GroupState {
    fn new(info: &GroupExprInfo) -> Self {
        Self {
            first_row: None,
            accumulators: info.calls.iter().map(Accumulator::new).collect(),
        }
    }
}

pub(super) fn evaluate_group_predicate<G: GroupRows + ?Sized>(group: &G, predicate: &Expr) -> bool {
    evaluate_group_expr(group, predicate)
        .as_ref()
        .is_some_and(|value| match value {
            Value::Bool(v) => *v,
            Value::Null | Value::Void => false,
            _ => true,
        })
}

pub(super) fn evaluate_group_expr<G: GroupRows + ?Sized>(group: &G, expr: &Expr) -> Option<Value> {
    match expr {
        Expr::Aggregate { op, distinct, arg } => group.aggregate(*op, *distinct, arg),
        Expr::ProjectionRef(_) | Expr::Operand(Operand::Parameter(_)) => None,
        Expr::Unary { op, expr } => {
            let value = evaluate_group_expr(group, expr)?;
            if value.is_nullish() {
                return Some(Value::Null);
            }
            let empty = Object::new();
            let one = group.first_row().unwrap_or(&empty);
            evaluate_expr(
                one,
                &Expr::Unary {
                    op: *op,
                    expr: Box::new(Expr::Operand(Operand::Literal(value))),
                },
            )
        }
        Expr::Binary { op, left, right } => {
            let left = evaluate_group_expr(group, left)?;
            let right = evaluate_group_expr(group, right)?;
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
            let one = group.first_row().unwrap_or(&empty);
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
            let condition = evaluate_group_expr(group, cond)?;
            if matches!(condition, Value::Bool(true)) {
                evaluate_group_expr(group, then_expr)
            } else {
                evaluate_group_expr(group, else_expr)
            }
        }
        Expr::Coalesce(items) => {
            for item in items {
                let Some(value) = evaluate_group_expr(group, item) else {
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
                        let value = evaluate_group_expr(group, expr)?;
                        contains_null |= value.is_nullish();
                        FunctionArg::Expr(Expr::Operand(Operand::Literal(value)))
                    }
                    FunctionArg::Wildcard => FunctionArg::Wildcard,
                });
            }
            let empty = Object::new();
            let one = group.first_row().unwrap_or(&empty);
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
            let target = evaluate_group_expr(group, expr)?;
            if list.is_empty() {
                return Some(Value::Bool(*negated));
            }
            if target.is_nullish() {
                return Some(Value::Null);
            }
            let mut contains_null = false;
            for item in list {
                let Some(candidate) = evaluate_group_expr(group, item) else {
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
            let value = evaluate_group_expr(group, expr)?;
            let low = evaluate_group_expr(group, low)?;
            let high = evaluate_group_expr(group, high)?;
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
            let value = evaluate_group_expr(group, expr)?;
            let pattern = evaluate_group_expr(group, pattern)?;
            if value.is_nullish() || pattern.is_nullish() {
                return Some(Value::Null);
            }
            evaluate_group_scalar_expr(
                group,
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
            let value = evaluate_group_expr(group, expr)?;
            let pattern = evaluate_group_expr(group, pattern)?;
            if value.is_nullish() || pattern.is_nullish() {
                return Some(Value::Null);
            }
            evaluate_group_scalar_expr(
                group,
                Expr::RegexMatch {
                    expr: Box::new(Expr::Operand(Operand::Literal(value))),
                    pattern: Box::new(Expr::Operand(Operand::Literal(pattern))),
                    case_insensitive: *case_insensitive,
                    negated: *negated,
                },
            )
        }
        Expr::TextMatch {
            exprs,
            query,
            mode,
            analyzer,
        } => {
            let literal = |value: Value| Expr::Operand(Operand::Literal(value));
            let exprs = exprs
                .iter()
                .map(|expr| literal(evaluate_group_expr(group, expr).unwrap_or(Value::Null)))
                .collect();
            let query = evaluate_group_expr(group, query)?;
            evaluate_group_scalar_expr(
                group,
                Expr::TextMatch {
                    exprs,
                    query: Box::new(literal(query)),
                    mode: *mode,
                    analyzer: *analyzer,
                },
            )
        }
        Expr::IsNull { expr, negated } => {
            let is_null = evaluate_group_expr(group, expr).is_none_or(|value| value.is_nullish());
            Some(Value::Bool(if *negated { !is_null } else { is_null }))
        }
        Expr::Operand(Operand::Literal(value)) => Some(value.clone()),
        Expr::Operand(Operand::Field(_))
        | Expr::Subquery(_)
        | Expr::Exists { .. }
        | Expr::RelationExists { .. } => group.first_row().and_then(|row| evaluate_expr(row, expr)),
    }
}

fn evaluate_group_scalar_expr<G: GroupRows + ?Sized>(group: &G, expr: Expr) -> Option<Value> {
    let empty = Object::new();
    evaluate_expr(group.first_row().unwrap_or(&empty), &expr)
}

/// The former collect-then-aggregate operator, kept as a test oracle for the
/// streaming implementation.
#[cfg(test)]
pub(super) mod reference {
    use std::collections::BTreeSet;

    use semantic_data::query::AggregateOp;
    use semantic_data::value::{Object, Value};

    use super::{GroupRows, evaluate_group_expr, evaluate_group_predicate};
    use crate::plan::PhysicalProjectionField;
    use crate::plan::execute::aggregate_output_key;
    use crate::query::{Expr, FunctionArg, evaluate_expr};

    impl GroupRows for [Object] {
        fn first_row(&self) -> Option<&Object> {
            self.first()
        }

        fn aggregate(&self, op: AggregateOp, distinct: bool, arg: &FunctionArg) -> Option<Value> {
            evaluate_aggregate_expr(self, op, distinct, arg)
        }
    }

    pub(crate) fn aggregate_collected(
        owned_rows: Vec<Object>,
        group_by: &[Expr],
        projection: &[PhysicalProjectionField],
        having: &Option<Expr>,
    ) -> Vec<Object> {
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

        let mut out = Vec::new();
        for (_key, rows) in groups {
            if let Some(having) = having
                && !evaluate_group_predicate(rows.as_slice(), having)
            {
                continue;
            }
            let mut projected = Object::new();
            for field in projection {
                let value = evaluate_group_expr(rows.as_slice(), &field.expr);
                let Some(value) = value else {
                    continue;
                };
                projected.insert(aggregate_output_key(field), value);
            }
            out.push(projected);
        }
        out
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
}

#[cfg(test)]
mod tests {
    use semantic_data::query::{AggregateOp, BinaryOp};
    use semantic_data::value::{FieldPath, Object, Value};

    use super::GroupedAggregation;
    use super::reference::aggregate_collected;
    use crate::plan::PhysicalProjectionField;
    use crate::query::{Expr, FunctionArg, Operand};

    /// Deterministic splitmix64 generator for reproducible random data.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            (z ^ (z >> 31)) % bound
        }
    }

    fn field(name: &str) -> Expr {
        Expr::Operand(Operand::Field(FieldPath::from_fields([name])))
    }

    fn literal(value: Value) -> Expr {
        Expr::Operand(Operand::Literal(value))
    }

    fn aggregate(op: AggregateOp, distinct: bool, arg: Option<&str>) -> Expr {
        Expr::Aggregate {
            op,
            distinct,
            arg: Box::new(match arg {
                Some(name) => FunctionArg::Expr(field(name)),
                None => FunctionArg::Wildcard,
            }),
        }
    }

    fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
        Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn projection_field(alias: &str, expr: Expr) -> PhysicalProjectionField {
        PhysicalProjectionField {
            expr,
            field: None,
            source_path: None,
            alias: Some(alias.to_string()),
            wildcard: None,
        }
    }

    /// Rows with a few groups (`g`, `h`), a mostly numeric column `n` and a
    /// mixed-type column `m`, including nulls and missing fields.
    fn random_rows(rng: &mut Rng, count: usize) -> Vec<Object> {
        (0..count)
            .map(|position| {
                let mut row = Object::new();
                row.insert("position", Value::I64(position as i64));
                match rng.below(6) {
                    0 => {}
                    1 => {
                        row.insert("g", Value::Null);
                    }
                    other => {
                        row.insert("g", Value::I64(other as i64 % 3));
                    }
                }
                row.insert("h", Value::String(format!("h{}", rng.below(2))));
                match rng.below(7) {
                    0 => {}
                    1 => {
                        row.insert("n", Value::Null);
                    }
                    2 => {
                        row.insert("n", Value::F64((rng.below(8) as f64 / 4.0).into()));
                    }
                    _ => {
                        row.insert("n", Value::I64(rng.below(6) as i64 - 2));
                    }
                }
                match rng.below(5) {
                    0 => {}
                    1 => {
                        row.insert("m", Value::String(format!("m{}", rng.below(3))));
                    }
                    2 => {
                        row.insert("m", Value::Bool(rng.below(2) == 0));
                    }
                    _ => {
                        row.insert("m", Value::I64(rng.below(4) as i64));
                    }
                }
                row
            })
            .collect()
    }

    fn projection() -> Vec<PhysicalProjectionField> {
        use AggregateOp::{Avg, Count, Max, Min, Sum};
        let mut fields = vec![
            projection_field("g", field("g")),
            projection_field("first_position", field("position")),
            projection_field("rows", aggregate(Count, false, None)),
            projection_field("distinct_rows", aggregate(Count, true, None)),
            projection_field("sum_rows", aggregate(Sum, false, None)),
            projection_field(
                "rows_plus_sum",
                binary(
                    BinaryOp::Add,
                    aggregate(Count, false, None),
                    aggregate(Sum, false, Some("n")),
                ),
            ),
            projection_field(
                "literal_only",
                binary(
                    BinaryOp::Add,
                    literal(Value::I64(1)),
                    aggregate(Count, false, Some("n")),
                ),
            ),
            projection_field(
                "coalesced_min",
                Expr::Coalesce(vec![
                    aggregate(Min, false, Some("m")),
                    literal(Value::I64(-1)),
                ]),
            ),
        ];
        for column in ["n", "m"] {
            for distinct in [false, true] {
                for (name, op) in [
                    ("count", Count),
                    ("sum", Sum),
                    ("avg", Avg),
                    ("min", Min),
                    ("max", Max),
                ] {
                    let alias = format!("{name}_{}{column}", if distinct { "d_" } else { "" });
                    fields.push(projection_field(
                        &alias,
                        aggregate(op, distinct, Some(column)),
                    ));
                }
            }
        }
        fields
    }

    fn havings() -> Vec<Option<Expr>> {
        use AggregateOp::{Count, Max, Sum};
        vec![
            None,
            Some(binary(
                BinaryOp::Gt,
                aggregate(Count, false, None),
                literal(Value::I64(3)),
            )),
            Some(binary(
                BinaryOp::Gte,
                aggregate(Max, true, Some("n")),
                literal(Value::I64(2)),
            )),
            Some(binary(
                BinaryOp::Or,
                binary(
                    BinaryOp::Lt,
                    aggregate(Sum, false, Some("n")),
                    literal(Value::I64(0)),
                ),
                binary(
                    BinaryOp::Eq,
                    field("h"),
                    literal(Value::String("h1".into())),
                ),
            )),
        ]
    }

    fn stream_aggregate(
        rows: &[Object],
        group_by: &[Expr],
        projection: &[PhysicalProjectionField],
        having: &Option<Expr>,
    ) -> Vec<Object> {
        let mut aggregation =
            GroupedAggregation::new(group_by.to_vec(), projection.to_vec(), having.clone());
        for row in rows {
            aggregation.push(row);
        }
        aggregation
            .finish()
            .into_iter()
            .map(|row| row.to_object())
            .collect()
    }

    #[test]
    fn streaming_aggregation_matches_collecting_aggregation() {
        let mut rng = Rng(11);
        let group_bys = [vec![], vec![field("g")], vec![field("g"), field("h")]];
        let projection = projection();
        for round in 0..30 {
            let count = if round == 0 {
                0
            } else {
                rng.below(120) as usize
            };
            let rows = random_rows(&mut rng, count);
            for group_by in &group_bys {
                for having in havings() {
                    assert_eq!(
                        stream_aggregate(&rows, group_by, &projection, &having),
                        aggregate_collected(rows.clone(), group_by, &projection, &having),
                        "round {round}, group by {group_by:?}, having {having:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn first_row_is_kept_only_when_needed() {
        let aggregates_only = GroupedAggregation::new(
            vec![field("g")],
            vec![projection_field(
                "rows",
                aggregate(AggregateOp::Count, false, None),
            )],
            None,
        );
        assert!(!aggregates_only.info.needs_first_row);

        let with_field = GroupedAggregation::new(
            vec![field("g")],
            vec![projection_field("g", field("g"))],
            None,
        );
        assert!(with_field.info.needs_first_row);
    }

    #[test]
    fn repeated_aggregate_calls_share_one_accumulator() {
        let count = aggregate(AggregateOp::Count, false, None);
        let aggregation = GroupedAggregation::new(
            Vec::new(),
            vec![
                projection_field("a", count.clone()),
                projection_field("b", count.clone()),
            ],
            Some(binary(BinaryOp::Gt, count, literal(Value::I64(0)))),
        );
        assert_eq!(aggregation.info.calls.len(), 1);
    }
}
