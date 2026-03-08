use semantic_data::{
    query::BinaryOp,
    value::{FieldPath, PathSegment, Value},
};

use crate::QueryContext;
use crate::catalog::CollectionSchema;
use crate::plan::{
    FieldRef, LogicalJoinCondition, LogicalJoinPlan, LogicalPlan, PhysicalJoinAlgorithm,
    PhysicalJoinCondition, PhysicalJoinKey, PhysicalJoinPlan, PhysicalOrderField, PhysicalPlan,
    PhysicalProjectionField, PhysicalSource, SourceRef, StatsProvider, build_logical_plan,
    source_ref_for_collection,
};
use crate::query::{Expr, Operand, QueryField, SelectQuery};

#[derive(Debug, Clone, PartialEq)]
pub struct PlanPair {
    pub logical: LogicalPlan,
    pub physical: PhysicalPlan,
}

pub trait LogicalRewritePass: Send + Sync {
    fn name(&self) -> &'static str;
    fn rewrite(&self, plan: LogicalPlan, context: &QueryContext) -> LogicalPlan;
}

pub trait PhysicalLoweringPass: Send + Sync {
    fn name(&self) -> &'static str;
    fn lower(
        &self,
        plan: &LogicalPlan,
        context: &QueryContext,
        stats: Option<&dyn StatsProvider>,
        input: &dyn Fn(&LogicalPlan) -> PhysicalPlan,
    ) -> Option<PhysicalPlan>;
}

pub struct Optimizer {
    logical_passes: Vec<Box<dyn LogicalRewritePass>>,
    lowering_passes: Vec<Box<dyn PhysicalLoweringPass>>,
    max_iterations: usize,
}

impl Optimizer {
    pub fn new() -> Self {
        Self {
            logical_passes: Vec::new(),
            lowering_passes: Vec::new(),
            max_iterations: 8,
        }
    }

    pub fn core() -> Self {
        Self::new()
            .add_logical_pass(FlattenBooleanPass)
            .add_logical_pass(FilterLiftPass)
            .add_logical_pass(ProjectCollapsePass)
            .add_logical_pass(JoinPredicatePushdownPass)
            .add_lowering_pass(CoreLoweringPass)
    }

    pub fn with_max_iterations(mut self, max_iterations: usize) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    pub fn add_logical_pass(mut self, pass: impl LogicalRewritePass + 'static) -> Self {
        self.logical_passes.push(Box::new(pass));
        self
    }

    pub fn add_lowering_pass(mut self, pass: impl PhysicalLoweringPass + 'static) -> Self {
        self.lowering_passes.push(Box::new(pass));
        self
    }

    pub fn clear_logical_passes(mut self) -> Self {
        self.logical_passes.clear();
        self
    }

    pub fn clear_lowering_passes(mut self) -> Self {
        self.lowering_passes.clear();
        self
    }

    pub fn optimize_query(
        &self,
        query: &SelectQuery,
        source: Option<String>,
        stats: Option<&dyn StatsProvider>,
        context: &QueryContext,
    ) -> PlanPair {
        let collection_id = source
            .as_ref()
            .filter(|name| !crate::is_all_collection_alias(name))
            .and_then(|name| context.catalog().collection_by_name(name))
            .map(|c| c.lid);
        let source_ref =
            source_ref_for_collection(source, query.source_alias.clone(), collection_id);
        self.optimize_query_with_source(query, source_ref, stats, context)
    }

    pub fn optimize_query_with_source(
        &self,
        query: &SelectQuery,
        source: SourceRef,
        stats: Option<&dyn StatsProvider>,
        context: &QueryContext,
    ) -> PlanPair {
        let logical = build_logical_plan(query, source);
        let logical = self.optimize_logical(logical, context);
        let physical = self.lower_to_physical(&logical, stats, context);
        PlanPair { logical, physical }
    }

    pub fn optimize_logical(
        &self,
        mut logical: LogicalPlan,
        context: &QueryContext,
    ) -> LogicalPlan {
        let mut iteration = 0usize;
        loop {
            let mut changed = false;
            for pass in &self.logical_passes {
                let next = pass.rewrite(logical.clone(), context);
                if next != logical {
                    logical = next;
                    changed = true;
                }
            }
            iteration += 1;
            if !changed || iteration >= self.max_iterations {
                return logical;
            }
        }
    }

