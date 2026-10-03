//! Projection resolution shared by SQL lowering and directly submitted ASTs.

use crate::sql::SqlQueryError;
use crate::{
    Expr, FunctionArg, JoinCondition, JoinQuery, Operand, OrderBy as DbOrderBy, Query, QueryField,
    SelectQuery,
};
use semantic_data::value::{FieldPath, PathSegment, Value};

/// Resolve explicit projection references after values have been bound. Ordinary
/// AST expressions retain their existing semantics, including constant ORDER BY.
pub(crate) fn resolve_projection_references(
    query: semantic_data::query::Query,
) -> Result<Query, SqlQueryError> {
    use semantic_data::query as public;
    fn resolve_select(query: &mut public::SelectQuery) -> Result<(), SqlQueryError> {
        if !query
            .group_by
            .iter()
            .any(|expr| matches!(expr, public::Expr::ProjectionRef(_)))
            && !query
                .order_by
                .iter()
                .any(|order| matches!(order.expr, public::Expr::ProjectionRef(_)))
        {
            return Ok(());
        }
        let mut internal: SelectQuery = query.clone().into();
        internal.group_by = internal
            .group_by
            .into_iter()
            .map(|expr| match expr {
                Expr::ProjectionRef(expr) => resolve_group_by_expr(*expr, &internal.projection),
                expr => Ok(expr),
            })
            .collect::<Result<_, _>>()?;
        let bindings = base_group_bindings(
            internal.collection.as_deref(),
            internal.source_alias.as_deref(),
            internal.joins.is_empty(),
        );
        let references = internal
            .order_by
            .iter()
            .filter_map(|order| match &order.expr {
                Expr::ProjectionRef(expr) => Some(DbOrderBy {
                    expr: *expr.clone(),
                    direction: order.direction,
                }),
                _ => None,
            })
            .collect();
        let mut resolved = validate_and_resolve_select_semantics(
            &internal.projection,
            internal.predicate.as_ref(),
            &internal.joins,
            &internal.group_by,
            internal.having.as_ref(),
            internal.limit.as_ref(),
            &internal.offset,
            &bindings,
            references,
        )?
        .into_iter();
        for order in &mut internal.order_by {
            if matches!(order.expr, Expr::ProjectionRef(_)) {
                *order = resolved.next().expect("one resolved order per reference");
            }
        }
        *query = internal.into();
        Ok(())
    }
    let mut public = query;
    public.visit_expressions_mut(&mut |expr| {
        if let public::Expr::Subquery(query) | public::Expr::Exists { query, .. } = expr {
            resolve_select(query)?;
        }
        Ok(())
    })?;
    match &mut public {
        public::Query::Select(query) => resolve_select(query)?,
        public::Query::Insert(public::InsertQuery {
            source: public::InsertSource::Select(query),
            ..
        }) => resolve_select(query)?,
        _ => {}
    }
    public.visit_expressions_mut(&mut |expr| {
        if matches!(expr, public::Expr::ProjectionRef(_)) {
            Err(SqlQueryError::Invalid(
                "projection references are only allowed as root GROUP BY or ORDER BY expressions"
                    .to_string(),
            ))
        } else {
            Ok(())
        }
    })?;
    Ok(public.into())
}

