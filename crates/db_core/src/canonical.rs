use semantic_data::value::{FieldPath, PathSegment, Value};
use thiserror::Error;

use crate::{
    Assignment, DeleteQuery, Expr, InsertQuery, Operand, OrderBy, Query, QueryField, SelectQuery,
    UpdateQuery,
    catalog::{Catalog, CollectionSchema, OBJECT_TYPE_FIELD, is_special_builtin_field},
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
    #[error("ambiguous field alias '{field}' in collection '{collection}' ({context})")]
    AmbiguousField {
        collection: String,
        field: String,
        context: &'static str,
    },
}

pub type CanonicalResult<T> = std::result::Result<T, QueryCanonicalizationError>;

pub fn canonicalize_select_query(
    query: &SelectQuery,
    catalog: &Catalog,
    collection: &CollectionSchema,
) -> CanonicalResult<SelectQuery> {
    let base_binding = select_base_binding(query, collection);
    let join_bindings = select_join_bindings(query);
    let predicate = query
        .predicate
        .as_ref()
        .map(|predicate| {
            canonicalize_expr_with_join_bindings(
                predicate,
                catalog,
                collection,
                Some(base_binding),
                &join_bindings,
                "select predicate",
            )
        })
        .transpose()?;

    let projection = query
        .projection
        .iter()
        .map(|field| canonicalize_projection_field(query, field, catalog, collection))
        .collect::<CanonicalResult<Vec<_>>>()?;

    let joins = query
        .joins
        .iter()
        .map(|join| {
            let condition = match &join.condition {
                crate::JoinCondition::OnExpr(expr) => {
                    crate::JoinCondition::OnExpr(canonicalize_expr_with_join_bindings(
                        expr,
                        catalog,
                        collection,
                        Some(base_binding),
                        &join_bindings,
                        "select join on",
                    )?)
                }
                crate::JoinCondition::UsingFields { left, right } => {
                    crate::JoinCondition::UsingFields {
                        left: canonicalize_path_with_join_bindings(
                            left,
                            catalog,
                            collection,
                            Some(base_binding),
                            &join_bindings,
                            "select join using left",
                        )?,
                        right: canonicalize_path_with_join_bindings(
                            right,
                            catalog,
                            collection,
                            Some(base_binding),
                            &join_bindings,
                            "select join using right",
                        )?,
                    }
                }
            };
            Ok(crate::JoinQuery {
                source: join.source.clone(),
                alias: join.alias.clone(),
                join_type: join.join_type,
                condition,
                predicate: join
                    .predicate
                    .as_ref()
                    .map(|expr| {
                        canonicalize_expr_with_join_bindings(
                            expr,
                            catalog,
                            collection,
                            Some(base_binding),
                            &join_bindings,
                            "select join predicate",
                        )
                    })
                    .transpose()?,
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    let order_by = query
        .order_by
        .iter()
        .map(|order| {
            Ok(OrderBy {
                expr: canonicalize_expr_with_join_bindings(
                    &order.expr,
                    catalog,
                    collection,
                    Some(base_binding),
                    &join_bindings,
                    "select order_by",
                )?,
                direction: order.direction,
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(SelectQuery {
        collection: query.collection.clone(),
        source_alias: query.source_alias.clone(),
        joins,
        predicate,
        projection,
        distinct: query.distinct,
        group_by: query
            .group_by
            .iter()
            .map(|expr| {
                canonicalize_expr_with_join_bindings(
                    expr,
                    catalog,
                    collection,
                    Some(base_binding),
                    &join_bindings,
                    "select group_by",
                )
            })
            .collect::<CanonicalResult<Vec<_>>>()?,
        having: query
            .having
            .as_ref()
            .map(|predicate| {
                canonicalize_expr_with_join_bindings(
                    predicate,
                    catalog,
                    collection,
                    Some(base_binding),
                    &join_bindings,
                    "select having",
                )
            })
            .transpose()?,
        order_by,
        offset: canonicalize_expr(
            &query.offset,
            catalog,
            collection,
            Some(base_binding),
            "select offset",
        )?,
        limit: query
            .limit
            .as_ref()
            .map(|expr| {
                canonicalize_expr(
                    expr,
                    catalog,
                    collection,
                    Some(base_binding),
                    "select limit",
                )
            })
            .transpose()?,
        field_format: query.field_format,
    })
}

pub fn canonicalize_query(
    query: &Query,
    catalog: &Catalog,
    collection: &CollectionSchema,
) -> CanonicalResult<Query> {
    match query {
        Query::Select(query) => {
            canonicalize_select_query(query, catalog, collection).map(Query::Select)
        }
        Query::Insert(query) => {
            canonicalize_insert_query(query, catalog, collection).map(Query::Insert)
        }
        Query::Update(query) => {
            canonicalize_update_query(query, catalog, collection).map(Query::Update)
        }
        Query::Delete(query) => {
            canonicalize_delete_query(query, catalog, collection).map(Query::Delete)
        }
        Query::Ddl(query) => Ok(Query::Ddl(query.clone())),
    }
}

pub fn canonicalize_insert_query(
    query: &InsertQuery,
    catalog: &Catalog,
    collection: &CollectionSchema,
) -> CanonicalResult<InsertQuery> {
    let returning = query
        .returning
        .iter()
        .map(|field| {
            Ok(QueryField {
                expr: Box::new(canonicalize_expr(
                    &field.expr,
                    catalog,
                    collection,
                    None,
                    "insert returning",
                )?),
                alias: field.alias.clone(),
                wildcard: field.wildcard.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(InsertQuery {
        collection: query.collection.clone(),
        columns: query.columns.clone(),
        source: query.source.clone(),
        returning,
        field_format: query.field_format,
    })
}

pub fn canonicalize_update_query(
    query: &UpdateQuery,
    catalog: &Catalog,
    collection: &CollectionSchema,
) -> CanonicalResult<UpdateQuery> {
    let predicate = query
        .predicate
        .as_ref()
        .map(|predicate| {
            canonicalize_expr(predicate, catalog, collection, None, "update predicate")
        })
        .transpose()?;

    let assignments = query
        .assignments
        .iter()
        .map(|assignment| {
            Ok(Assignment {
                path: canonicalize_path(
                    &assignment.path,
                    catalog,
                    collection,
                    None,
                    "update assignment path",
                )?,
                value: canonicalize_expr(
                    &assignment.value,
                    catalog,
                    collection,
                    None,
                    "update assignment value",
                )?,
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
                    catalog,
                    collection,
                    None,
                    "update returning",
                )?),
                alias: field.alias.clone(),
                wildcard: field.wildcard.clone(),
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
            .map(|expr| canonicalize_expr(expr, catalog, collection, None, "update limit"))
            .transpose()?,
        returning,
        field_format: query.field_format,
    })
}

pub fn canonicalize_delete_query(
    query: &DeleteQuery,
    catalog: &Catalog,
    collection: &CollectionSchema,
) -> CanonicalResult<DeleteQuery> {
    let predicate = query
        .predicate
        .as_ref()
        .map(|predicate| {
            canonicalize_expr(predicate, catalog, collection, None, "delete predicate")
        })
        .transpose()?;

    let returning = query
        .returning
        .iter()
        .map(|field| {
            Ok(QueryField {
                expr: Box::new(canonicalize_expr(
                    &field.expr,
                    catalog,
                    collection,
                    None,
                    "delete returning",
                )?),
                alias: field.alias.clone(),
                wildcard: field.wildcard.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    Ok(DeleteQuery {
        collection: query.collection.clone(),
        predicate,
        limit: query
            .limit
            .as_ref()
            .map(|expr| canonicalize_expr(expr, catalog, collection, None, "delete limit"))
            .transpose()?,
        returning,
        field_format: query.field_format,
    })
}

fn canonicalize_projection_field(
    query: &SelectQuery,
    field: &QueryField,
    catalog: &Catalog,
    collection: &CollectionSchema,
) -> CanonicalResult<QueryField> {
    let join_bindings = select_join_bindings(query);
    if let Some(path) = &field.wildcard {
        if path.segments().is_empty() {
            return Ok(field.clone());
        }
        if wildcard_matches_binding(path, query, collection) {
            let wildcard = if query.joins.is_empty()
                && wildcard_matches_base_binding(path, query, collection)
            {
                FieldPath::new()
            } else {
                path.clone()
            };
            return Ok(QueryField {
                expr: field.expr.clone(),
                alias: field.alias.clone(),
                wildcard: Some(wildcard),
            });
        }
    }

    Ok(QueryField {
        expr: Box::new(canonicalize_expr_with_join_bindings(
            &field.expr,
            catalog,
            collection,
            Some(select_base_binding(query, collection)),
            &join_bindings,
            "select projection",
        )?),
        alias: field.alias.clone(),
        wildcard: field
            .wildcard
            .as_ref()
            .map(|path| {
                canonicalize_path_with_join_bindings(
                    path,
                    catalog,
                    collection,
                    Some(select_base_binding(query, collection)),
                    &join_bindings,
                    "select projection",
                )
            })
            .transpose()?,
    })
}

fn wildcard_matches_binding(
    path: &FieldPath,
    query: &SelectQuery,
    collection: &CollectionSchema,
) -> bool {
    wildcard_matches_base_binding(path, query, collection)
        || query.joins.iter().any(|join| {
            let binding = join
                .alias
                .clone()
                .unwrap_or_else(|| join.source.default_binding());
            path_is_single_field(path, &binding)
        })
}

fn wildcard_matches_base_binding(
    path: &FieldPath,
    query: &SelectQuery,
    collection: &CollectionSchema,
) -> bool {
    path_is_single_field(path, select_base_binding(query, collection))
}

fn select_base_binding<'a>(query: &'a SelectQuery, collection: &'a CollectionSchema) -> &'a str {
    query
        .source_alias
        .as_deref()
        .or(query.collection.as_deref())
        .unwrap_or(collection.name.as_str())
}

fn select_join_bindings(query: &SelectQuery) -> Vec<String> {
    query
        .joins
        .iter()
        .map(|join| {
            join.alias
                .clone()
                .unwrap_or_else(|| join.source.default_binding())
        })
        .collect()
}

fn path_is_single_field(path: &FieldPath, value: &str) -> bool {
    matches!(path.segments(), [PathSegment::Field(field)] if field == value)
}

fn canonicalize_expr(
    expr: &Expr,
    catalog: &Catalog,
    collection: &CollectionSchema,
    base_binding: Option<&str>,
    context: &'static str,
) -> CanonicalResult<Expr> {
    canonicalize_expr_with_join_bindings(expr, catalog, collection, base_binding, &[], context)
}

fn canonicalize_expr_with_join_bindings(
    expr: &Expr,
    catalog: &Catalog,
    collection: &CollectionSchema,
    base_binding: Option<&str>,
    join_bindings: &[String],
    context: &'static str,
) -> CanonicalResult<Expr> {
    match expr {
        Expr::Operand(operand) => canonicalize_operand_with_join_bindings(
            operand,
            catalog,
            collection,
            base_binding,
            join_bindings,
            context,
        )
        .map(Expr::Operand),
        Expr::Unary { op, expr } => Ok(Expr::Unary {
            op: *op,
            expr: Box::new(canonicalize_expr_with_join_bindings(
                expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
        }),
        Expr::Binary { op, left, right } => Ok(Expr::Binary {
            op: *op,
            left: Box::new(canonicalize_expr_with_join_bindings(
                left,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            right: Box::new(canonicalize_expr_with_join_bindings(
                right,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
        }),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => Ok(Expr::IfElse {
            cond: Box::new(canonicalize_expr_with_join_bindings(
                cond,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            then_expr: Box::new(canonicalize_expr_with_join_bindings(
                then_expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            else_expr: Box::new(canonicalize_expr_with_join_bindings(
                else_expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
        }),
        Expr::Coalesce(items) => items
            .iter()
            .map(|item| {
                canonicalize_expr_with_join_bindings(
                    item,
                    catalog,
                    collection,
                    base_binding,
                    join_bindings,
                    context,
                )
            })
            .collect::<CanonicalResult<Vec<_>>>()
            .map(Expr::Coalesce),
        Expr::Function { name, args } => Ok(Expr::Function {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| match arg {
                    crate::FunctionArg::Expr(expr) => canonicalize_expr_with_join_bindings(
                        expr,
                        catalog,
                        collection,
                        base_binding,
                        join_bindings,
                        context,
                    )
                    .map(crate::FunctionArg::Expr),
                    crate::FunctionArg::Wildcard => Ok(crate::FunctionArg::Wildcard),
                })
                .collect::<CanonicalResult<Vec<_>>>()?,
        }),
        Expr::Aggregate { op, distinct, arg } => Ok(Expr::Aggregate {
            op: *op,
            distinct: *distinct,
            arg: Box::new(match arg.as_ref() {
                crate::FunctionArg::Expr(expr) => {
                    crate::FunctionArg::Expr(canonicalize_expr_with_join_bindings(
                        expr,
                        catalog,
                        collection,
                        base_binding,
                        join_bindings,
                        context,
                    )?)
                }
                crate::FunctionArg::Wildcard => crate::FunctionArg::Wildcard,
            }),
        }),
        Expr::InList {
            expr,
            list,
            negated,
        } => Ok(Expr::InList {
            expr: Box::new(canonicalize_expr_with_join_bindings(
                expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            list: list
                .iter()
                .map(|item| {
                    canonicalize_expr_with_join_bindings(
                        item,
                        catalog,
                        collection,
                        base_binding,
                        join_bindings,
                        context,
                    )
                })
                .collect::<CanonicalResult<Vec<_>>>()?,
            negated: *negated,
        }),
        Expr::Subquery(query) => Ok(Expr::Subquery(Box::new(canonicalize_subquery(
            query, catalog, collection,
        )?))),
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => Ok(Expr::Between {
            expr: Box::new(canonicalize_expr_with_join_bindings(
                expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            low: Box::new(canonicalize_expr_with_join_bindings(
                low,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            high: Box::new(canonicalize_expr_with_join_bindings(
                high,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
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
            expr: Box::new(canonicalize_expr_with_join_bindings(
                expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            pattern: Box::new(canonicalize_expr_with_join_bindings(
                pattern,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            case_insensitive: *case_insensitive,
            negated: *negated,
        }),
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive,
            negated,
        } => Ok(Expr::RegexMatch {
            expr: Box::new(canonicalize_expr_with_join_bindings(
                expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            pattern: Box::new(canonicalize_expr_with_join_bindings(
                pattern,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            case_insensitive: *case_insensitive,
            negated: *negated,
        }),
        Expr::IsNull { expr, negated } => Ok(Expr::IsNull {
            expr: Box::new(canonicalize_expr_with_join_bindings(
                expr,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            negated: *negated,
        }),
        Expr::Exists { query, negated } => Ok(Expr::Exists {
            query: Box::new(canonicalize_subquery(query, catalog, collection)?),
            negated: *negated,
        }),
        Expr::RelationExists {
            relation,
            source,
            target,
            transitive,
            max_depth,
        } => Ok(Expr::RelationExists {
            relation: Box::new(canonicalize_expr_with_join_bindings(
                relation,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            source: Box::new(canonicalize_expr_with_join_bindings(
                source,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            target: Box::new(canonicalize_expr_with_join_bindings(
                target,
                catalog,
                collection,
                base_binding,
                join_bindings,
                context,
            )?),
            transitive: *transitive,
            max_depth: max_depth
                .as_ref()
                .map(|value| {
                    canonicalize_expr_with_join_bindings(
                        value,
                        catalog,
                        collection,
                        base_binding,
                        join_bindings,
                        context,
                    )
                    .map(Box::new)
                })
                .transpose()?,
        }),
    }
    .map(|expr| normalize_type_predicate_literals(expr, catalog, collection))
}

fn canonicalize_subquery(
    query: &SelectQuery,
    catalog: &Catalog,
    parent_collection: &CollectionSchema,
) -> CanonicalResult<SelectQuery> {
    let collection = query
        .collection
        .as_deref()
        .and_then(|name| catalog.collection_by_name(name))
        .unwrap_or(parent_collection);
    canonicalize_select_query(query, catalog, collection)
}

fn normalize_type_predicate_literals(
    expr: Expr,
    catalog: &Catalog,
    collection: &CollectionSchema,
) -> Expr {
    match expr {
        Expr::Binary { op, left, right } => {
            let left_expr = *left;
            let right_expr = *right;
            if is_object_type_field_expr(&left_expr, collection)
                && let Expr::Operand(Operand::Literal(Value::String(name))) = &right_expr
            {
                let normalized = normalize_type_name_literal(catalog, name);
                return Expr::Binary {
                    op,
                    left: Box::new(left_expr),
                    right: Box::new(Expr::Operand(Operand::Literal(Value::String(normalized)))),
                };
            }
            if is_object_type_field_expr(&right_expr, collection)
                && let Expr::Operand(Operand::Literal(Value::String(name))) = &left_expr
            {
                let normalized = normalize_type_name_literal(catalog, name);
                return Expr::Binary {
                    op,
                    left: Box::new(Expr::Operand(Operand::Literal(Value::String(normalized)))),
                    right: Box::new(right_expr),
                };
            }
            Expr::Binary {
                op,
                left: Box::new(left_expr),
                right: Box::new(right_expr),
            }
        }
        other => other,
    }
}

fn is_object_type_field_expr(expr: &Expr, collection: &CollectionSchema) -> bool {
    let Expr::Operand(Operand::Field(path)) = expr else {
        return false;
    };
    if path.segments().is_empty() {
        return false;
    }
    let Some(PathSegment::Field(field)) = path.segments().last() else {
        return false;
    };
    collection.canonical_field_name(field) == OBJECT_TYPE_FIELD || field == OBJECT_TYPE_FIELD
}

fn normalize_type_name_literal(catalog: &Catalog, value: &str) -> String {
    if !should_canonicalize_type_literal(value) {
        return value.to_string();
    }
    if let Some(class_id) = catalog.class_id(value)
        && let Some(class) = catalog.class_by_lid(class_id)
    {
        return class.class.id.clone();
    }
    if let Some(record_id) = catalog.record_type_id(value)
        && let Some(record) = catalog.record_type_by_lid(record_id)
    {
        return record.id.clone();
    }
    value.to_string()
}

fn should_canonicalize_type_literal(value: &str) -> bool {
    value.contains(':') || value.contains('.') || value.contains('_')
}

fn canonicalize_operand_with_join_bindings(
    operand: &Operand,
    catalog: &Catalog,
    collection: &CollectionSchema,
    base_binding: Option<&str>,
    join_bindings: &[String],
    context: &'static str,
) -> CanonicalResult<Operand> {
    match operand {
        Operand::Field(path) => canonicalize_path_with_join_bindings(
            path,
            catalog,
            collection,
            base_binding,
            join_bindings,
            context,
        )
        .map(Operand::Field),
        Operand::Literal(value) => Ok(Operand::Literal(value.clone())),
    }
}

fn canonicalize_path(
    path: &FieldPath,
    catalog: &Catalog,
    collection: &CollectionSchema,
    base_binding: Option<&str>,
    context: &'static str,
) -> CanonicalResult<FieldPath> {
    canonicalize_path_with_join_bindings(path, catalog, collection, base_binding, &[], context)
}

fn canonicalize_path_with_join_bindings(
    path: &FieldPath,
    catalog: &Catalog,
    collection: &CollectionSchema,
    base_binding: Option<&str>,
    join_bindings: &[String],
    context: &'static str,
) -> CanonicalResult<FieldPath> {
    let Some(PathSegment::Field(first)) = path.segments().first() else {
        return Err(QueryCanonicalizationError::InvalidPath {
            context,
            path: format_path(path),
        });
    };

    if path.0.len() >= 2 && join_bindings.iter().any(|binding| binding == first) {
        return Ok(path.clone());
    }

    let mut out = if base_binding.is_some_and(|binding| binding == first) && path.0.len() >= 2 {
        FieldPath(path.0.iter().skip(1).cloned().collect())
    } else {
        path.clone()
    };
    let Some(PathSegment::Field(first)) = out.segments().first() else {
        return Err(QueryCanonicalizationError::InvalidPath {
            context,
            path: format_path(path),
        });
    };
    let canonical_top_level = canonicalize_field_name(first, catalog, collection, context)?;
    if collection.is_closed_field_set() && !collection.knows_field(&canonical_top_level) {
        return Err(QueryCanonicalizationError::UnknownField {
            collection: collection.name.clone(),
            field: canonical_top_level,
            context,
        });
    }
    out.0[0] = PathSegment::Field(canonical_top_level);

    Ok(out)
}

fn canonicalize_field_name(
    field: &str,
    catalog: &Catalog,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<String> {
    if let Some(field) = catalog.class_named_collection_field_for_alias(collection, field) {
        return Ok(field);
    }

    let attr_ids = catalog.attribute_ids(field);
    if attr_ids.len() > 1 && !is_special_builtin_field(field) {
        return Err(QueryCanonicalizationError::AmbiguousField {
            collection: collection.name.clone(),
            field: field.to_string(),
            context,
        });
    }
    if let Some(attr_id) = attr_ids.first()
        && let Some(attr) = catalog.attribute_by_lid(*attr_id)
    {
        return Ok(attr.attribute.id.clone());
    }
    Ok(collection.canonical_field_name(field).to_string())
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use semantic_data::{
        query::{BinaryOp, JoinType, SortDirection},
        schema::{
            ClassType,
            attribute::attribute_type::AttributeType,
            core::{meta::Meta, type_kind::TypeKind, type_node::Type},
            primitives::string_type::StringType,
        },
        value::{FieldPath, PathSegment, Value},
    };

    use crate::{
        Expr, JoinCondition, JoinQuery, JoinSource, Operand, QueryField, SelectQuery,
        catalog::{Catalog, CollectionKind, IntegrityMode, OBJECT_TYPE_FIELD},
    };

    use super::{QueryCanonicalizationError, canonicalize_select_query};

    #[test]
    fn canonicalizes_plain_and_underscore_fields() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_collection("items", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let query = SelectQuery::new()
            .with_collection("items")
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "title",
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                    "hello".to_string(),
                )))),
            })
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "semantic_title",
                ])))),
                alias: None,
                wildcard: None,
            }]);

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        let Expr::Binary { left, .. } = canonical.predicate.unwrap() else {
            panic!("predicate must be binary");
        };
        let Expr::Operand(Operand::Field(path)) = left.as_ref() else {
            panic!("predicate lhs must be field");
        };
        assert_eq!(
            path.segments().first(),
            Some(&PathSegment::Field("semantic:title".to_string()))
        );

        let Expr::Operand(Operand::Field(project_path)) = canonical.projection[0].expr.as_ref()
        else {
            panic!("projection must be field");
        };
        assert_eq!(
            project_path.segments().first(),
            Some(&PathSegment::Field("semantic:title".to_string()))
        );
    }

    #[test]
    fn leaves_plain_type_literals_unchanged() {
        let mut catalog = Catalog::new();
        let _ = catalog
            .upsert_class(ClassType {
                id: "semantic:article".to_string(),
                name: "Article".to_string(),
                inherits: None,
                extends: vec![],
                attributes: BTreeMap::new(),
                constraints: vec![],
                meta: Meta::default(),
            })
            .unwrap();
        let _ = catalog
            .upsert_collection("items", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let query = SelectQuery::new()
            .with_collection("items")
            .with_predicate(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    OBJECT_TYPE_FIELD,
                ])))),
                right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                    "Article".to_string(),
                )))),
            });

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        let Expr::Binary { right, .. } = canonical.predicate.unwrap() else {
            panic!("predicate must be binary");
        };
        let Expr::Operand(Operand::Literal(Value::String(value))) = right.as_ref() else {
            panic!("predicate rhs must be string literal");
        };
        assert_eq!(value, "Article");
    }

    #[test]
    fn rejects_ambiguous_plain_attribute_in_query_path() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog.upsert_attribute(AttributeType {
            id: "shared:blog:title".to_string(),
            name: "title".to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_collection("items", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let query = SelectQuery::new()
            .with_collection("items")
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "title",
                ])))),
                alias: None,
                wildcard: None,
            }]);

        let err = canonicalize_select_query(&query, &catalog, collection).unwrap_err();
        assert!(matches!(
            err,
            QueryCanonicalizationError::AmbiguousField { field, .. } if field == "title"
        ));
    }

    #[test]
    fn class_named_collection_disambiguates_shadowed_attribute_alias() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(AttributeType {
            id: "semantic:relation:from".to_string(),
            name: "from".to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog.upsert_attribute(AttributeType {
            id: "semantic:base:directory_node:from".to_string(),
            name: "from".to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_class(ClassType {
                id: "semantic:relation".to_string(),
                name: "Relation".to_string(),
                inherits: None,
                extends: vec![],
                attributes: BTreeMap::from([(
                    "from".to_string(),
                    semantic_data::schema::ClassAttribute {
                        attribute: semantic_data::schema::AttributeRef {
                            id: "semantic:relation:from".to_string(),
                        },
                        required: true,
                        ui_order: None,
                        computed: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )]),
                constraints: vec![],
                meta: Meta::default(),
            })
            .unwrap();
        let _ = catalog
            .upsert_class(ClassType {
                id: "semantic:base:directory_node".to_string(),
                name: "DirectoryNode".to_string(),
                inherits: Some(semantic_data::schema::ClassRef {
                    id: "semantic:relation".to_string(),
                }),
                extends: vec![],
                attributes: BTreeMap::from([(
                    "from".to_string(),
                    semantic_data::schema::ClassAttribute {
                        attribute: semantic_data::schema::AttributeRef {
                            id: "semantic:base:directory_node:from".to_string(),
                        },
                        required: true,
                        ui_order: None,
                        computed: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )]),
                constraints: vec![],
                meta: Meta::default(),
            })
            .unwrap();
        let _ = catalog
            .upsert_collection(
                "semantic:base:directory_node",
                CollectionKind::Schema,
                IntegrityMode::Permissive,
            )
            .unwrap();
        let collection = catalog
            .collection_by_name("semantic:base:directory_node")
            .unwrap();

        let query = SelectQuery::new()
            .with_collection("semantic:base:directory_node")
            .with_source_alias("n")
            .with_predicate(eq_field("n", "from", "parent-dir"));

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        assert_binary_lhs_path(
            canonical.predicate.as_ref().unwrap(),
            &["semantic:base:directory_node:from"],
        );
    }

    #[test]
    fn canonicalizes_binding_qualified_wildcards_without_schema_lookup() {
        let mut catalog = Catalog::new();
        let _ = catalog
            .upsert_collection(
                "entities",
                CollectionKind::Schema,
                IntegrityMode::StrictRegisteredSchema,
            )
            .unwrap();
        let collection = catalog.collection_by_name("entities").unwrap();

        let query = SelectQuery::new()
            .with_collection("entities")
            .with_source_alias("d")
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["d"])))),
                alias: None,
                wildcard: Some(FieldPath::from_fields(["d"])),
            }]);

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        assert_eq!(canonical.projection[0].wildcard, Some(FieldPath::new()));
    }

    #[test]
    fn canonicalizes_mixed_bare_wildcard_without_losing_projection_order() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_collection("items", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();
        let query = SelectQuery::new()
            .with_collection("items")
            .with_projection(vec![
                QueryField {
                    expr: Box::new(Expr::Operand(Operand::Literal(Value::Null))),
                    alias: None,
                    wildcard: Some(FieldPath::new()),
                },
                QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                        "title",
                    ])))),
                    alias: Some("selected_title".to_string()),
                    wildcard: None,
                },
            ]);

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        assert_eq!(canonical.projection.len(), 2);
        assert_eq!(canonical.projection[0].wildcard, Some(FieldPath::new()));
        assert!(matches!(
            canonical.projection[0].expr.as_ref(),
            Expr::Operand(Operand::Literal(Value::Null))
        ));
        let Expr::Operand(Operand::Field(path)) = canonical.projection[1].expr.as_ref() else {
            panic!("second projection must remain the explicit field");
        };
        assert_eq!(path, &FieldPath::from_fields(["semantic:title"]));
        assert_eq!(
            canonical.projection[1].alias.as_deref(),
            Some("selected_title")
        );
    }

    #[test]
    fn canonicalizes_join_binding_qualified_wildcards() {
        let mut catalog = Catalog::new();
        let _ = catalog
            .upsert_collection(
                "entities",
                CollectionKind::Schema,
                IntegrityMode::StrictRegisteredSchema,
            )
            .unwrap();
        let collection = catalog.collection_by_name("entities").unwrap();

        let query = SelectQuery::new()
            .with_collection("entities")
            .with_source_alias("n")
            .with_joins(vec![JoinQuery {
                source: JoinSource {
                    collection: Some("entities".to_string()),
                    class: None,
                },
                alias: Some("child".to_string()),
                join_type: JoinType::Inner,
                condition: JoinCondition::OnExpr(Expr::Operand(Operand::Literal(Value::Bool(
                    true,
                )))),
                predicate: None,
            }])
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "child",
                ])))),
                alias: None,
                wildcard: Some(FieldPath::from_fields(["child"])),
            }]);

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        assert_eq!(
            canonical.projection[0].wildcard,
            Some(FieldPath::from_fields(["child"]))
        );
    }

    #[test]
    fn rejects_unknown_qualified_wildcards_in_closed_collections() {
        let mut catalog = Catalog::new();
        let _ = catalog
            .upsert_collection(
                "entities",
                CollectionKind::Schema,
                IntegrityMode::StrictRegisteredSchema,
            )
            .unwrap();
        let collection = catalog.collection_by_name("entities").unwrap();

        let query = SelectQuery::new()
            .with_collection("entities")
            .with_source_alias("d")
            .with_projection(vec![QueryField {
                expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                    "missing",
                ])))),
                alias: None,
                wildcard: Some(FieldPath::from_fields(["missing"])),
            }]);

        let err = canonicalize_select_query(&query, &catalog, collection).unwrap_err();
        assert!(matches!(
            err,
            QueryCanonicalizationError::UnknownField { field, .. } if field == "missing"
        ));
    }

    #[test]
    fn canonicalizes_base_binding_qualified_select_paths() {
        let mut catalog = Catalog::new();
        let _ = catalog.upsert_attribute(AttributeType {
            id: "semantic:title".to_string(),
            name: "title".to_string(),
            ty: Type {
                kind: TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
                constraints: vec![],
                annotations: vec![],
            },
            constraints: vec![],
            meta: Meta::default(),
        });
        let _ = catalog
            .upsert_collection("items", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let collection = catalog.collection_by_name("items").unwrap();

        let query = SelectQuery::new()
            .with_collection("items")
            .with_source_alias("item")
            .with_predicate(eq_field("item", OBJECT_TYPE_FIELD, "semantic:article"))
            .with_projection(vec![QueryField {
                expr: Box::new(field_expr(["item", "title"])),
                alias: None,
                wildcard: None,
            }])
            .with_group_by(vec![field_expr(["item", OBJECT_TYPE_FIELD])])
            .with_order_by(vec![crate::OrderBy {
                expr: field_expr(["item", "title"]),
                direction: SortDirection::Asc,
            }])
            .with_having(eq_field("item", OBJECT_TYPE_FIELD, "semantic:article"));

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        assert_binary_lhs_path(canonical.predicate.as_ref().unwrap(), &[OBJECT_TYPE_FIELD]);
        assert_expr_path(canonical.projection[0].expr.as_ref(), &["semantic:title"]);
        assert_expr_path(&canonical.group_by[0], &[OBJECT_TYPE_FIELD]);
        assert_expr_path(&canonical.order_by[0].expr, &["semantic:title"]);
        assert_binary_lhs_path(canonical.having.as_ref().unwrap(), &[OBJECT_TYPE_FIELD]);
    }

    #[test]
    fn canonicalizes_base_binding_qualified_subquery_paths() {
        let mut catalog = Catalog::new();
        let _ = catalog
            .upsert_collection(
                "entities",
                CollectionKind::Schema,
                IntegrityMode::StrictRegisteredSchema,
            )
            .unwrap();
        let _ = catalog
            .upsert_collection("nodes", CollectionKind::Schema, IntegrityMode::Permissive)
            .unwrap();
        let entities = catalog.collection_by_name("entities").unwrap();

        let subquery = SelectQuery::new()
            .with_collection("nodes")
            .with_source_alias("n")
            .with_projection(vec![QueryField {
                expr: Box::new(field_expr(["n", "to"])),
                alias: None,
                wildcard: None,
            }])
            .with_predicate(eq_field("n", "relation", "semantic:base:directory_node"));
        let query = SelectQuery::new()
            .with_collection("entities")
            .with_source_alias("d")
            .with_predicate(Expr::Binary {
                op: BinaryOp::In,
                left: Box::new(field_expr(["d", "id"])),
                right: Box::new(Expr::Subquery(Box::new(subquery))),
            });

        let canonical = canonicalize_select_query(&query, &catalog, entities).unwrap();
        let Expr::Binary { left, right, .. } = canonical.predicate.as_ref().unwrap() else {
            panic!("predicate must be binary");
        };
        assert_expr_path(left.as_ref(), &["id"]);
        let Expr::Subquery(subquery) = right.as_ref() else {
            panic!("rhs must be a subquery");
        };
        assert_expr_path(subquery.projection[0].expr.as_ref(), &["to"]);
        assert_binary_lhs_path(subquery.predicate.as_ref().unwrap(), &["relation"]);
    }

    #[test]
    fn preserves_join_binding_qualified_paths() {
        let mut catalog = Catalog::new();
        let _ = catalog
            .upsert_collection(
                "entities",
                CollectionKind::Schema,
                IntegrityMode::Permissive,
            )
            .unwrap();
        let collection = catalog.collection_by_name("entities").unwrap();

        let query = SelectQuery::new()
            .with_collection("entities")
            .with_source_alias("base")
            .with_joins(vec![JoinQuery {
                source: JoinSource {
                    collection: Some("entities".to_string()),
                    class: None,
                },
                alias: Some("c".to_string()),
                join_type: JoinType::Inner,
                condition: JoinCondition::OnExpr(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(field_expr(["base", "id"])),
                    right: Box::new(field_expr(["c", "parent_id"])),
                }),
                predicate: Some(eq_field("c", OBJECT_TYPE_FIELD, "semantic:child")),
            }])
            .with_projection(vec![QueryField {
                expr: Box::new(field_expr(["c", "title"])),
                alias: None,
                wildcard: None,
            }]);

        let canonical = canonicalize_select_query(&query, &catalog, collection).unwrap();
        let JoinCondition::OnExpr(Expr::Binary { left, right, .. }) = &canonical.joins[0].condition
        else {
            panic!("join condition must be binary");
        };
        assert_expr_path(left.as_ref(), &["id"]);
        assert_expr_path(right.as_ref(), &["c", "parent_id"]);
        assert_binary_lhs_path(
            canonical.joins[0].predicate.as_ref().unwrap(),
            &["c", OBJECT_TYPE_FIELD],
        );
        assert_expr_path(canonical.projection[0].expr.as_ref(), &["c", "title"]);
    }

    fn field_expr<const N: usize>(segments: [&str; N]) -> Expr {
        Expr::Operand(Operand::Field(FieldPath::from_fields(segments)))
    }

    fn eq_field(field_binding: &str, field_name: &str, value: &str) -> Expr {
        Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field_expr([field_binding, field_name])),
            right: Box::new(Expr::Operand(Operand::Literal(Value::String(
                value.to_string(),
            )))),
        }
    }

    fn assert_binary_lhs_path(expr: &Expr, expected: &[&str]) {
        let Expr::Binary { left, .. } = expr else {
            panic!("expression must be binary");
        };
        assert_expr_path(left.as_ref(), expected);
    }

    fn assert_expr_path(expr: &Expr, expected: &[&str]) {
        let Expr::Operand(Operand::Field(path)) = expr else {
            panic!("expression must be field");
        };
        let actual = path
            .segments()
            .iter()
            .map(|segment| match segment {
                PathSegment::Field(field) => field.as_str(),
                PathSegment::Index(_) => panic!("expected only field path segments"),
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}
