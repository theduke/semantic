use semantic_data::value::{FieldPath, PathSegment};
use thiserror::Error;

use crate::{
    Assignment, DeleteQuery, Expr, InsertQuery, Operand, OrderBy, Predicate, Query, QueryField,
    SelectQuery, UpdateQuery, catalog::CollectionSchema,
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum QueryCanonicalizationError {
    #[error("invalid path '{path}' in {context}: top-level segment must be a field")]
    InvalidPath { context: &'static str, path: String },
    #[error("unknown field '{field}' in closed collection '{collection}' ({context})")]
    UnknownField {
        collection: String,
        field: String,
        context: &'static str,
    },
}

pub type CanonicalResult<T> = std::result::Result<T, QueryCanonicalizationError>;

pub fn canonicalize_select_query(
    query: &SelectQuery,
    collection: &CollectionSchema,
) -> CanonicalResult<SelectQuery> {
    let predicate = query
        .predicate
        .as_ref()
        .map(|predicate| canonicalize_predicate(predicate, collection, "select predicate"))
        .transpose()?;

    let projection = query
        .projection
        .iter()
        .map(|field| {
            Ok(QueryField {
                expr: Box::new(canonicalize_expr(
                    &field.expr,
                    collection,
                    "select projection",
                )?),
                alias: field.alias.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    let order_by = query
        .order_by
        .iter()
        .map(|order| {
            Ok(OrderBy {
                expr: canonicalize_expr(&order.expr, collection, "select order_by")?,
                direction: order.direction,
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(SelectQuery {
        collection: query.collection.clone(),
        source_alias: query.source_alias.clone(),
        joins: query.joins.clone(),
        predicate,
        projection,
        distinct: query.distinct,
        group_by: query
            .group_by
            .iter()
            .map(|expr| canonicalize_expr(expr, collection, "select group_by"))
            .collect::<CanonicalResult<Vec<_>>>()?,
        having: query
            .having
            .as_ref()
            .map(|predicate| canonicalize_predicate(predicate, collection, "select having"))
            .transpose()?,
        order_by,
        offset: canonicalize_expr(&query.offset, collection, "select offset")?,
        limit: query
            .limit
            .as_ref()
            .map(|expr| canonicalize_expr(expr, collection, "select limit"))
            .transpose()?,
    })
}

pub fn canonicalize_query(query: &Query, collection: &CollectionSchema) -> CanonicalResult<Query> {
    match query {
        Query::Select(query) => canonicalize_select_query(query, collection).map(Query::Select),
        Query::Insert(query) => canonicalize_insert_query(query, collection).map(Query::Insert),
        Query::Update(query) => canonicalize_update_query(query, collection).map(Query::Update),
        Query::Delete(query) => canonicalize_delete_query(query, collection).map(Query::Delete),
    }
}

pub fn canonicalize_insert_query(
    query: &InsertQuery,
    collection: &CollectionSchema,
) -> CanonicalResult<InsertQuery> {
    let returning = query
        .returning
        .iter()
        .map(|field| {
            Ok(QueryField {
                expr: Box::new(canonicalize_expr(
                    &field.expr,
                    collection,
                    "insert returning",
                )?),
                alias: field.alias.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(InsertQuery {
        collection: query.collection.clone(),
        columns: query.columns.clone(),
        source: query.source.clone(),
        returning,
    })
}

pub fn canonicalize_update_query(
    query: &UpdateQuery,
    collection: &CollectionSchema,
) -> CanonicalResult<UpdateQuery> {
    let predicate = query
        .predicate
        .as_ref()
        .map(|predicate| canonicalize_predicate(predicate, collection, "update predicate"))
        .transpose()?;

    let assignments = query
        .assignments
        .iter()
        .map(|assignment| {
            Ok(Assignment {
                path: canonicalize_path(&assignment.path, collection, "update assignment path")?,
                value: canonicalize_expr(&assignment.value, collection, "update assignment value")?,
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    let returning = query
        .returning
        .iter()
        .map(|field| {
            Ok(QueryField {
                expr: Box::new(canonicalize_expr(
                    &field.expr,
                    collection,
                    "update returning",
                )?),
                alias: field.alias.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(UpdateQuery {
        collection: query.collection.clone(),
        predicate,
        assignments,
        limit: query
            .limit
            .as_ref()
            .map(|expr| canonicalize_expr(expr, collection, "update limit"))
            .transpose()?,
        returning,
    })
}

pub fn canonicalize_delete_query(
    query: &DeleteQuery,
    collection: &CollectionSchema,
) -> CanonicalResult<DeleteQuery> {
    let predicate = query
        .predicate
        .as_ref()
        .map(|predicate| canonicalize_predicate(predicate, collection, "delete predicate"))
        .transpose()?;

    let returning = query
        .returning
        .iter()
        .map(|field| {
            Ok(QueryField {
                expr: Box::new(canonicalize_expr(
                    &field.expr,
                    collection,
                    "delete returning",
                )?),
                alias: field.alias.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(DeleteQuery {
        collection: query.collection.clone(),
        predicate,
        limit: query
            .limit
            .as_ref()
            .map(|expr| canonicalize_expr(expr, collection, "delete limit"))
            .transpose()?,
        returning,
    })
}

fn canonicalize_predicate(
    predicate: &Predicate,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<Predicate> {
    match predicate {
        Predicate::Compare { op, left, right } => {
            let left = canonicalize_operand(left, collection, context)?;
            let right = canonicalize_operand(right, collection, context)?;
            Ok(Predicate::Compare {
                op: *op,
                left,
                right,
            })
        }
        Predicate::Expr(expr) => canonicalize_expr(expr, collection, context).map(Predicate::Expr),
        Predicate::Exists(path) => {
            canonicalize_path(path, collection, context).map(Predicate::Exists)
        }
        Predicate::And(items) => items
            .iter()
            .map(|item| canonicalize_predicate(item, collection, context))
            .collect::<CanonicalResult<Vec<_>>>()
            .map(Predicate::And),
        Predicate::Or(items) => items
            .iter()
            .map(|item| canonicalize_predicate(item, collection, context))
            .collect::<CanonicalResult<Vec<_>>>()
            .map(Predicate::Or),
        Predicate::Not(item) => canonicalize_predicate(item, collection, context)
            .map(|item| Predicate::Not(Box::new(item))),
    }
}

fn canonicalize_expr(
    expr: &Expr,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<Expr> {
    match expr {
        Expr::Operand(operand) => {
            canonicalize_operand(operand, collection, context).map(Expr::Operand)
        }
        Expr::Unary { op, expr } => Ok(Expr::Unary {
            op: *op,
            expr: Box::new(canonicalize_expr(expr, collection, context)?),
        }),
        Expr::Binary { op, left, right } => Ok(Expr::Binary {
            op: *op,
            left: Box::new(canonicalize_expr(left, collection, context)?),
            right: Box::new(canonicalize_expr(right, collection, context)?),
        }),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => Ok(Expr::IfElse {
            cond: Box::new(canonicalize_expr(cond, collection, context)?),
            then_expr: Box::new(canonicalize_expr(then_expr, collection, context)?),
            else_expr: Box::new(canonicalize_expr(else_expr, collection, context)?),
        }),
        Expr::Coalesce(items) => items
            .iter()
            .map(|item| canonicalize_expr(item, collection, context))
            .collect::<CanonicalResult<Vec<_>>>()
            .map(Expr::Coalesce),
        Expr::Function { name, args } => Ok(Expr::Function {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| match arg {
                    crate::FunctionArg::Expr(expr) => {
                        canonicalize_expr(expr, collection, context).map(crate::FunctionArg::Expr)
                    }
                    crate::FunctionArg::Wildcard => Ok(crate::FunctionArg::Wildcard),
                })
                .collect::<CanonicalResult<Vec<_>>>()?,
        }),
        Expr::Aggregate { op, distinct, arg } => Ok(Expr::Aggregate {
            op: *op,
            distinct: *distinct,
            arg: Box::new(match arg.as_ref() {
                crate::FunctionArg::Expr(expr) => {
                    crate::FunctionArg::Expr(canonicalize_expr(expr, collection, context)?)
                }
                crate::FunctionArg::Wildcard => crate::FunctionArg::Wildcard,
            }),
        }),
        Expr::InList {
            expr,
            list,
            negated,
        } => Ok(Expr::InList {
            expr: Box::new(canonicalize_expr(expr, collection, context)?),
            list: list
                .iter()
                .map(|item| canonicalize_expr(item, collection, context))
                .collect::<CanonicalResult<Vec<_>>>()?,
            negated: *negated,
        }),
        Expr::InSubquery {
            expr,
            query,
            negated,
        } => Ok(Expr::InSubquery {
            expr: Box::new(canonicalize_expr(expr, collection, context)?),
            query: query.clone(),
            negated: *negated,
        }),
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => Ok(Expr::Between {
            expr: Box::new(canonicalize_expr(expr, collection, context)?),
            low: Box::new(canonicalize_expr(low, collection, context)?),
            high: Box::new(canonicalize_expr(high, collection, context)?),
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
            expr: Box::new(canonicalize_expr(expr, collection, context)?),
            pattern: Box::new(canonicalize_expr(pattern, collection, context)?),
            case_insensitive: *case_insensitive,
            negated: *negated,
        }),
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive,
            negated,
        } => Ok(Expr::RegexMatch {
            expr: Box::new(canonicalize_expr(expr, collection, context)?),
            pattern: Box::new(canonicalize_expr(pattern, collection, context)?),
            case_insensitive: *case_insensitive,
            negated: *negated,
        }),
        Expr::IsNull { expr, negated } => Ok(Expr::IsNull {
            expr: Box::new(canonicalize_expr(expr, collection, context)?),
            negated: *negated,
        }),
        Expr::Exists { query, negated } => Ok(Expr::Exists {
            query: query.clone(),
            negated: *negated,
        }),
        Expr::RelationExists {
            relation,
            source,
            target,
            transitive,
            max_depth,
        } => Ok(Expr::RelationExists {
            relation: Box::new(canonicalize_expr(relation, collection, context)?),
            source: Box::new(canonicalize_expr(source, collection, context)?),
            target: Box::new(canonicalize_expr(target, collection, context)?),
            transitive: *transitive,
            max_depth: max_depth
                .as_ref()
                .map(|value| canonicalize_expr(value, collection, context).map(Box::new))
                .transpose()?,
        }),
    }
}

fn canonicalize_operand(
    operand: &Operand,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<Operand> {
    match operand {
        Operand::Field(path) => canonicalize_path(path, collection, context).map(Operand::Field),
        Operand::Literal(value) => Ok(Operand::Literal(value.clone())),
    }
}

fn canonicalize_path(
    path: &FieldPath,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<FieldPath> {
    let Some(PathSegment::Field(first)) = path.segments().first() else {
        return Err(QueryCanonicalizationError::InvalidPath {
            context,
            path: format_path(path),
        });
    };

    let canonical = collection.canonical_field_name(first).to_string();
    if collection.is_closed_field_set() && !collection.knows_field(&canonical) {
        return Err(QueryCanonicalizationError::UnknownField {
            collection: collection.name.clone(),
            field: canonical,
            context,
        });
    }

    let mut out = path.clone();
    out.0[0] = PathSegment::Field(canonical);
    Ok(out)
}

fn format_path(path: &FieldPath) -> String {
    if path.segments().is_empty() {
        return "<empty>".to_string();
    }
    let mut out = String::new();
    for segment in path.segments() {
        match segment {
            PathSegment::Field(field) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(field);
            }
            PathSegment::Index(index) => {
                out.push('[');
                out.push_str(&index.to_string());
                out.push(']');
            }
        }
    }
    out
}