pub(crate) fn resolve_group_by_expr(
    expr: Expr,
    projection: &[QueryField],
) -> Result<Expr, SqlQueryError> {
    let target = if let Some(index) = order_ordinal(&expr) {
        let index = index.checked_sub(1).ok_or_else(|| {
            SqlQueryError::Invalid("GROUP BY ordinal must be at least 1".to_string())
        })?;
        Some(projection.get(index).ok_or_else(|| {
            SqlQueryError::Invalid(format!(
                "GROUP BY ordinal {} exceeds projection length {}",
                index + 1,
                projection.len()
            ))
        })?)
    } else if let Some(alias) = single_field_name(&expr) {
        if let Some(field) = projection
            .iter()
            .find(|field| field.alias.as_deref() == Some(alias))
        {
            if field_expr_final_name(&field.expr) == Some(alias) {
                return Ok(expr);
            }
            return Err(SqlQueryError::Invalid(format!(
                "GROUP BY identifier {alias:?} is ambiguous with a projection alias; use its ordinal or repeat the source expression"
            )));
        }
        None
    } else {
        None
    };
    let Some(target) = target else {
        return Ok(expr);
    };
    if target.wildcard.is_some() {
        return Err(SqlQueryError::Unsupported(
            "GROUP BY cannot reference a wildcard projection".to_string(),
        ));
    }
    if expr_contains_aggregate(&target.expr) {
        return Err(SqlQueryError::Invalid(
            "GROUP BY cannot reference an aggregate projection".to_string(),
        ));
    }
    Ok((*target.expr).clone())
}

pub(crate) fn validate_and_resolve_select_semantics(
    projection: &[QueryField],
    predicate: Option<&Expr>,
    joins: &[JoinQuery],
    group_by: &[Expr],
    having: Option<&Expr>,
    limit: Option<&Expr>,
    offset: &Expr,
    group_bindings: &[String],
    order_by: Vec<DbOrderBy>,
) -> Result<Vec<DbOrderBy>, SqlQueryError> {
    validate_unique_projection_keys(projection)?;

    if predicate.is_some_and(expr_contains_aggregate) {
        return Err(SqlQueryError::Invalid(
            "aggregate expressions are not allowed in WHERE".to_string(),
        ));
    }
    for join in joins {
        let condition_has_aggregate = match &join.condition {
            JoinCondition::OnExpr(expr) => expr_contains_aggregate(expr),
            JoinCondition::UsingFields { .. } => false,
        };
        if condition_has_aggregate || join.predicate.as_ref().is_some_and(expr_contains_aggregate) {
            return Err(SqlQueryError::Invalid(
                "aggregate expressions are not allowed in JOIN conditions".to_string(),
            ));
        }
    }
    if group_by.iter().any(expr_contains_aggregate) {
        return Err(SqlQueryError::Invalid(
            "aggregate expressions are not allowed in GROUP BY".to_string(),
        ));
    }
    if limit.is_some_and(expr_contains_aggregate) || expr_contains_aggregate(offset) {
        return Err(SqlQueryError::Invalid(
            "aggregate expressions are not allowed in LIMIT or OFFSET".to_string(),
        ));
    }

    for expr in projection
        .iter()
        .map(|field| field.expr.as_ref())
        .chain(having)
        .chain(order_by.iter().map(|order| &order.expr))
    {
        if expr_contains_nested_aggregate(expr) {
            return Err(SqlQueryError::Invalid(
                "nested aggregate expressions are not supported".to_string(),
            ));
        }
    }

    let aggregate_query = !group_by.is_empty()
        || having.is_some()
        || projection
            .iter()
            .any(|field| expr_contains_aggregate(&field.expr))
        || order_by
            .iter()
            .any(|order| expr_contains_aggregate(&order.expr));

    if !aggregate_query {
        return order_by
            .into_iter()
            .map(|order| resolve_nonaggregate_order(order, projection))
            .collect();
    }

    if projection.is_empty() || projection.iter().any(|field| field.wildcard.is_some()) {
        return Err(SqlQueryError::Invalid(
            "wildcard projections are not allowed in aggregate queries".to_string(),
        ));
    }
    for field in projection {
        validate_grouped_expr(&field.expr, group_by, group_bindings, "SELECT projection")?;
    }
    if let Some(having) = having {
        validate_grouped_expr(having, group_by, group_bindings, "HAVING")?;
    }

    order_by
        .into_iter()
        .map(|order| resolve_aggregate_order(order, projection, group_by, group_bindings))
        .collect()
}

pub(crate) fn validate_unique_projection_keys(
    projection: &[QueryField],
) -> Result<(), SqlQueryError> {
    let mut keys = std::collections::HashSet::new();
    for field in projection {
        let Some(key) = projection_output_key(field) else {
            continue;
        };
        if !keys.insert(key.clone()) {
            return Err(SqlQueryError::Invalid(format!(
                "duplicate SQL projection output key '{key}'; use distinct aliases"
            )));
        }
    }
    Ok(())
}

