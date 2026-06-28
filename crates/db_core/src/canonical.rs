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
    let predicate = query
        .predicate
        .as_ref()
        .map(|predicate| canonicalize_expr(predicate, catalog, collection, "select predicate"))
        .transpose()?;

    let projection = query
        .projection
        .iter()
        .map(|field| {
            Ok(QueryField {
                expr: Box::new(canonicalize_expr(
                    &field.expr,
                    catalog,
                    collection,
                    "select projection",
                )?),
                alias: field.alias.clone(),
                wildcard: field.wildcard.clone(),
            })
        })
        .collect::<CanonicalResult<Vec<_>>>()?;

    let joins = query
        .joins
        .iter()
        .map(|join| {
            let condition = match &join.condition {
                crate::JoinCondition::OnExpr(expr) => crate::JoinCondition::OnExpr(
                    canonicalize_expr(expr, catalog, collection, "select join on")?,
                ),
                crate::JoinCondition::UsingFields { left, right } => {
                    crate::JoinCondition::UsingFields {
                        left: canonicalize_path(
                            left,
                            catalog,
                            collection,
                            "select join using left",
                        )?,
                        right: canonicalize_path(
                            right,
                            catalog,
                            collection,
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
                        canonicalize_expr(expr, catalog, collection, "select join predicate")
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
                expr: canonicalize_expr(&order.expr, catalog, collection, "select order_by")?,
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
            .map(|expr| canonicalize_expr(expr, catalog, collection, "select group_by"))
            .collect::<CanonicalResult<Vec<_>>>()?,
        having: query
            .having
            .as_ref()
            .map(|predicate| canonicalize_expr(predicate, catalog, collection, "select having"))
            .transpose()?,
        order_by,
        offset: canonicalize_expr(&query.offset, catalog, collection, "select offset")?,
        limit: query
            .limit
            .as_ref()
            .map(|expr| canonicalize_expr(expr, catalog, collection, "select limit"))
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
        .map(|predicate| canonicalize_expr(predicate, catalog, collection, "update predicate"))
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
                    "update assignment path",
                )?,
                value: canonicalize_expr(
                    &assignment.value,
                    catalog,
                    collection,
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
            .map(|expr| canonicalize_expr(expr, catalog, collection, "update limit"))
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
        .map(|predicate| canonicalize_expr(predicate, catalog, collection, "delete predicate"))
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
            .map(|expr| canonicalize_expr(expr, catalog, collection, "delete limit"))
            .transpose()?,
        returning,
        field_format: query.field_format,
    })
}

fn canonicalize_expr(
    expr: &Expr,
    catalog: &Catalog,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<Expr> {
    match expr {
        Expr::Operand(operand) => {
            canonicalize_operand(operand, catalog, collection, context).map(Expr::Operand)
        }
        Expr::Unary { op, expr } => Ok(Expr::Unary {
            op: *op,
            expr: Box::new(canonicalize_expr(expr, catalog, collection, context)?),
        }),
        Expr::Binary { op, left, right } => Ok(Expr::Binary {
            op: *op,
            left: Box::new(canonicalize_expr(left, catalog, collection, context)?),
            right: Box::new(canonicalize_expr(right, catalog, collection, context)?),
        }),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => Ok(Expr::IfElse {
            cond: Box::new(canonicalize_expr(cond, catalog, collection, context)?),
            then_expr: Box::new(canonicalize_expr(then_expr, catalog, collection, context)?),
            else_expr: Box::new(canonicalize_expr(else_expr, catalog, collection, context)?),
        }),
        Expr::Coalesce(items) => items
            .iter()
            .map(|item| canonicalize_expr(item, catalog, collection, context))
            .collect::<CanonicalResult<Vec<_>>>()
            .map(Expr::Coalesce),
        Expr::Function { name, args } => Ok(Expr::Function {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| match arg {
                    crate::FunctionArg::Expr(expr) => {
                        canonicalize_expr(expr, catalog, collection, context)
                            .map(crate::FunctionArg::Expr)
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
                    crate::FunctionArg::Expr(canonicalize_expr(expr, catalog, collection, context)?)
                }
                crate::FunctionArg::Wildcard => crate::FunctionArg::Wildcard,
            }),
        }),
        Expr::InList {
            expr,
            list,
            negated,
        } => Ok(Expr::InList {
            expr: Box::new(canonicalize_expr(expr, catalog, collection, context)?),
            list: list
                .iter()
                .map(|item| canonicalize_expr(item, catalog, collection, context))
                .collect::<CanonicalResult<Vec<_>>>()?,
            negated: *negated,
        }),
        Expr::Subquery(query) => Ok(Expr::Subquery(query.clone())),
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => Ok(Expr::Between {
            expr: Box::new(canonicalize_expr(expr, catalog, collection, context)?),
            low: Box::new(canonicalize_expr(low, catalog, collection, context)?),
            high: Box::new(canonicalize_expr(high, catalog, collection, context)?),
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
            expr: Box::new(canonicalize_expr(expr, catalog, collection, context)?),
            pattern: Box::new(canonicalize_expr(pattern, catalog, collection, context)?),
            case_insensitive: *case_insensitive,
            negated: *negated,
        }),
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive,
            negated,
        } => Ok(Expr::RegexMatch {
            expr: Box::new(canonicalize_expr(expr, catalog, collection, context)?),
            pattern: Box::new(canonicalize_expr(pattern, catalog, collection, context)?),
            case_insensitive: *case_insensitive,
            negated: *negated,
        }),
        Expr::IsNull { expr, negated } => Ok(Expr::IsNull {
            expr: Box::new(canonicalize_expr(expr, catalog, collection, context)?),
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
            relation: Box::new(canonicalize_expr(relation, catalog, collection, context)?),
            source: Box::new(canonicalize_expr(source, catalog, collection, context)?),
            target: Box::new(canonicalize_expr(target, catalog, collection, context)?),
            transitive: *transitive,
            max_depth: max_depth
                .as_ref()
                .map(|value| canonicalize_expr(value, catalog, collection, context).map(Box::new))
                .transpose()?,
        }),
    }
    .map(|expr| normalize_type_predicate_literals(expr, catalog, collection))
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

fn canonicalize_operand(
    operand: &Operand,
    catalog: &Catalog,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<Operand> {
    match operand {
        Operand::Field(path) => {
            canonicalize_path(path, catalog, collection, context).map(Operand::Field)
        }
        Operand::Literal(value) => Ok(Operand::Literal(value.clone())),
    }
}

fn canonicalize_path(
    path: &FieldPath,
    catalog: &Catalog,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<FieldPath> {
    let Some(PathSegment::Field(first)) = path.segments().first() else {
        return Err(QueryCanonicalizationError::InvalidPath {
            context,
            path: format_path(path),
        });
    };

    let mut out = path.clone();
    let canonical_first = collection.canonical_field_name(first).to_string();
    let alias_prefixed = out.0.len() >= 2
        && first.len() <= 2
        && matches!(out.0.get(1), Some(PathSegment::Field(_)))
        && !collection.knows_field(&canonical_first);
    let top_level_index = if alias_prefixed { 1 } else { 0 };

    let Some(PathSegment::Field(top_level_field)) = out.0.get(top_level_index).cloned() else {
        return Err(QueryCanonicalizationError::InvalidPath {
            context,
            path: format_path(path),
        });
    };

    let canonical_top_level =
        canonicalize_field_name(&top_level_field, catalog, collection, context)?;
    if collection.is_closed_field_set() && !collection.knows_field(&canonical_top_level) {
        return Err(QueryCanonicalizationError::UnknownField {
            collection: collection.name.clone(),
            field: canonical_top_level,
            context,
        });
    }
    out.0[top_level_index] = PathSegment::Field(canonical_top_level);

    Ok(out)
}

fn canonicalize_field_name(
    field: &str,
    catalog: &Catalog,
    collection: &CollectionSchema,
    context: &'static str,
) -> CanonicalResult<String> {
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
        query::BinaryOp,
        schema::{
            ClassType,
            attribute::attribute_type::AttributeType,
            core::{meta::Meta, type_kind::TypeKind, type_node::Type},
            primitives::string_type::StringType,
        },
        value::{FieldPath, PathSegment, Value},
    };

    use crate::{
        Expr, Operand, QueryField, SelectQuery,
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
}
