use semantic_data::{
    query::BinaryOp,
    query::JoinType,
    schema::core::type_kind::TypeKind,
    value::{FieldPath, PathSegment, Value},
};

use crate::QueryContext;
use crate::catalog::CollectionSchema;
use crate::plan::{
    FieldRef, LogicalJoinCondition, LogicalJoinPlan, LogicalPlan, PhysicalIndexProbe,
    PhysicalJoinAlgorithm, PhysicalJoinCondition, PhysicalJoinKey, PhysicalJoinPlan,
    PhysicalOrderField, PhysicalPlan, PhysicalProjectionField, PhysicalSource, SourceRef,
    StatsProvider, build_logical_plan, source_ref_for_collection,
};
use crate::query::{Expr, Operand, QueryField, SelectQuery, evaluate_usize_expr};

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
                    pushed_predicate,
                } => LogicalPlan::Source {
                    source,
                    pushed_predicate: combine_conjuncts(
                        pushed_predicate
                            .into_iter()
                            .chain(std::iter::once(predicate)),
                    ),
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
        rewrite_plan(plan, &push_join_predicates)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprBinding {
    None,
    Left,
    Right,
    Both,
    Unknown,
}

fn push_join_predicates(node: LogicalPlan) -> LogicalPlan {
    match node {
        LogicalPlan::Filter { input, predicate } => match *input {
            LogicalPlan::Join(mut join) => {
                let (left_bindings, right_bindings) = join_binding_sets(&join);
                let mut left = Vec::new();
                let mut right = Vec::new();
                let mut residual = Vec::new();
                for predicate in top_level_conjuncts(predicate) {
                    match expression_binding(&predicate, &left_bindings, &right_bindings) {
                        ExprBinding::Left
                            if matches!(join.join_type, JoinType::Inner | JoinType::Left) =>
                        {
                            left.push(predicate)
                        }
                        ExprBinding::Right
                            if matches!(join.join_type, JoinType::Inner | JoinType::Right) =>
                        {
                            right.push(predicate)
                        }
                        _ => residual.push(predicate),
                    }
                }
                if let Some(predicate) = combine_conjuncts(left) {
                    join.left = Box::new(push_predicate_to_input(*join.left, predicate));
                }
                if let Some(predicate) = combine_conjuncts(right) {
                    join.right = Box::new(push_predicate_to_input(*join.right, predicate));
                }
                let join = push_on_predicates(join);
                match combine_conjuncts(residual) {
                    Some(predicate) => LogicalPlan::Filter {
                        input: Box::new(LogicalPlan::Join(join)),
                        predicate,
                    },
                    None => LogicalPlan::Join(join),
                }
            }
            other => LogicalPlan::Filter {
                input: Box::new(other),
                predicate,
            },
        },
        LogicalPlan::Join(join) => LogicalPlan::Join(push_on_predicates(join)),
        other => other,
    }
}

fn push_on_predicates(mut join: LogicalJoinPlan) -> LogicalJoinPlan {
    let LogicalJoinCondition::Predicate(predicate) = join.condition.clone() else {
        return join;
    };
    let (left_bindings, right_bindings) = join_binding_sets(&join);
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut residual = Vec::new();
    for predicate in top_level_conjuncts(predicate) {
        match expression_binding(&predicate, &left_bindings, &right_bindings) {
            ExprBinding::Left if matches!(join.join_type, JoinType::Inner | JoinType::Right) => {
                left.push(predicate)
            }
            ExprBinding::Right if matches!(join.join_type, JoinType::Inner | JoinType::Left) => {
                right.push(predicate)
            }
            _ => residual.push(predicate),
        }
    }
    if let Some(predicate) = combine_conjuncts(left) {
        join.left = Box::new(push_predicate_to_input(*join.left, predicate));
    }
    if let Some(predicate) = combine_conjuncts(right) {
        join.right = Box::new(push_predicate_to_input(*join.right, predicate));
    }
    join.condition = combine_conjuncts(residual)
        .map(LogicalJoinCondition::Predicate)
        .unwrap_or(LogicalJoinCondition::True);
    join
}

fn push_predicate_to_input(input: LogicalPlan, mut predicate: Expr) -> LogicalPlan {
    if let LogicalPlan::Source {
        source,
        pushed_predicate,
    } = input
    {
        if let Some(binding) = source.binding.as_deref() {
            strip_direct_binding(&mut predicate, binding);
        }
        return LogicalPlan::Source {
            source,
            pushed_predicate: combine_conjuncts(
                pushed_predicate
                    .into_iter()
                    .chain(std::iter::once(predicate)),
            ),
        };
    }
    LogicalPlan::Filter {
        input: Box::new(input),
        predicate,
    }
}

fn join_binding_sets(
    join: &LogicalJoinPlan,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let mut left = std::collections::HashSet::new();
    let mut right = std::collections::HashSet::new();
    collect_binding_names(&join.left, &mut left);
    collect_binding_names(&join.right, &mut right);
    left.insert(join.left_binding.clone());
    right.insert(join.right_binding.clone());
    (left, right)
}

fn collect_binding_names(plan: &LogicalPlan, out: &mut std::collections::HashSet<String>) {
    match plan {
        LogicalPlan::Source { source, .. } => {
            if let Some(binding) = source.binding.as_ref().or(source.source_name.as_ref()) {
                out.insert(binding.clone());
            }
        }
        LogicalPlan::Join(join) => {
            collect_binding_names(&join.left, out);
            collect_binding_names(&join.right, out);
            out.insert(join.left_binding.clone());
            out.insert(join.right_binding.clone());
        }
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Project { input, .. }
        | LogicalPlan::Aggregate { input, .. }
        | LogicalPlan::Limit { input, .. }
        | LogicalPlan::Distinct { input }
        | LogicalPlan::Exchange { input, .. }
        | LogicalPlan::RepartitionHash { input, .. }
        | LogicalPlan::ApplyExists { input, .. }
        | LogicalPlan::ApplyInSubquery { input, .. } => collect_binding_names(input, out),
        LogicalPlan::Union { inputs, .. } => {
            for input in inputs {
                collect_binding_names(input, out);
            }
        }
        LogicalPlan::Values { .. } => {}
    }
}

fn expression_binding(
    expr: &Expr,
    left_bindings: &std::collections::HashSet<String>,
    right_bindings: &std::collections::HashSet<String>,
) -> ExprBinding {
    fn merge(left: ExprBinding, right: ExprBinding) -> ExprBinding {
        match (left, right) {
            (ExprBinding::Unknown, _) | (_, ExprBinding::Unknown) => ExprBinding::Unknown,
            (ExprBinding::None, other) | (other, ExprBinding::None) => other,
            (ExprBinding::Left, ExprBinding::Left) => ExprBinding::Left,
            (ExprBinding::Right, ExprBinding::Right) => ExprBinding::Right,
            _ => ExprBinding::Both,
        }
    }
    fn path_binding(
        path: &FieldPath,
        left: &std::collections::HashSet<String>,
        right: &std::collections::HashSet<String>,
    ) -> ExprBinding {
        let Some(PathSegment::Field(first)) = path.segments().first() else {
            return ExprBinding::Left;
        };
        let in_left = left.contains(first);
        let in_right = right.contains(first);
        match (in_left, in_right) {
            (true, false) => ExprBinding::Left,
            (false, true) => ExprBinding::Right,
            (true, true) => ExprBinding::Unknown,
            (false, false) => ExprBinding::Left,
        }
    }
    fn recurse(
        expr: &Expr,
        left: &std::collections::HashSet<String>,
        right: &std::collections::HashSet<String>,
    ) -> ExprBinding {
        match expr {
            Expr::Operand(Operand::Literal(_)) => ExprBinding::None,
            Expr::Operand(Operand::Field(path)) => path_binding(path, left, right),
            Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => recurse(expr, left, right),
            Expr::Binary {
                left: a, right: b, ..
            }
            | Expr::PatternMatch {
                expr: a,
                pattern: b,
                ..
            }
            | Expr::RegexMatch {
                expr: a,
                pattern: b,
                ..
            } => merge(recurse(a, left, right), recurse(b, left, right)),
            Expr::IfElse {
                cond,
                then_expr,
                else_expr,
            } => merge(
                recurse(cond, left, right),
                merge(
                    recurse(then_expr, left, right),
                    recurse(else_expr, left, right),
                ),
            ),
            Expr::Coalesce(items) => items.iter().fold(ExprBinding::None, |binding, item| {
                merge(binding, recurse(item, left, right))
            }),
            Expr::Function { args, .. } => {
                args.iter()
                    .fold(ExprBinding::None, |binding, arg| match arg {
                        crate::FunctionArg::Expr(expr) => {
                            merge(binding, recurse(expr, left, right))
                        }
                        crate::FunctionArg::Wildcard => ExprBinding::Unknown,
                    })
            }
            Expr::Aggregate { .. } | Expr::Subquery(_) | Expr::Exists { .. } => {
                ExprBinding::Unknown
            }
            Expr::InList { expr, list, .. } => list
                .iter()
                .fold(recurse(expr, left, right), |binding, item| {
                    merge(binding, recurse(item, left, right))
                }),
            Expr::Between {
                expr, low, high, ..
            } => merge(
                recurse(expr, left, right),
                merge(recurse(low, left, right), recurse(high, left, right)),
            ),
            Expr::RelationExists {
                relation,
                source,
                target,
                max_depth,
                ..
            } => {
                let mut binding = merge(
                    recurse(relation, left, right),
                    merge(recurse(source, left, right), recurse(target, left, right)),
                );
                if let Some(max_depth) = max_depth {
                    binding = merge(binding, recurse(max_depth, left, right));
                }
                binding
            }
        }
    }
    recurse(expr, left_bindings, right_bindings)
}

fn strip_direct_binding(expr: &mut Expr, binding: &str) {
    fn strip_path(path: &mut FieldPath, binding: &str) {
        if matches!(path.segments().first(), Some(PathSegment::Field(first)) if first == binding)
            && path.segments().len() > 1
        {
            path.0.remove(0);
        }
    }
    match expr {
        Expr::Operand(Operand::Field(path)) => strip_path(path, binding),
        Expr::Operand(Operand::Literal(_)) | Expr::Subquery(_) | Expr::Exists { .. } => {}
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => strip_direct_binding(expr, binding),
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
            strip_direct_binding(left, binding);
            strip_direct_binding(right, binding);
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            strip_direct_binding(cond, binding);
            strip_direct_binding(then_expr, binding);
            strip_direct_binding(else_expr, binding);
        }
        Expr::Coalesce(items) => {
            for item in items {
                strip_direct_binding(item, binding);
            }
        }
        Expr::Function { args, .. } => {
            for arg in args {
                if let crate::FunctionArg::Expr(expr) = arg {
                    strip_direct_binding(expr, binding);
                }
            }
        }
        Expr::Aggregate { arg, .. } => {
            if let crate::FunctionArg::Expr(expr) = arg.as_mut() {
                strip_direct_binding(expr, binding);
            }
        }
        Expr::InList { expr, list, .. } => {
            strip_direct_binding(expr, binding);
            for item in list {
                strip_direct_binding(item, binding);
            }
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            strip_direct_binding(expr, binding);
            strip_direct_binding(low, binding);
            strip_direct_binding(high, binding);
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            strip_direct_binding(relation, binding);
            strip_direct_binding(source, binding);
            strip_direct_binding(target, binding);
            if let Some(max_depth) = max_depth {
                strip_direct_binding(max_depth, binding);
            }
        }
    }
}

