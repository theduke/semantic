use semantic_data::{
    query::BinaryOp,
    query::JoinType,
    schema::core::type_kind::TypeKind,
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
            .add_logical_pass(RefPathJoinLiftPass)
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
pub struct RefPathJoinLiftPass;

impl LogicalRewritePass for RefPathJoinLiftPass {
    fn name(&self) -> &'static str {
        "ref_path_join_lift"
    }

    fn rewrite(&self, plan: LogicalPlan, context: &QueryContext) -> LogicalPlan {
        rewrite_plan(plan, &|node| rewrite_node_with_ref_lift(node, context))
    }
}

fn rewrite_node_with_ref_lift(plan: LogicalPlan, context: &QueryContext) -> LogicalPlan {
    match plan {
        LogicalPlan::Filter {
            mut input,
            mut predicate,
        } => {
            if let Some(mut lifter) = RefPathJoinLifter::new(&input, context) {
                lifter.rewrite_expr(&mut predicate);
                input = Box::new(lifter.into_plan(*input));
            }
            LogicalPlan::Filter { input, predicate }
        }
        LogicalPlan::Sort {
            mut input,
            mut order_by,
        } => {
            if let Some(mut lifter) = RefPathJoinLifter::new(&input, context) {
                for item in &mut order_by {
                    lifter.rewrite_expr(&mut item.expr);
                }
                input = Box::new(lifter.into_plan(*input));
            }
            LogicalPlan::Sort { input, order_by }
        }
        LogicalPlan::Project {
            mut input,
            mut projection,
        } => {
            if let Some(mut lifter) = RefPathJoinLifter::new(&input, context) {
                for field in &mut projection {
                    lifter.rewrite_expr(&mut field.expr);
                }
                input = Box::new(lifter.into_plan(*input));
            }
            LogicalPlan::Project { input, projection }
        }
        LogicalPlan::Aggregate {
            mut input,
            mut group_by,
            mut projection,
            mut having,
        } => {
            if let Some(mut lifter) = RefPathJoinLifter::new(&input, context) {
                for expr in &mut group_by {
                    lifter.rewrite_expr(expr);
                }
                for field in &mut projection {
                    lifter.rewrite_expr(&mut field.expr);
                }
                if let Some(having) = &mut having {
                    lifter.rewrite_expr(having);
                }
                input = Box::new(lifter.into_plan(*input));
            }
            LogicalPlan::Aggregate {
                input,
                group_by,
                projection,
                having,
            }
        }
        other => other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct JoinKey {
    from_binding: String,
    field: String,
}

#[derive(Debug, Clone)]
struct BindingInfo {
    source_name: Option<String>,
    collection_id: Option<crate::catalog::LocalCollectionId>,
}

struct RefPathJoinLifter<'a> {
    context: &'a QueryContext,
    bindings: std::collections::HashMap<String, BindingInfo>,
    join_aliases: std::collections::HashMap<JoinKey, String>,
    pending: Vec<(JoinKey, BindingInfo, Option<String>)>,
    base_binding: String,
    alias_counter: usize,
}

impl<'a> RefPathJoinLifter<'a> {
    fn new(input: &LogicalPlan, context: &'a QueryContext) -> Option<Self> {
        let mut bindings = std::collections::HashMap::new();
        let mut base_binding = None::<String>;
        collect_plan_bindings(input, &mut bindings, &mut base_binding);
        let base_binding = base_binding?;

        let mut join_aliases = std::collections::HashMap::new();
        collect_existing_ref_joins(input, &base_binding, &mut join_aliases);
        let alias_counter = join_aliases
            .values()
            .filter_map(|alias| alias.strip_prefix("__ref_"))
            .filter_map(|suffix| suffix.parse::<usize>().ok())
            .max()
            .map(|value| value.saturating_add(1))
            .unwrap_or(0);

        Some(Self {
            context,
            bindings,
            join_aliases,
            pending: Vec::new(),
            base_binding,
            alias_counter,
        })
    }

    fn rewrite_expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Operand(Operand::Field(path)) => {
                if let Some(rewritten) = self.rewrite_path(path) {
                    *path = rewritten;
                }
            }
            Expr::Unary { expr, .. } => self.rewrite_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.rewrite_expr(left);
                self.rewrite_expr(right);
            }
            Expr::IfElse {
                cond,
                then_expr,
                else_expr,
            } => {
                self.rewrite_expr(cond);
                self.rewrite_expr(then_expr);
                self.rewrite_expr(else_expr);
            }
            Expr::Coalesce(items) => {
                for item in items {
                    self.rewrite_expr(item);
                }
            }
            Expr::Function { args, .. } => {
                for arg in args {
                    if let crate::FunctionArg::Expr(expr) = arg {
                        self.rewrite_expr(expr);
                    }
                }
            }
            Expr::Aggregate { arg, .. } => {
                if let crate::FunctionArg::Expr(expr) = arg.as_mut() {
                    self.rewrite_expr(expr);
                }
            }
            Expr::InList { expr, list, .. } => {
                self.rewrite_expr(expr);
                for item in list {
                    self.rewrite_expr(item);
                }
            }
            Expr::Between {
                expr, low, high, ..
            } => {
                self.rewrite_expr(expr);
                self.rewrite_expr(low);
                self.rewrite_expr(high);
            }
            Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
                self.rewrite_expr(expr);
                self.rewrite_expr(pattern);
            }
            Expr::IsNull { expr, .. } => self.rewrite_expr(expr),
            Expr::RelationExists {
                relation,
                source,
                target,
                max_depth,
                ..
            } => {
                self.rewrite_expr(relation);
                self.rewrite_expr(source);
                self.rewrite_expr(target);
                if let Some(max_depth) = max_depth {
                    self.rewrite_expr(max_depth);
                }
            }
            Expr::Subquery(_) | Expr::Exists { .. } | Expr::Operand(Operand::Literal(_)) => {}
        }
    }

    fn rewrite_path(&mut self, path: &FieldPath) -> Option<FieldPath> {
        let segments = path.segments();
        if segments.len() < 2 {
            return None;
        }

        let mut from_binding = self.base_binding.clone();
        let mut rest_start = 0usize;
        if let Some(PathSegment::Field(first)) = segments.first()
            && self.bindings.contains_key(first)
            && segments.len() >= 3
        {
            from_binding = first.clone();
            rest_start = 1;
        }

        let rest = &segments[rest_start..];
        if rest.len() < 2 {
            return None;
        }

        let mut current_binding = from_binding;
        let mut current_source = self.bindings.get(&current_binding)?.clone();
        let mut consumed = 0usize;

        while consumed + 1 < rest.len() {
            let field = match &rest[consumed] {
                PathSegment::Field(name) => name.clone(),
                PathSegment::Index(_) => break,
            };
            let Some((canonical_field, target_source, target_class)) =
                self.try_ref_target(&current_source, &field)
            else {
                break;
            };
            let key = JoinKey {
                from_binding: current_binding.clone(),
                field: canonical_field,
            };
            let alias = if let Some(existing) = self.join_aliases.get(&key) {
                existing.clone()
            } else {
                let alias = self.next_alias();
                self.join_aliases.insert(key.clone(), alias.clone());
                self.pending
                    .push((key, target_source.clone(), target_class));
                alias
            };
            self.bindings.insert(alias.clone(), target_source.clone());
            current_binding = alias;
            current_source = target_source;
            consumed += 1;
        }

        if consumed == 0 {
            return None;
        }

        let mut rewritten = Vec::new();
        rewritten.push(PathSegment::Field(current_binding));
        if let Some(first) = rest.get(consumed) {
            match first {
                PathSegment::Field(name) => {
                    let canonical =
                        resolve_collection_schema_for_binding(self.context, &current_source)
                            .map(|schema| schema.canonical_field_name(name).to_string())
                            .unwrap_or_else(|| name.clone());
                    rewritten.push(PathSegment::Field(canonical));
                }
                PathSegment::Index(index) => rewritten.push(PathSegment::Index(*index)),
            }
            rewritten.extend(rest[consumed + 1..].iter().cloned());
        }
        Some(FieldPath::from(rewritten))
    }

    fn try_ref_target(
        &self,
        source: &BindingInfo,
        field_name: &str,
    ) -> Option<(String, BindingInfo, Option<String>)> {
        let schema = resolve_collection_schema_for_binding(self.context, source)?;
        let canonical = schema.canonical_field_name(field_name).to_string();
        let field_type = schema.field_type(&canonical)?;
        let TypeKind::Ref(type_ref) = &field_type.kind else {
            return None;
        };

        let mut target = source.clone();
        let target_class = if self.context.catalog().class_id(&type_ref.name).is_some() {
            Some(type_ref.name.clone())
        } else {
            None
        };
        if target.collection_id.is_none()
            && let Some(name) = &target.source_name
        {
            target.collection_id = self
                .context
                .catalog()
                .collection_by_name(name)
                .map(|c| c.lid);
        }
        Some((canonical, target, target_class))
    }

    fn next_alias(&mut self) -> String {
        let alias = format!("__ref_{}", self.alias_counter);
        self.alias_counter += 1;
        alias
    }

    fn into_plan(self, mut input: LogicalPlan) -> LogicalPlan {
        for (join_key, right_source, right_class) in self.pending {
            let right_binding = self
                .join_aliases
                .get(&join_key)
                .cloned()
                .unwrap_or_else(|| "__ref_fallback".to_string());
            let right = LogicalPlan::Source {
                source: SourceRef {
                    source_name: right_source.source_name.clone(),
                    collection_id: right_source.collection_id,
                    binding: Some(right_binding.clone()),
                    backend_tag: None,
                },
                pushed_predicate: right_class.map(|class_name| Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "type",
                    ])))),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String(class_name)))),
                }),
            };

            let left_path = if join_key.from_binding == self.base_binding {
                FieldPath::from_fields([join_key.field.as_str()])
            } else {
                FieldPath::from_fields([join_key.from_binding.as_str(), join_key.field.as_str()])
            };
            let right_path = FieldPath::from_fields(["id"]);
            input = LogicalPlan::Join(LogicalJoinPlan {
                left: Box::new(input),
                right: Box::new(right),
                join_type: JoinType::Left,
                condition: LogicalJoinCondition::UsingFields {
                    left: left_path,
                    right: right_path,
                },
                left_binding: join_key.from_binding,
                right_binding,
            });
        }
        input
    }
}