pub(crate) fn base_group_bindings(
    collection: Option<&str>,
    source_alias: Option<&str>,
    no_joins: bool,
) -> Vec<String> {
    if !no_joins {
        return Vec::new();
    }
    if let Some(alias) = source_alias {
        return vec![alias.to_string()];
    }
    let Some(collection) = collection else {
        return Vec::new();
    };
    let mut bindings = vec![collection.to_string()];
    if let Some(tail) = collection.rsplit('.').next()
        && tail != collection
    {
        bindings.push(tail.to_string());
    }
    bindings
}

pub(crate) fn projection_output_key(field: &QueryField) -> Option<String> {
    if field.wildcard.is_some() {
        return None;
    }
    if let Some(alias) = &field.alias {
        return Some(alias.clone());
    }
    if let Some(name) = field_expr_final_name(&field.expr) {
        return Some(name.to_string());
    }
    Some("value".to_string())
}

pub(crate) fn field_expr_final_name(expr: &Expr) -> Option<&str> {
    let Expr::Operand(Operand::Field(path)) = expr else {
        return None;
    };
    path.segments()
        .iter()
        .rev()
        .find_map(|segment| match segment {
            PathSegment::Field(name) => Some(name.as_str()),
            _ => None,
        })
}

pub(crate) fn resolve_nonaggregate_order(
    mut order: DbOrderBy,
    projection: &[QueryField],
) -> Result<DbOrderBy, SqlQueryError> {
    if let Some(index) = order_ordinal(&order.expr) {
        if projection.iter().any(|field| field.wildcard.is_some()) {
            return Err(SqlQueryError::Unsupported(
                "ORDER BY ordinals are not supported with wildcard projections".to_string(),
            ));
        }
        let field = projection
            .get(index.checked_sub(1).ok_or_else(|| {
                SqlQueryError::Invalid("ORDER BY ordinal must be at least 1".to_string())
            })?)
            .ok_or_else(|| {
                SqlQueryError::Invalid(format!(
                    "ORDER BY ordinal {index} exceeds projection length {}",
                    projection.len()
                ))
            })?;
        order.expr = (*field.expr).clone();
        return Ok(order);
    }
    if let Some(alias) = single_field_name(&order.expr)
        && let Some(field) = projection
            .iter()
            .find(|field| field.alias.as_deref() == Some(alias))
    {
        order.expr = (*field.expr).clone();
    }
    Ok(order)
}

pub(crate) fn resolve_aggregate_order(
    mut order: DbOrderBy,
    projection: &[QueryField],
    group_by: &[Expr],
    group_bindings: &[String],
) -> Result<DbOrderBy, SqlQueryError> {
    let projection_index = if let Some(index) = order_ordinal(&order.expr) {
        Some(index.checked_sub(1).ok_or_else(|| {
            SqlQueryError::Invalid("ORDER BY ordinal must be at least 1".to_string())
        })?)
    } else if let Some(name) = single_field_name(&order.expr) {
        projection
            .iter()
            .position(|field| field.alias.as_deref() == Some(name))
            .or_else(|| {
                projection
                    .iter()
                    .position(|field| field.expr.as_ref() == &order.expr)
            })
            .or_else(|| {
                projection.iter().position(|field| {
                    field.alias.is_none() && field_expr_final_name(&field.expr) == Some(name)
                })
            })
    } else {
        projection
            .iter()
            .position(|field| field.expr.as_ref() == &order.expr)
    };
    let Some(index) = projection_index else {
        return Err(SqlQueryError::Invalid(
            "aggregate ORDER BY expressions must reference a projected expression, alias, or ordinal"
                .to_string(),
        ));
    };
    let field = projection.get(index).ok_or_else(|| {
        SqlQueryError::Invalid(format!(
            "ORDER BY ordinal {} exceeds projection length {}",
            index + 1,
            projection.len()
        ))
    })?;
    validate_grouped_expr(&field.expr, group_by, group_bindings, "ORDER BY")?;
    let key = projection_output_key(field).ok_or_else(|| {
        SqlQueryError::Unsupported(
            "aggregate ORDER BY cannot reference a wildcard projection".to_string(),
        )
    })?;
    order.expr = Expr::Operand(Operand::Field(FieldPath::from_fields([key])));
    Ok(order)
}

