use semantic_data::value::{FieldPath, PathSegment};
use thiserror::Error;

use crate::{
    Assignment, DeleteQuery, Expr, Operand, OrderBy, Predicate, Query, QueryField, SelectQuery,
    UpdateQuery, catalog::CollectionSchema,
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
                path: canonicalize_path(&field.path, collection, "select projection")?,
                alias: field.alias.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    let order_by = query
        .order_by
        .iter()
        .map(|order| {
            Ok(OrderBy {
                path: canonicalize_path(&order.path, collection, "select order_by")?,
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
        order_by,
        offset: query.offset,
        limit: query.limit,
    })
}

pub fn canonicalize_query(query: &Query, collection: &CollectionSchema) -> CanonicalResult<Query> {
    match query {
        Query::Select(query) => canonicalize_select_query(query, collection).map(Query::Select),
        Query::Update(query) => canonicalize_update_query(query, collection).map(Query::Update),
        Query::Delete(query) => canonicalize_delete_query(query, collection).map(Query::Delete),
    }
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
                path: canonicalize_path(&field.path, collection, "update returning")?,
                alias: field.alias.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(UpdateQuery {
        collection: query.collection.clone(),
        predicate,
        assignments,
        limit: query.limit,
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
                path: canonicalize_path(&field.path, collection, "delete returning")?,
                alias: field.alias.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(DeleteQuery {
        collection: query.collection.clone(),
        predicate,
        limit: query.limit,
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