fn resolve_collection_schema_for_binding<'a>(
    context: &'a QueryContext,
    source: &BindingInfo,
) -> Option<&'a CollectionSchema> {
    if let Some(collection_id) = source.collection_id {
        return context.catalog().collection_by_lid(collection_id);
    }
    source
        .source_name
        .as_deref()
        .and_then(|name| context.catalog().collection_by_name(name))
}

fn collect_plan_bindings(
    plan: &LogicalPlan,
    bindings: &mut std::collections::HashMap<String, BindingInfo>,
    base_binding: &mut Option<String>,
) {
    match plan {
        LogicalPlan::Source { source, .. } => {
            let binding = source
                .binding
                .clone()
                .or_else(|| source.source_name.clone())
                .unwrap_or_else(|| "left".to_string());
            if base_binding.is_none() {
                *base_binding = Some(binding.clone());
            }
            bindings.insert(
                binding,
                BindingInfo {
                    source_name: source.source_name.clone(),
                    collection_id: source.collection_id,
                },
            );
        }
        LogicalPlan::Join(join) => {
            collect_plan_bindings(&join.left, bindings, base_binding);
            if let LogicalPlan::Source { source, .. } = join.right.as_ref() {
                bindings.insert(
                    join.right_binding.clone(),
                    BindingInfo {
                        source_name: source.source_name.clone(),
                        collection_id: source.collection_id,
                    },
                );
            } else {
                collect_plan_bindings(&join.right, bindings, base_binding);
            }
        }
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Project { input, .. }
        | LogicalPlan::Distinct { input }
        | LogicalPlan::Limit { input, .. }
        | LogicalPlan::Exchange { input, .. }
        | LogicalPlan::RepartitionHash { input, .. }
        | LogicalPlan::ApplyExists { input, .. }
        | LogicalPlan::ApplyInSubquery { input, .. }
        | LogicalPlan::Aggregate { input, .. } => {
            collect_plan_bindings(input, bindings, base_binding)
        }
        LogicalPlan::Union { inputs, .. } => {
            if let Some(first) = inputs.first() {
                collect_plan_bindings(first, bindings, base_binding);
            }
        }
        LogicalPlan::Values { .. } => {}
    }
}