pub(crate) fn order_ordinal(expr: &Expr) -> Option<usize> {
    let Expr::Operand(Operand::Literal(value)) = expr else {
        return None;
    };
    match value {
        Value::I8(value) => usize::try_from(*value).ok(),
        Value::I16(value) => usize::try_from(*value).ok(),
        Value::I32(value) => usize::try_from(*value).ok(),
        Value::I64(value) => usize::try_from(*value).ok(),
        Value::I128(value) => usize::try_from(*value).ok(),
        Value::U8(value) => Some(*value as usize),
        Value::U16(value) => Some(*value as usize),
        Value::U32(value) => usize::try_from(*value).ok(),
        Value::U64(value) => usize::try_from(*value).ok(),
        Value::U128(value) => usize::try_from(*value).ok(),
        _ => None,
    }
}

pub(crate) fn single_field_name(expr: &Expr) -> Option<&str> {
    let Expr::Operand(Operand::Field(path)) = expr else {
        return None;
    };
    let [PathSegment::Field(name)] = path.segments() else {
        return None;
    };
    Some(name)
}

pub(crate) fn expr_contains_nested_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Aggregate { arg, .. } => match arg.as_ref() {
            FunctionArg::Expr(expr) => expr_contains_aggregate(expr),
            FunctionArg::Wildcard => false,
        },
        Expr::Operand(_) | Expr::Subquery(_) | Expr::Exists { .. } => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } | Expr::ProjectionRef(expr) => {
            expr_contains_nested_aggregate(expr)
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
        } => expr_contains_nested_aggregate(left) || expr_contains_nested_aggregate(right),
        Expr::TextMatch { exprs, query, .. } => {
            exprs.iter().any(expr_contains_nested_aggregate)
                || expr_contains_nested_aggregate(query)
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_nested_aggregate(cond)
                || expr_contains_nested_aggregate(then_expr)
                || expr_contains_nested_aggregate(else_expr)
        }
        Expr::Coalesce(items) => items.iter().any(expr_contains_nested_aggregate),
        Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            FunctionArg::Expr(expr) => expr_contains_nested_aggregate(expr),
            FunctionArg::Wildcard => false,
        }),
        Expr::InList { expr, list, .. } => {
            expr_contains_nested_aggregate(expr) || list.iter().any(expr_contains_nested_aggregate)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_nested_aggregate(expr)
                || expr_contains_nested_aggregate(low)
                || expr_contains_nested_aggregate(high)
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            expr_contains_nested_aggregate(relation)
                || expr_contains_nested_aggregate(source)
                || expr_contains_nested_aggregate(target)
                || max_depth
                    .as_deref()
                    .is_some_and(expr_contains_nested_aggregate)
        }
    }
}

pub(crate) fn group_expr_equivalent(left: &Expr, right: &Expr, bindings: &[String]) -> bool {
    if left == right {
        return true;
    }
    normalize_group_expr(left, bindings) == normalize_group_expr(right, bindings)
}