fn top_level_conjuncts(expr: Expr) -> Vec<Expr> {
    let mut out = Vec::new();
    collect_binary_terms(expr, BinaryOp::And, &mut out);
    out
}

fn combine_conjuncts(items: impl IntoIterator<Item = Expr>) -> Option<Expr> {
    let mut items = items.into_iter();
    let first = items.next()?;
    Some(items.fold(first, |left, right| Expr::Binary {
        op: BinaryOp::And,
        left: Box::new(left),
        right: Box::new(right),
    }))
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
                let index_probe = choose_index_join_probe(join, &condition, stats, context);
                let algorithm = index_probe
                    .as_ref()
                    .map(|_| PhysicalJoinAlgorithm::IndexNestedLoop)
                    .unwrap_or_else(|| choose_join_algorithm(&condition, stats));
                PhysicalPlan::Join(PhysicalJoinPlan {
                    left: Box::new(input(&join.left)),
                    right: Box::new(input(&join.right)),
                    join_type: join.join_type,
                    algorithm,
                    condition,
                    index_probe,
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

fn choose_index_join_probe(
    join: &LogicalJoinPlan,
    condition: &PhysicalJoinCondition,
    stats: Option<&dyn StatsProvider>,
    context: &QueryContext,
) -> Option<PhysicalIndexProbe> {
    if !matches!(join.join_type, JoinType::Inner | JoinType::Left) {
        return None;
    }
    let PhysicalJoinCondition::Eq { right, .. } = condition else {
        return None;
    };
    let LogicalPlan::Source {
        source,
        pushed_predicate,
    } = join.right.as_ref()
    else {
        return None;
    };
    let stats = stats?;
    let field = resolve_field_ref_for_source(context, source, &right.source_path);
    if stats.has_equality_index(source, &field) != Some(true) {
        return None;
    }
    let left_rows = estimate_logical_rows(&join.left, stats, context)?;
    let right_rows = stats.relation_stats(source)?.row_count.max(1.0);
    if left_rows > right_rows {
        return None;
    }
    Some(PhysicalIndexProbe {
        source: source.clone(),
        field,
        residual_predicate: pushed_predicate.clone(),
    })
}

fn estimate_logical_rows(
    plan: &LogicalPlan,
    stats: &dyn StatsProvider,
    context: &QueryContext,
) -> Option<f64> {
    match plan {
        LogicalPlan::Source {
            source,
            pushed_predicate,
        } => {
            let rows = stats.relation_stats(source)?.row_count.max(1.0);
            let Some(predicate) = pushed_predicate else {
                return Some(rows);
            };
            let Some((path, _)) = extract_equality_lookup(predicate) else {
                return Some(rows);
            };
            let field = resolve_field_ref_for_source(context, source, &path);
            Some(rows * estimate_equality_selectivity(stats, source, &field))
        }
        LogicalPlan::Filter { input, .. } => {
            estimate_logical_rows(input, stats, context).map(|rows| rows * 0.25)
        }
        LogicalPlan::Limit { input, limit, .. } => {
            let rows = estimate_logical_rows(input, stats, context)?;
            Some(evaluate_usize_expr(limit.as_ref()?).map_or(rows, |limit| rows.min(limit as f64)))
        }
        LogicalPlan::Sort { input, .. }
        | LogicalPlan::Project { input, .. }
        | LogicalPlan::Distinct { input }
        | LogicalPlan::Exchange { input, .. }
        | LogicalPlan::RepartitionHash { input, .. }
        | LogicalPlan::ApplyExists { input, .. }
        | LogicalPlan::ApplyInSubquery { input, .. } => {
            estimate_logical_rows(input, stats, context)
        }
        LogicalPlan::Values { values } => Some(values.len() as f64),
        LogicalPlan::Aggregate { .. } | LogicalPlan::Union { .. } | LogicalPlan::Join(_) => None,
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
        wildcard: field.wildcard.clone(),
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
            try_lower_equi_join_predicate(join, predicate, context)
                .unwrap_or_else(|| PhysicalJoinCondition::Predicate(predicate.clone()))
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
            residual_predicate: None,
        },
    }
}

fn try_lower_equi_join_predicate(
    join: &LogicalJoinPlan,
    predicate: &Expr,
    context: &QueryContext,
) -> Option<PhysicalJoinCondition> {
    let conjuncts = top_level_conjuncts(predicate.clone());
    let (index, left, right) = conjuncts
        .iter()
        .enumerate()
        .find_map(|(index, predicate)| {
            let Expr::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } = predicate
            else {
                return None;
            };
            let Expr::Operand(Operand::Field(left)) = left.as_ref() else {
                return None;
            };
            let Expr::Operand(Operand::Field(right)) = right.as_ref() else {
                return None;
            };
            orient_equi_join_paths(join, left, right)
                .or_else(|| orient_equi_join_paths(join, right, left))
                .map(|(left, right)| (index, left, right))
        })?;
    let residual_predicate = combine_conjuncts(
        conjuncts
            .into_iter()
            .enumerate()
            .filter_map(|(candidate, predicate)| (candidate != index).then_some(predicate)),
    );
    Some(PhysicalJoinCondition::Eq {
        left: PhysicalJoinKey {
            field: resolve_field_ref_for_path(context, &left),
            source_path: left,
        },
        right: PhysicalJoinKey {
            field: resolve_field_ref_for_path(context, &right),
            source_path: right,
        },
        residual_predicate,
    })
}

fn orient_equi_join_paths(
    join: &LogicalJoinPlan,
    left: &FieldPath,
    right: &FieldPath,
) -> Option<(FieldPath, FieldPath)> {
    let (left_binding, left_tail, left_qualified) = match split_qualified_path(left) {
        Some((binding, tail)) if logical_plan_has_binding(&join.left, binding) => {
            (binding, tail, true)
        }
        // A path explicitly qualified by the right input cannot be treated as an
        // unqualified path from the left input. Doing so would turn predicates
        // such as `right.a = right.b` into cross-input equality joins.
        Some((binding, _)) if binding == join.right_binding => return None,
        _ => (join.left_binding.as_str(), left.clone(), false),
    };
    let (right_binding, right_tail) = split_qualified_path(right)?;
    if right_binding != join.right_binding
        || !logical_plan_has_binding(&join.left, left_binding)
        || logical_plan_has_binding(&join.left, right_binding)
    {
        return None;
    }

    let left_path = if left_qualified && left_binding != join.left_binding {
        left.clone()
    } else {
        left_tail
    };
    Some((left_path, right_tail))
}

fn split_qualified_path(path: &FieldPath) -> Option<(&str, FieldPath)> {
    let [PathSegment::Field(binding), tail @ ..] = path.segments() else {
        return None;
    };
    if tail.is_empty() {
        return None;
    }
    Some((binding, tail.to_vec().into()))
}

fn logical_plan_has_binding(plan: &LogicalPlan, binding: &str) -> bool {
    match plan {
        LogicalPlan::Source { source, .. } => {
            source.binding.as_deref().or(source.source_name.as_deref()) == Some(binding)
        }
        LogicalPlan::Join(join) => {
            join.left_binding == binding
                || join.right_binding == binding
                || logical_plan_has_binding(&join.left, binding)
                || logical_plan_has_binding(&join.right, binding)
        }
        LogicalPlan::Filter { input, .. }
        | LogicalPlan::Sort { input, .. }
        | LogicalPlan::Project { input, .. }
        | LogicalPlan::Aggregate { input, .. }
        | LogicalPlan::Limit { input, .. }
        | LogicalPlan::Distinct { input }
        | LogicalPlan::ApplyExists { input, .. }
        | LogicalPlan::ApplyInSubquery { input, .. }
        | LogicalPlan::Exchange { input, .. }
        | LogicalPlan::RepartitionHash { input, .. } => logical_plan_has_binding(input, binding),
        LogicalPlan::Union { inputs, .. } => inputs
            .iter()
            .any(|input| logical_plan_has_binding(input, binding)),
        LogicalPlan::Values { .. } => false,
    }
}

fn choose_scan_source(
    source: SourceRef,
    predicate: Expr,
    stats: Option<&dyn StatsProvider>,
    context: &QueryContext,
) -> PhysicalPlan {
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
        Expr::InList {
            expr,
            mut list,
            negated: false,
        } if list.len() == 1 => Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(flatten_boolean_expr(*expr)),
            right: Box::new(flatten_boolean_expr(
                list.pop().expect("singleton list has one item"),
            )),
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

    fn bound_source(name: &str, binding: &str) -> LogicalPlan {
        LogicalPlan::Source {
            source: SourceRef {
                source_name: Some(name.to_string()),
                collection_id: None,
                binding: Some(binding.to_string()),
                backend_tag: None,
            },
            pushed_predicate: None,
        }
    }

    fn field(path: impl IntoIterator<Item = &'static str>) -> Expr {
        Expr::Operand(Operand::Field(FieldPath::from_fields(path)))
    }

    fn lower_test_join(predicate: Expr) -> PhysicalJoinPlan {
        let logical = LogicalPlan::Join(LogicalJoinPlan {
            left: Box::new(bound_source("items", "s")),
            right: Box::new(bound_source("artists", "a")),
            join_type: JoinType::Inner,
            condition: LogicalJoinCondition::Predicate(predicate),
            left_binding: "s".to_string(),
            right_binding: "a".to_string(),
        });
        let optimizer = Optimizer::new().add_lowering_pass(CoreLoweringPass);
        let PhysicalPlan::Join(join) =
            optimizer.lower_to_physical(&logical, None, &QueryContext::default())
        else {
            panic!("expected physical join");
        };
        join
    }

    fn eq_literal(path: impl IntoIterator<Item = &'static str>, value: &str) -> Expr {
        Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(path)),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                value.to_string(),
            )))),
        }
    }

    fn test_join(join_type: JoinType, condition: LogicalJoinCondition) -> LogicalJoinPlan {
        LogicalJoinPlan {
            left: Box::new(bound_source("items", "s")),
            right: Box::new(bound_source("artists", "a")),
            join_type,
            condition,
            left_binding: "s".to_string(),
            right_binding: "a".to_string(),
        }
    }

    fn source_predicate(plan: &LogicalPlan) -> Option<&Expr> {
        let LogicalPlan::Source {
            pushed_predicate, ..
        } = plan
        else {
            panic!("expected source")
        };
        pushed_predicate.as_ref()
    }

    #[test]
    fn inner_where_pushes_single_side_conjuncts_and_keeps_cross_side_residual() {
        let left = eq_literal(["s", "kind"], "song");
        let right = eq_literal(["a", "active"], "yes");
        let cross = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["s", "artist_id"])),
            right: Box::new(field(["a", "id"])),
        };
        let predicate =
            combine_conjuncts([left.clone(), right.clone(), cross.clone()]).expect("predicates");
        let plan = JoinPredicatePushdownPass.rewrite(
            LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Join(test_join(
                    JoinType::Inner,
                    LogicalJoinCondition::True,
                ))),
                predicate,
            },
            &QueryContext::default(),
        );

        let LogicalPlan::Filter { input, predicate } = plan else {
            panic!("expected cross-side residual filter")
        };
        assert_eq!(predicate, cross);
        let LogicalPlan::Join(join) = *input else {
            panic!("expected join")
        };
        assert_eq!(
            source_predicate(&join.left),
            Some(&eq_literal(["kind"], "song"))
        );
        assert_eq!(
            source_predicate(&join.right),
            Some(&eq_literal(["active"], "yes"))
        );
    }

    #[test]
    fn outer_where_pushdown_preserves_null_extended_side() {
        for (join_type, left_pushed, right_pushed) in [
            (JoinType::Left, true, false),
            (JoinType::Right, false, true),
            (JoinType::Full, false, false),
        ] {
            let predicate = combine_conjuncts([
                eq_literal(["s", "kind"], "song"),
                eq_literal(["a", "active"], "yes"),
            ])
            .expect("predicates");
            let plan = JoinPredicatePushdownPass.rewrite(
                LogicalPlan::Filter {
                    input: Box::new(LogicalPlan::Join(test_join(
                        join_type,
                        LogicalJoinCondition::True,
                    ))),
                    predicate,
                },
                &QueryContext::default(),
            );
            let (join, has_residual) = match plan {
                LogicalPlan::Filter { input, .. } => match *input {
                    LogicalPlan::Join(join) => (join, true),
                    _ => panic!("expected join"),
                },
                LogicalPlan::Join(join) => (join, false),
                _ => panic!("expected join or filter"),
            };
            assert_eq!(source_predicate(&join.left).is_some(), left_pushed);
            assert_eq!(source_predicate(&join.right).is_some(), right_pushed);
            assert_eq!(has_residual, !(left_pushed && right_pushed));
        }
    }

    #[test]
    fn outer_on_pushdown_only_filters_non_preserved_input() {
        for (join_type, left_pushed, right_pushed) in [
            (JoinType::Inner, true, true),
            (JoinType::Left, false, true),
            (JoinType::Right, true, false),
            (JoinType::Full, false, false),
        ] {
            let condition = combine_conjuncts([
                eq_literal(["s", "kind"], "song"),
                eq_literal(["a", "active"], "yes"),
            ])
            .expect("predicates");
            let LogicalPlan::Join(join) = JoinPredicatePushdownPass.rewrite(
                LogicalPlan::Join(test_join(
                    join_type,
                    LogicalJoinCondition::Predicate(condition),
                )),
                &QueryContext::default(),
            ) else {
                panic!("expected join")
            };
            assert_eq!(source_predicate(&join.left).is_some(), left_pushed);
            assert_eq!(source_predicate(&join.right).is_some(), right_pushed);
            assert_eq!(
                matches!(join.condition, LogicalJoinCondition::True),
                left_pushed && right_pushed
            );
        }
    }

    #[test]
    fn pushdown_combines_existing_predicates_and_does_not_split_or() {
        let mut join = test_join(JoinType::Inner, LogicalJoinCondition::True);
        let LogicalPlan::Source {
            pushed_predicate, ..
        } = join.left.as_mut()
        else {
            panic!("expected source")
        };
        *pushed_predicate = Some(eq_literal(["existing"], "yes"));
        let or_predicate = Expr::Binary {
            op: BinaryOp::Or,
            left: Box::new(eq_literal(["s", "kind"], "song")),
            right: Box::new(eq_literal(["a", "active"], "yes")),
        };
        let plan = JoinPredicatePushdownPass.rewrite(
            LogicalPlan::Filter {
                input: Box::new(LogicalPlan::Join(join)),
                predicate: combine_conjuncts([
                    eq_literal(["s", "new"], "yes"),
                    or_predicate.clone(),
                ])
                .expect("predicates"),
            },
            &QueryContext::default(),
        );
        let LogicalPlan::Filter { input, predicate } = plan else {
            panic!("expected OR residual")
        };
        assert_eq!(predicate, or_predicate);
        let LogicalPlan::Join(join) = *input else {
            panic!("expected join")
        };
        assert!(matches!(
            source_predicate(&join.left),
            Some(Expr::Binary {
                op: BinaryOp::And,
                ..
            })
        ));
    }

    #[test]
    fn singleton_in_normalizes_to_equality() {
        let normalized = flatten_boolean_expr(Expr::InList {
            expr: Box::new(field(["type"])),
            list: vec![Expr::Operand(Operand::Literal(Value::String(
                "directory".to_string(),
            )))],
            negated: false,
        });
        assert!(matches!(
            normalized,
            Expr::Binary {
                op: BinaryOp::Eq,
                ..
            }
        ));
    }

    struct IndexedJoinStats;

    impl StatsProvider for IndexedJoinStats {
        fn relation_stats(&self, _source: &SourceRef) -> Option<crate::plan::RelationStats> {
            Some(crate::plan::RelationStats { row_count: 100.0 })
        }

        fn field_stats(
            &self,
            _source: &SourceRef,
            _field: &FieldRef,
        ) -> Option<crate::plan::FieldStats> {
            Some(crate::plan::FieldStats {
                distinct_count: Some(10.0),
                null_fraction: Some(0.0),
            })
        }

        fn has_equality_index(&self, _source: &SourceRef, _field: &FieldRef) -> Option<bool> {
            Some(true)
        }
    }

    #[test]
    fn lowering_selects_indexed_probe_only_for_supported_join_types() {
        let condition = LogicalJoinCondition::Predicate(Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["s", "artist_id"])),
            right: Box::new(field(["a", "id"])),
        });
        for (join_type, expected) in [
            (JoinType::Inner, PhysicalJoinAlgorithm::IndexNestedLoop),
            (JoinType::Left, PhysicalJoinAlgorithm::IndexNestedLoop),
            (JoinType::Right, PhysicalJoinAlgorithm::Hash),
            (JoinType::Full, PhysicalJoinAlgorithm::Hash),
        ] {
            let mut join = test_join(join_type, condition.clone());
            let LogicalPlan::Source {
                pushed_predicate, ..
            } = join.left.as_mut()
            else {
                panic!("expected source")
            };
            *pushed_predicate = Some(eq_literal(["kind"], "song"));
            let physical = Optimizer::new()
                .add_lowering_pass(CoreLoweringPass)
                .lower_to_physical(
                    &LogicalPlan::Join(join),
                    Some(&IndexedJoinStats),
                    &QueryContext::default(),
                );
            let PhysicalPlan::Join(join) = physical else {
                panic!("expected join")
            };
            assert_eq!(join.algorithm, expected);
            assert_eq!(
                join.index_probe.is_some(),
                expected == PhysicalJoinAlgorithm::IndexNestedLoop
            );
        }
    }

    #[test]
    fn exact_qualified_equi_join_uses_hash_algorithm() {
        let join = lower_test_join(Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["s", "artist_id"])),
            right: Box::new(field(["a", "id"])),
        });

        assert_eq!(join.algorithm, PhysicalJoinAlgorithm::Hash);
        let PhysicalJoinCondition::Eq { left, right, .. } = join.condition else {
            panic!("expected equality join condition");
        };
        assert_eq!(left.source_path, FieldPath::from_fields(["artist_id"]));
        assert_eq!(right.source_path, FieldPath::from_fields(["id"]));
    }

    #[test]
    fn reversed_qualified_equi_join_uses_hash_algorithm() {
        let join = lower_test_join(Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["a", "id"])),
            right: Box::new(field(["s", "artist_id"])),
        });

        assert_eq!(join.algorithm, PhysicalJoinAlgorithm::Hash);
        let PhysicalJoinCondition::Eq { left, right, .. } = join.condition else {
            panic!("expected equality join condition");
        };
        assert_eq!(left.source_path, FieldPath::from_fields(["artist_id"]));
        assert_eq!(right.source_path, FieldPath::from_fields(["id"]));
    }

    #[test]
    fn ambiguous_join_predicates_remain_nested_loops_and_residual_equality_hashes() {
        let ambiguous = lower_test_join(Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["artist_id"])),
            right: Box::new(field(["id"])),
        });
        assert_eq!(ambiguous.algorithm, PhysicalJoinAlgorithm::NestedLoop);
        assert!(matches!(
            ambiguous.condition,
            PhysicalJoinCondition::Predicate(_)
        ));

        let residual = lower_test_join(Expr::Binary {
            op: BinaryOp::And,
            left: Box::new(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(field(["s", "artist_id"])),
                right: Box::new(field(["a", "id"])),
            }),
            right: Box::new(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(field(["a", "active"])),
                right: Box::new(Expr::Operand(Operand::Literal(Value::Bool(true)))),
            }),
        });
        assert_eq!(residual.algorithm, PhysicalJoinAlgorithm::Hash);
        assert!(matches!(
            residual.condition,
            PhysicalJoinCondition::Eq {
                residual_predicate: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn right_only_equality_is_never_lowered_as_a_cross_input_join_key() {
        let right_only = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["a", "parent_id"])),
            right: Box::new(field(["a", "id"])),
        };
        for join_type in [JoinType::Right, JoinType::Full] {
            let logical = LogicalPlan::Join(test_join(
                join_type,
                LogicalJoinCondition::Predicate(right_only.clone()),
            ));
            let PhysicalPlan::Join(join) = Optimizer::new()
                .add_lowering_pass(CoreLoweringPass)
                .lower_to_physical(&logical, None, &QueryContext::default())
            else {
                panic!("expected physical join")
            };
            assert_eq!(join.algorithm, PhysicalJoinAlgorithm::NestedLoop);
            assert!(matches!(
                join.condition,
                PhysicalJoinCondition::Predicate(ref predicate) if predicate == &right_only
            ));
        }
    }

    #[test]
    fn lowering_skips_right_only_equality_and_uses_following_cross_input_key() {
        let right_only = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["a", "parent_id"])),
            right: Box::new(field(["a", "id"])),
        };
        let cross = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field(["s", "artist_id"])),
            right: Box::new(field(["a", "id"])),
        };
        let join = lower_test_join(Expr::Binary {
            op: BinaryOp::And,
            left: Box::new(right_only.clone()),
            right: Box::new(cross),
        });

        assert_eq!(join.algorithm, PhysicalJoinAlgorithm::Hash);
        let PhysicalJoinCondition::Eq {
            left,
            right,
            residual_predicate,
        } = join.condition
        else {
            panic!("expected equality join condition")
        };
        assert_eq!(left.source_path, FieldPath::from_fields(["artist_id"]));
        assert_eq!(right.source_path, FieldPath::from_fields(["id"]));
        assert_eq!(residual_predicate, Some(right_only));
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
                wildcard: None,
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