fn collect_existing_ref_joins(
    plan: &LogicalPlan,
    base_binding: &str,
    out: &mut std::collections::HashMap<JoinKey, String>,
) {
    match plan {
        LogicalPlan::Join(join) => {
            collect_existing_ref_joins(&join.left, base_binding, out);
            collect_existing_ref_joins(&join.right, base_binding, out);
            let LogicalJoinCondition::UsingFields { left, right } = &join.condition else {
                return;
            };
            let left_segments = left.segments();
            let right_segments = right.segments();
            if right_segments.is_empty() {
                return;
            }
            let (from_binding, field) = match left_segments {
                [PathSegment::Field(field)] => (base_binding.to_string(), field.clone()),
                [PathSegment::Field(from_binding), PathSegment::Field(field)] => {
                    (from_binding.clone(), field.clone())
                }
                _ => return,
            };
            let right_id = match right_segments {
                [PathSegment::Field(field)] => field,
                [PathSegment::Field(_binding), PathSegment::Field(field)] => field,
                _ => return,
            };
            if !is_ref_join_id_field(right_id) {
                return;
            }
            out.insert(
                JoinKey {
                    from_binding,
                    field,
                },
                join.right_binding.clone(),
            );
        }
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Project { input, .. }
        | LogicalPlan::Distinct { input }
        | LogicalPlan::Limit { input, .. }
        | LogicalPlan::Exchange { input, .. }
        | LogicalPlan::RepartitionHash { input, .. }
        | LogicalPlan::ApplyExists { input, .. }
        | LogicalPlan::ApplyInSubquery { input, .. }
        | LogicalPlan::Aggregate { input, .. } => {
            collect_existing_ref_joins(input, base_binding, out)
        }
        LogicalPlan::Union { inputs, .. } => {
            for input in inputs {
                collect_existing_ref_joins(input, base_binding, out);
            }
        }
        LogicalPlan::Source { .. } | LogicalPlan::Values { .. } => {}
    }
}