pub(crate) fn normalize_group_expr(expr: &Expr, bindings: &[String]) -> Expr {
    let normalize = |expr: &Expr| normalize_group_expr(expr, bindings);
    match expr {
        Expr::Operand(Operand::Field(path)) => {
            Expr::Operand(Operand::Field(normalize_group_path(path, bindings)))
        }
        Expr::Operand(Operand::Literal(value)) => Expr::Operand(Operand::Literal(value.clone())),
        Expr::Operand(Operand::Parameter(name)) => Expr::Operand(Operand::Parameter(name.clone())),
        Expr::ProjectionRef(expr) => Expr::ProjectionRef(Box::new(normalize(expr))),
        Expr::Unary { op, expr } => Expr::Unary {
            op: *op,
            expr: Box::new(normalize(expr)),
        },
        Expr::Binary { op, left, right } => Expr::Binary {
            op: *op,
            left: Box::new(normalize(left)),
            right: Box::new(normalize(right)),
        },
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => Expr::IfElse {
            cond: Box::new(normalize(cond)),
            then_expr: Box::new(normalize(then_expr)),
            else_expr: Box::new(normalize(else_expr)),
        },
        Expr::Coalesce(items) => Expr::Coalesce(items.iter().map(normalize).collect()),
        Expr::TextMatch {
            exprs,
            query,
            mode,
            analyzer,
        } => Expr::TextMatch {
            exprs: exprs.iter().map(normalize).collect(),
            query: Box::new(normalize(query)),
            mode: *mode,
            analyzer: *analyzer,
        },
        Expr::Function { name, args } => Expr::Function {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| match arg {
                    FunctionArg::Expr(expr) => FunctionArg::Expr(normalize(expr)),
                    FunctionArg::Wildcard => FunctionArg::Wildcard,
                })
                .collect(),
        },
        Expr::Aggregate { op, distinct, arg } => Expr::Aggregate {
            op: *op,
            distinct: *distinct,
            arg: Box::new(match arg.as_ref() {
                FunctionArg::Expr(expr) => FunctionArg::Expr(normalize(expr)),
                FunctionArg::Wildcard => FunctionArg::Wildcard,
            }),
        },
        Expr::InList {
            expr,
            list,
            negated,
        } => Expr::InList {
            expr: Box::new(normalize(expr)),
            list: list.iter().map(normalize).collect(),
            negated: *negated,
        },
        Expr::Subquery(query) => Expr::Subquery(query.clone()),
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => Expr::Between {
            expr: Box::new(normalize(expr)),
            low: Box::new(normalize(low)),
            high: Box::new(normalize(high)),
            negated: *negated,
        },
        Expr::PatternMatch {
            kind,
            expr,
            pattern,
            case_insensitive,
            negated,
        } => Expr::PatternMatch {
            kind: *kind,
            expr: Box::new(normalize(expr)),
            pattern: Box::new(normalize(pattern)),
            case_insensitive: *case_insensitive,
            negated: *negated,
        },
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive,
            negated,
        } => Expr::RegexMatch {
            expr: Box::new(normalize(expr)),
            pattern: Box::new(normalize(pattern)),
            case_insensitive: *case_insensitive,
            negated: *negated,
        },
        Expr::IsNull { expr, negated } => Expr::IsNull {
            expr: Box::new(normalize(expr)),
            negated: *negated,
        },
        Expr::Exists { query, negated } => Expr::Exists {
            query: query.clone(),
            negated: *negated,
        },
        Expr::RelationExists {
            relation,
            source,
            target,
            transitive,
            max_depth,
        } => Expr::RelationExists {
            relation: Box::new(normalize(relation)),
            source: Box::new(normalize(source)),
            target: Box::new(normalize(target)),
            transitive: *transitive,
            max_depth: max_depth.as_deref().map(normalize).map(Box::new),
        },
    }
}

pub(crate) fn normalize_group_path(path: &FieldPath, bindings: &[String]) -> FieldPath {
    for binding in bindings {
        let parts = binding.split('.').collect::<Vec<_>>();
        if path.segments().len() <= parts.len() {
            continue;
        }
        let matches =
            path.segments().iter().zip(parts.iter()).all(
                |(segment, part)| matches!(segment, PathSegment::Field(field) if field == part),
            );
        if matches {
            return FieldPath::from(path.segments()[parts.len()..].to_vec());
        }
    }
    path.clone()
}