    pub fn lower_to_physical(
        &self,
        logical: &LogicalPlan,
        stats: Option<&dyn StatsProvider>,
        context: &QueryContext,
    ) -> PhysicalPlan {
        fn recurse(
            optimizer: &Optimizer,
            plan: &LogicalPlan,
            stats: Option<&dyn StatsProvider>,
            context: &QueryContext,
        ) -> PhysicalPlan {
            for pass in &optimizer.lowering_passes {
                if let Some(lowered) = pass.lower(plan, context, stats, &|inner| {
                    recurse(optimizer, inner, stats, context)
                }) {
                    return lowered;
                }
            }
            panic!("no lowering pass handled logical plan node")
        }

        recurse(self, logical, stats, context)
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self::core()
    }
}

#[derive(Debug, Clone)]
pub struct FlattenBooleanPass;

impl LogicalRewritePass for FlattenBooleanPass {
    fn name(&self) -> &'static str {
        "flatten_boolean"
    }

    fn rewrite(&self, plan: LogicalPlan, _context: &QueryContext) -> LogicalPlan {
        rewrite_plan(plan, &|node| match node {
            LogicalPlan::Filter { input, predicate } => LogicalPlan::Filter {
                input,
                predicate: flatten_boolean_expr(predicate),
            },
            other => other,
        })
    }
}

#[derive(Debug, Clone)]
pub struct FilterLiftPass;