fn is_ref_join_id_field(field: &str) -> bool {
    field == "id" || field == "semantic:catalog:id"
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use semantic_data::query::BinaryOp;
    use semantic_data::value::{FieldPath, Value};

    use super::*;
    use crate::catalog::{CollectionKind, IntegrityMode};
    use crate::query::{Operand, QueryField, SelectQuery};

    fn context_with_collection(name: &str) -> QueryContext {
        let mut catalog = crate::fresh_catalog_with_core_schema().expect("core schema");
        catalog
            .upsert_collection(name, CollectionKind::Polymorphic, IntegrityMode::Permissive)
            .expect("collection");
        QueryContext::new(Arc::new(catalog))
    }

    fn count_ref_joins(plan: &LogicalPlan) -> usize {
        match plan {
            LogicalPlan::Join(join) => {
                let mut count = usize::from(join.right_binding.starts_with("__ref_"));
                count += count_ref_joins(&join.left);
                count += count_ref_joins(&join.right);
                count
            }
            LogicalPlan::Filter { input, .. }
            | LogicalPlan::Sort { input, .. }
            | LogicalPlan::Project { input, .. }
            | LogicalPlan::Distinct { input }
            | LogicalPlan::Limit { input, .. }
            | LogicalPlan::Aggregate { input, .. }
            | LogicalPlan::Exchange { input, .. }
            | LogicalPlan::RepartitionHash { input, .. }
            | LogicalPlan::ApplyExists { input, .. }
            | LogicalPlan::ApplyInSubquery { input, .. } => count_ref_joins(input),
            LogicalPlan::Union { inputs, .. } => inputs.iter().map(count_ref_joins).sum(),
            LogicalPlan::Source { .. } | LogicalPlan::Values { .. } => 0,
        }
    }

    #[test]
    fn lifts_single_ref_path_in_filter_to_join() {
        let context = context_with_collection("events");
        let query = SelectQuery::new()
            .with_collection("events")
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "parent", "title",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                    "abc".to_string(),
                )))),
            });
        let plan =
            Optimizer::core().optimize_query(&query, Some("events".to_string()), None, &context);
        assert_eq!(count_ref_joins(&plan.logical), 1);
        let LogicalPlan::Filter { predicate, .. } = &plan.logical else {
            panic!("expected filter over lifted join");
        };
        let Expr::Binary { left, .. } = predicate else {
            panic!("expected binary predicate");
        };
        let Expr::Operand(Operand::Field(path)) = left.as_ref() else {
            panic!("expected field operand");
        };
        assert_eq!(path, &FieldPath::from_fields(["__ref_0", "title"]));
    }

    #[test]
    fn lifts_nested_ref_paths_to_multiple_joins() {
        let context = context_with_collection("events");
        let query = SelectQuery::new()
            .with_collection("events")
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "parent", "kind",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                    "blah".to_string(),
                )))),
            })
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "parent", "parent", "id",
                ])))),
                alias: Some("gp".to_string()),
            }]);
        let plan =
            Optimizer::core().optimize_query(&query, Some("events".to_string()), None, &context);
        assert_eq!(count_ref_joins(&plan.logical), 2);

        let LogicalPlan::Project { projection, .. } = &plan.logical else {
            panic!("expected project at root");
        };
        let Expr::Operand(Operand::Field(path)) = projection[0].expr.as_ref() else {
            panic!("expected field projection");
        };
        let canonical_id = context
            .catalog()
            .collection_by_name("events")
            .expect("events collection")
            .canonical_field_name("id");
        assert_eq!(path, &FieldPath::from_fields(["__ref_1", canonical_id]));
    }
}