pub(crate) fn validate_grouped_expr(
    expr: &Expr,
    group_by: &[Expr],
    group_bindings: &[String],
    clause: &str,
) -> Result<(), SqlQueryError> {
    if group_by
        .iter()
        .any(|group| group_expr_equivalent(group, expr, group_bindings))
    {
        return Ok(());
    }
    match expr {
        Expr::Operand(Operand::Literal(_) | Operand::Parameter(_))
        | Expr::Subquery(_)
        | Expr::Exists { .. } => Ok(()),
        Expr::Operand(Operand::Field(path)) => Err(SqlQueryError::Invalid(format!(
            "field '{}' in {clause} must appear in GROUP BY or be inside an aggregate",
            path_to_sql(path).unwrap_or_else(|_| format!("{path:?}"))
        ))),
        Expr::Aggregate { .. } => Ok(()),
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } | Expr::ProjectionRef(expr) => {
            validate_grouped_expr(expr, group_by, group_bindings, clause)
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
            validate_grouped_expr(left, group_by, group_bindings, clause)?;
            validate_grouped_expr(right, group_by, group_bindings, clause)
        }
        Expr::TextMatch { exprs, query, .. } => {
            for expr in exprs {
                validate_grouped_expr(expr, group_by, group_bindings, clause)?;
            }
            validate_grouped_expr(query, group_by, group_bindings, clause)
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            validate_grouped_expr(cond, group_by, group_bindings, clause)?;
            validate_grouped_expr(then_expr, group_by, group_bindings, clause)?;
            validate_grouped_expr(else_expr, group_by, group_bindings, clause)
        }
        Expr::Coalesce(items) => items
            .iter()
            .try_for_each(|expr| validate_grouped_expr(expr, group_by, group_bindings, clause)),
        Expr::Function { args, .. } => args.iter().try_for_each(|arg| match arg {
            FunctionArg::Expr(expr) => {
                validate_grouped_expr(expr, group_by, group_bindings, clause)
            }
            FunctionArg::Wildcard => Ok(()),
        }),
        Expr::InList { expr, list, .. } => {
            validate_grouped_expr(expr, group_by, group_bindings, clause)?;
            list.iter()
                .try_for_each(|expr| validate_grouped_expr(expr, group_by, group_bindings, clause))
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            validate_grouped_expr(expr, group_by, group_bindings, clause)?;
            validate_grouped_expr(low, group_by, group_bindings, clause)?;
            validate_grouped_expr(high, group_by, group_bindings, clause)
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            validate_grouped_expr(relation, group_by, group_bindings, clause)?;
            validate_grouped_expr(source, group_by, group_bindings, clause)?;
            validate_grouped_expr(target, group_by, group_bindings, clause)?;
            if let Some(max_depth) = max_depth {
                validate_grouped_expr(max_depth, group_by, group_bindings, clause)?;
            }
            Ok(())
        }
    }
}

pub(crate) fn expr_contains_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Aggregate { .. } => true,
        Expr::Operand(_) | Expr::Subquery(_) | Expr::Exists { .. } => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } | Expr::ProjectionRef(expr) => {
            expr_contains_aggregate(expr)
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
        } => expr_contains_aggregate(left) || expr_contains_aggregate(right),
        Expr::TextMatch { exprs, query, .. } => {
            exprs.iter().any(expr_contains_aggregate) || expr_contains_aggregate(query)
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
                || max_depth.as_deref().is_some_and(expr_contains_aggregate)
        }
    }
}

pub(crate) fn path_to_sql(path: &FieldPath) -> Result<String, SqlQueryError> {
    if path.segments().is_empty() {
        return Err(SqlQueryError::Invalid("empty field path".to_string()));
    }
    let mut out = String::new();
    let mut first = true;
    for seg in path.segments() {
        match seg {
            PathSegment::Field(name) => {
                if !first {
                    out.push('.');
                }
                first = false;
                out.push_str(name);
            }
            PathSegment::Index(_) => {
                return Err(SqlQueryError::Unsupported(
                    "indexed field paths are not supported in SQL printer".to_string(),
                ));
            }
        }
    }
    Ok(out)
}