impl LogicalRewritePass for FilterLiftPass {
    fn name(&self) -> &'static str {
        "filter_lift"
    }

    fn rewrite(&self, plan: LogicalPlan, _context: &QueryContext) -> LogicalPlan {
        rewrite_plan(plan, &|node| match node {
            LogicalPlan::Filter { input, predicate } => match *input {
                LogicalPlan::Filter {
                    input: nested_input,
                    predicate: nested_predicate,
                } => LogicalPlan::Filter {
                    input: nested_input,
                    predicate: Expr::Binary {
                        op: BinaryOp::And,
                        left: Box::new(nested_predicate),
                        right: Box::new(predicate),
                    },
                },
                LogicalPlan::Source {
                    source,
                    pushed_predicate: None,
                } => LogicalPlan::Source {
                    source,
                    pushed_predicate: Some(predicate),
                },
                other => LogicalPlan::Filter {
                    input: Box::new(other),
                    predicate,
                },
            },
            other => other,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProjectCollapsePass;

impl LogicalRewritePass for ProjectCollapsePass {
    fn name(&self) -> &'static str {
        "project_collapse"
    }

    fn rewrite(&self, plan: LogicalPlan, _context: &QueryContext) -> LogicalPlan {
        rewrite_plan(plan, &|node| match node {
            LogicalPlan::Project { input, projection } => match *input {
                LogicalPlan::Project { input, .. } => LogicalPlan::Project { input, projection },
                other => LogicalPlan::Project {
                    input: Box::new(other),
                    projection,
                },
            },
            other => other,
        })
    }
}

#[derive(Debug, Clone)]
pub struct JoinPredicatePushdownPass;

impl LogicalRewritePass for JoinPredicatePushdownPass {
    fn name(&self) -> &'static str {
        "join_predicate_pushdown"
    }

    fn rewrite(&self, plan: LogicalPlan, _context: &QueryContext) -> LogicalPlan {
        rewrite_plan(plan, &|node| match node {
            LogicalPlan::Join(mut join) => {
                if let LogicalJoinCondition::Predicate(predicate) = &join.condition {
                    join.condition =
                        LogicalJoinCondition::Predicate(flatten_boolean_expr(predicate.clone()));
                }
                LogicalPlan::Join(join)
            }
            other => other,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CoreLoweringPass;

impl PhysicalLoweringPass for CoreLoweringPass {
    fn name(&self) -> &'static str {
        "core_lowering"
    }

    fn lower(
        &self,
        plan: &LogicalPlan,
        context: &QueryContext,
        stats: Option<&dyn StatsProvider>,
        input: &dyn Fn(&LogicalPlan) -> PhysicalPlan,
    ) -> Option<PhysicalPlan> {
        let lowered = match plan {
            LogicalPlan::Source {
                source,
                pushed_predicate: Some(predicate),
            } => choose_scan_source(source.clone(), predicate.clone(), stats, context),
            LogicalPlan::Source {
                source,
                pushed_predicate: None,
            } => PhysicalPlan::Source(PhysicalSource::Scan {
                source: source.clone(),
            }),
            LogicalPlan::Values { values } => PhysicalPlan::Values {
                values: values.clone(),
            },
            LogicalPlan::Filter {
                input: inner,
                predicate,
            } => PhysicalPlan::Filter {
                input: Box::new(input(inner)),
                predicate: predicate.clone(),
            },
            LogicalPlan::Sort {
                input: inner,
                order_by,
            } => PhysicalPlan::Sort {
                input: Box::new(input(inner)),
                order_by: order_by
                    .iter()
                    .map(|item| PhysicalOrderField {
                        expr: item.expr.clone(),
                        direction: item.direction,
                    })
                    .collect(),
            },
            LogicalPlan::Project {
                input: inner,
                projection,
            } => PhysicalPlan::Project {
                input: Box::new(input(inner)),
                projection: projection
                    .iter()
                    .map(|item| to_projection_field(item, context))
                    .collect(),
            },
            LogicalPlan::Aggregate {
                input: inner,
                group_by,
                projection,
                having,
            } => PhysicalPlan::Aggregate {
                input: Box::new(input(inner)),
                group_by: group_by.clone(),
                projection: projection
                    .iter()
                    .map(|item| to_projection_field(item, context))
                    .collect(),
                having: having.clone(),
            },
            LogicalPlan::Limit {
                input: inner,
                offset,
                limit,
            } => PhysicalPlan::Limit {
                input: Box::new(input(inner)),
                offset: offset.clone(),
                limit: limit.clone(),
            },
            LogicalPlan::Distinct { input: inner } => PhysicalPlan::Distinct {
                input: Box::new(input(inner)),
            },
            LogicalPlan::Union { inputs, all } => PhysicalPlan::Union {
                inputs: inputs.iter().map(input).collect(),
                all: *all,
            },
            LogicalPlan::Join(join) => {
                let condition = lower_join_condition(join, context);
                let algorithm = choose_join_algorithm(&condition, stats);
                PhysicalPlan::Join(PhysicalJoinPlan {
                    left: Box::new(input(&join.left)),
                    right: Box::new(input(&join.right)),
                    join_type: join.join_type,
                    algorithm,
                    condition,
                    left_binding: join.left_binding.clone(),
                    right_binding: join.right_binding.clone(),
                })
            }
            LogicalPlan::ApplyExists {
                input: inner,
                subquery,
                negated,
            } => PhysicalPlan::ApplyExists {
                input: Box::new(input(inner)),
                subquery: Box::new(input(subquery)),
                negated: *negated,
            },
            LogicalPlan::ApplyInSubquery {
                input: inner,
                left,
                subquery,
                negated,
            } => PhysicalPlan::ApplyInSubquery {
                input: Box::new(input(inner)),
                left: left.clone(),
                subquery: Box::new(input(subquery)),
                negated: *negated,
            },
            LogicalPlan::Exchange {
                input: inner,
                partition_count,
            } => PhysicalPlan::Exchange {
                input: Box::new(input(inner)),
                partition_count: *partition_count,
            },
            LogicalPlan::RepartitionHash {
                input: inner,
                partition_count,
                partition_keys,
            } => PhysicalPlan::RepartitionHash {
                input: Box::new(input(inner)),
                partition_count: *partition_count,
                partition_keys: partition_keys
                    .iter()
                    .cloned()
                    .map(|source_path| PhysicalJoinKey {
                        field: FieldRef::Path(source_path.clone()),
                        source_path,
                    })
                    .collect(),
            },
        };
        Some(lowered)
    }
}

fn to_projection_field(field: &QueryField, context: &QueryContext) -> PhysicalProjectionField {
    let (field_ref, source_path) = match field.expr.as_ref() {
        crate::query::Expr::Operand(crate::query::Operand::Field(path)) => (
            Some(resolve_field_ref_for_path(context, path)),
            Some(path.clone()),
        ),
        _ => (None, None),
    };
    PhysicalProjectionField {
        expr: (*field.expr).clone(),
        field: field_ref,
        source_path,
        alias: field.alias.clone(),
    }
}

fn choose_join_algorithm(
    condition: &PhysicalJoinCondition,
    _stats: Option<&dyn StatsProvider>,
) -> PhysicalJoinAlgorithm {
    match condition {
        PhysicalJoinCondition::Eq { .. } => PhysicalJoinAlgorithm::Hash,
        _ => PhysicalJoinAlgorithm::NestedLoop,
    }
}

fn lower_join_condition(join: &LogicalJoinPlan, context: &QueryContext) -> PhysicalJoinCondition {
    match &join.condition {
        LogicalJoinCondition::True => PhysicalJoinCondition::True,
        LogicalJoinCondition::Predicate(predicate) => {
            PhysicalJoinCondition::Predicate(predicate.clone())
        }
        LogicalJoinCondition::UsingFields { left, right } => PhysicalJoinCondition::Eq {
            left: PhysicalJoinKey {
                field: resolve_field_ref_for_path(context, left),
                source_path: left.clone(),
            },
            right: PhysicalJoinKey {
                field: resolve_field_ref_for_path(context, right),
                source_path: right.clone(),
            },
        },
    }
}

fn choose_scan_source(
    source: SourceRef,
    predicate: Expr,
    stats: Option<&dyn StatsProvider>,
    context: &QueryContext,
) -> PhysicalPlan {
    if expr_contains_relationship_expr(&predicate) {
        return PhysicalPlan::Source(PhysicalSource::FilteredScan { source, predicate });
    }
    let Some((field_path, value)) = extract_equality_lookup(&predicate) else {
        return PhysicalPlan::Source(PhysicalSource::FilteredScan { source, predicate });
    };

    let field_ref = resolve_field_ref_for_source(context, &source, &field_path);

    let use_index = stats
        .and_then(|s| s.has_equality_index(&source, &field_ref))
        .unwrap_or(false);

    let filtered_scan_cost = estimate_filtered_scan_cost(stats, &source, &field_ref);
    let index_cost = estimate_index_lookup_cost(stats, &source, &field_ref);

    if use_index && (index_cost < filtered_scan_cost || filtered_scan_cost <= 5.0) {
        let residual = remove_single_lookup_predicate(predicate.clone(), &field_path, &value);
        return PhysicalPlan::Source(PhysicalSource::IndexLookup {
            source,
            field: field_ref,
            value,
            residual_predicate: residual,
        });
    }

    PhysicalPlan::Source(PhysicalSource::FilteredScan { source, predicate })
}

fn estimate_filtered_scan_cost(
    stats: Option<&dyn StatsProvider>,
    source: &SourceRef,
    field: &FieldRef,
) -> f64 {
    let Some(stats) = stats else {
        return 1_000.0;
    };

    let rows = stats
        .relation_stats(source)
        .map(|s| s.row_count)
        .unwrap_or(10_000.0)
        .max(1.0);

    let sel = estimate_equality_selectivity(stats, source, field).clamp(0.0001, 1.0);
    rows * (0.2 + sel)
}

fn estimate_index_lookup_cost(
    stats: Option<&dyn StatsProvider>,
    source: &SourceRef,
    field: &FieldRef,
) -> f64 {
    let Some(stats) = stats else {
        return 600.0;
    };

    let rows = stats
        .relation_stats(source)
        .map(|s| s.row_count)
        .unwrap_or(10_000.0)
        .max(1.0);
    let sel = estimate_equality_selectivity(stats, source, field).clamp(0.0001, 1.0);
    (rows * sel) + (rows.log10().max(1.0) * 20.0)
}

fn estimate_equality_selectivity(
    stats: &dyn StatsProvider,
    source: &SourceRef,
    field: &FieldRef,
) -> f64 {
    let Some(col) = stats.field_stats(source, field) else {
        return 0.1;
    };

    let null_fraction = col.null_fraction.unwrap_or(0.0).clamp(0.0, 1.0);
    if let Some(distinct) = col.distinct_count {
        if distinct > 0.0 {
            return ((1.0 - null_fraction) / distinct).clamp(0.0001, 1.0);
        }
    }

    0.1
}

fn resolve_collection_schema<'a>(
    context: &'a QueryContext,
    source: &SourceRef,
) -> Option<&'a CollectionSchema> {
    if let Some(collection_id) = source.collection_id {
        return context.catalog().collection_by_lid(collection_id);
    }
    source
        .source_name
        .as_deref()
        .and_then(|name| context.catalog().collection_by_name(name))
}

fn resolve_field_ref_for_source(
    context: &QueryContext,
    source: &SourceRef,
    path: &FieldPath,
) -> FieldRef {
    let Some(schema) = resolve_collection_schema(context, source) else {
        return FieldRef::Path(path.clone());
    };
    resolve_field_ref_for_schema(schema, path)
}

fn resolve_field_ref_for_path(context: &QueryContext, path: &FieldPath) -> FieldRef {
    let Some(PathSegment::Field(first)) = path.segments().first() else {
        return FieldRef::Path(path.clone());
    };
    let Some(collection) = context.catalog().collection_by_name(first) else {
        return FieldRef::Path(path.clone());
    };
    if path.segments().len() == 1 {
        return FieldRef::Path(path.clone());
    }
    let mut relative = FieldPath::new();
    for seg in path.segments().iter().skip(1) {
        relative.0.push(seg.clone());
    }
    resolve_field_ref_for_schema(collection, &relative)
}

fn resolve_field_ref_for_schema(schema: &CollectionSchema, path: &FieldPath) -> FieldRef {
    let Some(PathSegment::Field(first)) = path.segments().first() else {
        return FieldRef::Path(path.clone());
    };
    if path.segments().len() > 1 {
        return FieldRef::Path(path.clone());
    }
    if let Some(field_id) = schema.field_id(first) {
        if let Some(attr_id) = schema.attr_for_field_id(field_id) {
            return FieldRef::AttrId(attr_id);
        }
        return FieldRef::FieldId(field_id);
    }
    let canonical = schema.canonical_field_name(first).to_string();
    FieldRef::CanonicalName(canonical)
}

fn rewrite_plan(plan: LogicalPlan, f: &dyn Fn(LogicalPlan) -> LogicalPlan) -> LogicalPlan {
    let rewritten_children = match plan {
        LogicalPlan::Filter { input, predicate } => LogicalPlan::Filter {
            input: Box::new(rewrite_plan(*input, f)),
            predicate,
        },
        LogicalPlan::Sort { input, order_by } => LogicalPlan::Sort {
            input: Box::new(rewrite_plan(*input, f)),
            order_by,
        },
        LogicalPlan::Project { input, projection } => LogicalPlan::Project {
            input: Box::new(rewrite_plan(*input, f)),
            projection,
        },
        LogicalPlan::Aggregate {
            input,
            group_by,
            projection,
            having,
        } => LogicalPlan::Aggregate {
            input: Box::new(rewrite_plan(*input, f)),
            group_by,
            projection,
            having,
        },
        LogicalPlan::Limit {
            input,
            offset,
            limit,
        } => LogicalPlan::Limit {
            input: Box::new(rewrite_plan(*input, f)),
            offset,
            limit,
        },
        LogicalPlan::Distinct { input } => LogicalPlan::Distinct {
            input: Box::new(rewrite_plan(*input, f)),
        },
        LogicalPlan::Union { inputs, all } => LogicalPlan::Union {
            inputs: inputs
                .into_iter()
                .map(|input| rewrite_plan(input, f))
                .collect(),
            all,
        },
        LogicalPlan::Join(join) => LogicalPlan::Join(LogicalJoinPlan {
            left: Box::new(rewrite_plan(*join.left, f)),
            right: Box::new(rewrite_plan(*join.right, f)),
            ..join
        }),
        LogicalPlan::ApplyExists {
            input,
            subquery,
            negated,
        } => LogicalPlan::ApplyExists {
            input: Box::new(rewrite_plan(*input, f)),
            subquery: Box::new(rewrite_plan(*subquery, f)),
            negated,
        },
        LogicalPlan::ApplyInSubquery {
            input,
            left,
            subquery,
            negated,
        } => LogicalPlan::ApplyInSubquery {
            input: Box::new(rewrite_plan(*input, f)),
            left,
            subquery: Box::new(rewrite_plan(*subquery, f)),
            negated,
        },
        LogicalPlan::Exchange {
            input,
            partition_count,
        } => LogicalPlan::Exchange {
            input: Box::new(rewrite_plan(*input, f)),
            partition_count,
        },
        LogicalPlan::RepartitionHash {
            input,
            partition_count,
            partition_keys,
        } => LogicalPlan::RepartitionHash {
            input: Box::new(rewrite_plan(*input, f)),
            partition_count,
            partition_keys,
        },
        other => other,
    };
    f(rewritten_children)
}

fn flatten_boolean_expr(expr: Expr) -> Expr {
    match expr {
        Expr::Binary { op, left, right } if op == BinaryOp::And || op == BinaryOp::Or => {
            let mut items = Vec::new();
            collect_binary_terms(*left, op, &mut items);
            collect_binary_terms(*right, op, &mut items);
            let mut iter = items.into_iter();
            let first = iter.next().expect("binary tree has at least one term");
            iter.fold(first, |left, right| Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        Expr::Unary {
            op: semantic_data::query::UnaryOp::Not,
            expr,
        } => Expr::Unary {
            op: semantic_data::query::UnaryOp::Not,
            expr: Box::new(flatten_boolean_expr(*expr)),
        },
        other => other,
    }
}

fn extract_equality_lookup(predicate: &Expr) -> Option<(FieldPath, Value)> {
    match predicate {
        Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } => match (&**left, &**right) {
            (Expr::Operand(Operand::Field(path)), Expr::Operand(Operand::Literal(v)))
            | (Expr::Operand(Operand::Literal(v)), Expr::Operand(Operand::Field(path))) => {
                Some((path.clone(), v.clone()))
            }
            _ => None,
        },
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => extract_equality_lookup(left).or_else(|| extract_equality_lookup(right)),
        _ => None,
    }
}

fn remove_single_lookup_predicate(
    predicate: Expr,
    target_path: &FieldPath,
    target_value: &Value,
) -> Option<Expr> {
    match predicate {
        Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } => {
            let matches_target = match (&*left, &*right) {
                (Expr::Operand(Operand::Field(path)), Expr::Operand(Operand::Literal(value)))
                | (Expr::Operand(Operand::Literal(value)), Expr::Operand(Operand::Field(path))) => {
                    path == target_path && value == target_value
                }
                _ => false,
            };
            if matches_target {
                None
            } else {
                Some(Expr::Binary {
                    op: BinaryOp::Eq,
                    left,
                    right,
                })
            }
        }
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => {
            let left = remove_single_lookup_predicate(*left, target_path, target_value);
            let right = remove_single_lookup_predicate(*right, target_path, target_value);
            match (left, right) {
                (None, None) => None,
                (Some(expr), None) | (None, Some(expr)) => Some(expr),
                (Some(left), Some(right)) => Some(Expr::Binary {
                    op: BinaryOp::And,
                    left: Box::new(left),
                    right: Box::new(right),
                }),
            }
        }
        other => Some(other),
    }
}

fn expr_contains_relationship_expr(expr: &crate::query::Expr) -> bool {
    match expr {
        crate::query::Expr::RelationExists { .. } => true,
        crate::query::Expr::Operand(_) => false,
        crate::query::Expr::Unary { expr, .. } => expr_contains_relationship_expr(expr),
        crate::query::Expr::Binary { left, right, .. } => {
            expr_contains_relationship_expr(left) || expr_contains_relationship_expr(right)
        }
        crate::query::Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_relationship_expr(cond)
                || expr_contains_relationship_expr(then_expr)
                || expr_contains_relationship_expr(else_expr)
        }
        crate::query::Expr::Coalesce(items) => items.iter().any(expr_contains_relationship_expr),
        crate::query::Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            crate::FunctionArg::Expr(expr) => expr_contains_relationship_expr(expr),
            crate::FunctionArg::Wildcard => false,
        }),
        crate::query::Expr::Aggregate { arg, .. } => match arg.as_ref() {
            crate::FunctionArg::Expr(expr) => expr_contains_relationship_expr(expr),
            crate::FunctionArg::Wildcard => false,
        },
        crate::query::Expr::InList { expr, list, .. } => {
            expr_contains_relationship_expr(expr)
                || list.iter().any(expr_contains_relationship_expr)
        }
        crate::query::Expr::Subquery(_) => false,
        crate::query::Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_relationship_expr(expr)
                || expr_contains_relationship_expr(low)
                || expr_contains_relationship_expr(high)
        }
        crate::query::Expr::PatternMatch { expr, pattern, .. }
        | crate::query::Expr::RegexMatch { expr, pattern, .. } => {
            expr_contains_relationship_expr(expr) || expr_contains_relationship_expr(pattern)
        }
        crate::query::Expr::IsNull { expr, .. } => expr_contains_relationship_expr(expr),
        crate::query::Expr::Exists { .. } => false,
    }
}

fn collect_binary_terms(expr: Expr, target_op: BinaryOp, out: &mut Vec<Expr>) {
    match expr {
        Expr::Binary { op, left, right } if op == target_op => {
            collect_binary_terms(*left, target_op, out);
            collect_binary_terms(*right, target_op, out);
        }
        other => out.push(flatten_boolean_expr(other)),
    }
}
