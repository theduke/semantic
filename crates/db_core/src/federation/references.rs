use std::collections::BTreeSet;

use semantic_data::builtin::DEFAULT_COLLECTION;
use semantic_data::query::{
    Expr, FunctionArg, InsertSource, JoinCondition, Query, QueryField, SelectQuery,
};

use crate::catalog::Catalog;

fn join_collection_candidate(
    source: &semantic_data::query::JoinSource,
    local: &Catalog,
) -> Option<String> {
    match (&source.collection, &source.class) {
        (None, Some(class)) if local.class_ids(class).is_empty() => Some(class.clone()),
        (Some(collection), Some(class)) if local.collection_by_name(collection).is_none() => {
            Some(format!("{collection}.{class}"))
        }
        _ => None,
    }
}

fn visit_select_queries(query: &mut Query, visit: &mut impl FnMut(&mut SelectQuery)) {
    match query {
        Query::Select(select) => visit(select),
        Query::Insert(insert) => {
            if let InsertSource::Select(select) = &mut insert.source {
                visit(select);
            }
        }
        Query::Update(_) | Query::Delete(_) | Query::Ddl(_) => {}
    }
    let result = query.visit_expressions_mut(&mut |expr| {
        if let Expr::Subquery(select) | Expr::Exists { query: select, .. } = expr {
            visit(select);
        }
        Ok::<(), std::convert::Infallible>(())
    });
    match result {
        Ok(()) => {}
        Err(impossible) => match impossible {},
    }
}

/// SQL JOIN names may denote classes or collections. Return only names that
/// cannot already be resolved using the original local class/collection rules.
pub fn unresolved_join_collections(query: &Query, local: &Catalog) -> BTreeSet<String> {
    let mut candidates = BTreeSet::new();
    visit_select_queries(&mut query.clone(), &mut |select| {
        for join in &select.joins {
            if let Some(candidate) = join_collection_candidate(&join.source, local) {
                candidates.insert(candidate);
            }
        }
    });
    candidates
}

/// Resolve otherwise unresolved SQL JOIN names against available VDBs.
/// Registered local classes and explicit local collections retain precedence.
pub fn normalize_virtual_joins(
    query: &mut Query,
    local: &Catalog,
    virtual_names: &BTreeSet<String>,
) {
    visit_select_queries(query, &mut |select| {
        for join in &mut select.joins {
            if let Some(candidate) = join_collection_candidate(&join.source, local)
                && virtual_names.contains(&candidate)
            {
                // Keep the SQL binding (the class/export token) when a dotted
                // collection replaces a class-shaped source without an alias.
                if join.alias.is_none() {
                    join.alias = join.source.class.clone();
                }
                join.source.collection = Some(candidate);
                join.source.class = None;
            }
        }
    });
}

/// Every collection read or mutated by a query, including nested SELECTs.
/// Schema operations have no row sources and are routed separately.
pub fn referenced_collections(query: &Query) -> BTreeSet<String> {
    let mut collections = BTreeSet::new();
    match query {
        Query::Select(query) => visit_select(query, &mut collections),
        Query::Insert(query) => {
            visit_collection(query.collection.as_deref(), &mut collections);
            match &query.source {
                InsertSource::Objects(_) => {}
                InsertSource::Values(rows) => {
                    for row in rows {
                        for expr in row {
                            visit_expr(expr, &mut collections);
                        }
                    }
                }
                InsertSource::Select(query) => visit_select(query, &mut collections),
            }
            visit_projection(&query.returning, &mut collections);
        }
        Query::Update(query) => {
            visit_collection(query.collection.as_deref(), &mut collections);
            visit_optional_expr(query.predicate.as_ref(), &mut collections);
            for assignment in &query.assignments {
                visit_expr(&assignment.value, &mut collections);
            }
            visit_optional_expr(query.limit.as_ref(), &mut collections);
            visit_projection(&query.returning, &mut collections);
        }
        Query::Delete(query) => {
            visit_collection(query.collection.as_deref(), &mut collections);
            visit_optional_expr(query.predicate.as_ref(), &mut collections);
            visit_optional_expr(query.limit.as_ref(), &mut collections);
            visit_projection(&query.returning, &mut collections);
        }
        Query::Ddl(_) => {}
    }
    collections
}

fn visit_collection(collection: Option<&str>, collections: &mut BTreeSet<String>) {
    collections.insert(collection.unwrap_or(DEFAULT_COLLECTION).to_owned());
}

fn visit_select(query: &SelectQuery, collections: &mut BTreeSet<String>) {
    visit_collection(query.collection.as_deref(), collections);
    for join in &query.joins {
        visit_collection(join.source.collection.as_deref(), collections);
        match &join.condition {
            JoinCondition::OnExpr(expr) => visit_expr(expr, collections),
            JoinCondition::UsingFields { .. } => {}
        }
        visit_optional_expr(join.predicate.as_ref(), collections);
    }
    visit_optional_expr(query.predicate.as_ref(), collections);
    visit_projection(&query.projection, collections);
    for expr in &query.group_by {
        visit_expr(expr, collections);
    }
    visit_optional_expr(query.having.as_ref(), collections);
    for order in &query.order_by {
        visit_expr(&order.expr, collections);
    }
    visit_expr(&query.offset, collections);
    visit_optional_expr(query.limit.as_ref(), collections);
}

fn visit_projection(projection: &[QueryField], collections: &mut BTreeSet<String>) {
    for field in projection {
        visit_expr(&field.expr, collections);
    }
}

fn visit_optional_expr(expr: Option<&Expr>, collections: &mut BTreeSet<String>) {
    if let Some(expr) = expr {
        visit_expr(expr, collections);
    }
}

fn visit_function_arg(arg: &FunctionArg<Expr>, collections: &mut BTreeSet<String>) {
    match arg {
        FunctionArg::Expr(expr) => visit_expr(expr, collections),
        FunctionArg::Wildcard => {}
    }
}

fn visit_expr(expr: &Expr, collections: &mut BTreeSet<String>) {
    match expr {
        Expr::Operand(_) => {}
        Expr::Subquery(query) | Expr::Exists { query, .. } => visit_select(query, collections),
        Expr::ProjectionRef(expr) | Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => {
            visit_expr(expr, collections);
        }
        Expr::Binary { left, right, .. } => {
            visit_expr(left, collections);
            visit_expr(right, collections);
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            visit_expr(cond, collections);
            visit_expr(then_expr, collections);
            visit_expr(else_expr, collections);
        }
        Expr::Coalesce(items) => {
            for item in items {
                visit_expr(item, collections);
            }
        }
        Expr::Function { args, .. } => {
            for arg in args {
                visit_function_arg(arg, collections);
            }
        }
        Expr::Aggregate { arg, .. } => visit_function_arg(arg, collections),
        Expr::InList { expr, list, .. } => {
            visit_expr(expr, collections);
            for item in list {
                visit_expr(item, collections);
            }
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            visit_expr(expr, collections);
            visit_expr(low, collections);
            visit_expr(high, collections);
        }
        Expr::PatternMatch { expr, pattern, .. } | Expr::RegexMatch { expr, pattern, .. } => {
            visit_expr(expr, collections);
            visit_expr(pattern, collections);
        }
        Expr::TextMatch { exprs, query, .. } => {
            for expr in exprs {
                visit_expr(expr, collections);
            }
            visit_expr(query, collections);
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            visit_expr(relation, collections);
            visit_expr(source, collections);
            visit_expr(target, collections);
            visit_optional_expr(max_depth.as_deref(), collections);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic_data::query::{
        BinaryOp, DeleteQuery, InsertQuery, JoinQuery, JoinSource, JoinType, OrderBy,
        SortDirection, UpdateQuery,
    };
    use semantic_data::value::FieldPath;

    fn subquery(collection: &str) -> Expr {
        Expr::Subquery(Box::new(SelectQuery::new().with_collection(collection)))
    }

    fn field(expr: Expr) -> QueryField {
        QueryField {
            expr: Box::new(expr),
            alias: None,
            wildcard: None,
        }
    }

    fn assert_collections(query: Query, expected: &[&str]) {
        assert_eq!(
            referenced_collections(&query),
            expected.iter().map(|name| (*name).to_owned()).collect()
        );
    }

    #[test]
    fn base_collection() {
        assert_collections(
            SelectQuery::new().with_collection("local").into(),
            &["local"],
        );
    }

    #[test]
    fn missing_collection_uses_default() {
        assert_collections(SelectQuery::new().into(), &[DEFAULT_COLLECTION]);
    }

    #[test]
    fn joins_and_exists_in_join_conditions() {
        let join = |collection: Option<&str>, condition| JoinQuery {
            source: JoinSource {
                collection: collection.map(str::to_owned),
                class: None,
            },
            alias: None,
            join_type: JoinType::Inner,
            condition,
            predicate: None,
        };
        let query = SelectQuery::new().with_collection("local").with_joins(vec![
            join(
                Some("virtual"),
                JoinCondition::OnExpr(Expr::Exists {
                    query: Box::new(SelectQuery::new().with_collection("nested")),
                    negated: false,
                }),
            ),
            join(
                None,
                JoinCondition::UsingFields {
                    left: FieldPath::from_fields(["id"]),
                    right: FieldPath::from_fields(["id"]),
                },
            ),
        ]);
        assert_collections(
            query.into(),
            &["local", "virtual", "nested", DEFAULT_COLLECTION],
        );
    }

    #[test]
    fn nested_subquery_in_where() {
        let nested = SelectQuery::new()
            .with_collection("middle")
            .with_predicate(Expr::Exists {
                query: Box::new(SelectQuery::new().with_collection("inner")),
                negated: true,
            });
        let query = SelectQuery::new()
            .with_collection("outer")
            .with_predicate(Expr::Binary {
                op: BinaryOp::In,
                left: Box::new(Expr::from(1usize)),
                right: Box::new(Expr::Subquery(Box::new(nested))),
            });
        assert_collections(query.into(), &["outer", "middle", "inner"]);
    }

    #[test]
    fn insert_select_includes_target_and_sources() {
        let query = InsertQuery::new()
            .with_collection("target")
            .with_source(InsertSource::Select(
                SelectQuery::new().with_collection("virtual"),
            ));
        assert_collections(query.into(), &["target", "virtual"]);
    }

    #[test]
    fn all_select_expression_positions() {
        let query = SelectQuery::new()
            .with_collection("base")
            .with_projection(vec![field(subquery("projection"))])
            .with_group_by(vec![subquery("group")])
            .with_having(subquery("having"))
            .with_order_by(vec![OrderBy {
                expr: Expr::ProjectionRef(Box::new(subquery("order"))),
                direction: SortDirection::Asc,
            }])
            .with_offset(subquery("offset"))
            .with_limit(subquery("limit"));
        assert_collections(
            query.into(),
            &[
                "base",
                "projection",
                "group",
                "having",
                "order",
                "offset",
                "limit",
            ],
        );
    }

    #[test]
    fn mutation_expressions_include_nested_reads() {
        assert_collections(
            InsertQuery::new()
                .with_collection("target")
                .with_source(InsertSource::Values(vec![vec![subquery("values")]]))
                .with_returning(vec![field(subquery("returning"))])
                .into(),
            &["target", "values", "returning"],
        );
        assert_collections(
            UpdateQuery::new()
                .with_collection("target")
                .with_predicate(subquery("predicate"))
                .set(FieldPath::from_fields(["value"]), subquery("assignment"))
                .with_limit(subquery("limit"))
                .with_returning(vec![field(subquery("returning"))])
                .into(),
            &["target", "predicate", "assignment", "limit", "returning"],
        );
        assert_collections(
            DeleteQuery::new()
                .with_predicate(subquery("predicate"))
                .with_limit(subquery("limit"))
                .with_returning(vec![field(subquery("returning"))])
                .into(),
            &[DEFAULT_COLLECTION, "predicate", "limit", "returning"],
        );
    }

    #[cfg(feature = "sql")]
    #[test]
    fn parsed_sql_discovers_nested_sources() {
        for sql in [
            "SELECT o.id FROM outer_rows o WHERE o.id IN (SELECT v.id FROM virtual_rows v)",
            "SELECT a.id FROM outer_rows a JOIN joined_rows b ON EXISTS (SELECT v.id FROM virtual_rows v)",
            "INSERT INTO outer_rows SELECT * FROM virtual_rows",
        ] {
            let query =
                crate::sql::parse_sql_query_unbound(sql, crate::sql::SqlDialectKind::Generic)
                    .unwrap();
            assert!(
                referenced_collections(&query).contains("virtual_rows"),
                "{sql}"
            );
        }
    }

    #[cfg(feature = "sql")]
    #[test]
    fn virtual_join_candidates_preserve_local_class_and_collection_precedence() {
        let (local, _) = crate::federation::planner::tests::setup();
        let virtual_names = ["fx", "activation.export", "Item", "local.Item"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let parse = |sql| {
            crate::sql::parse_sql_query_unbound(sql, crate::sql::SqlDialectKind::Generic).unwrap()
        };
        for sql in [
            "SELECT a.id FROM local a JOIN Item b ON a.id = b.id",
            "SELECT a.id FROM local a JOIN local.Item b ON a.id = b.id",
        ] {
            let mut query = parse(sql);
            assert!(
                unresolved_join_collections(&query, &local).is_empty(),
                "{sql}"
            );
            let original = query.clone();
            normalize_virtual_joins(&mut query, &local, &virtual_names);
            assert_eq!(query, original, "{sql}");
        }
        for (sql, name, binding) in [
            (
                "SELECT a.id FROM local a JOIN fx b ON a.id = b.id",
                "fx",
                "b",
            ),
            (
                "SELECT a.id FROM local a JOIN activation.export b ON a.id = b.id",
                "activation.export",
                "b",
            ),
            (
                "SELECT a.id FROM local a JOIN activation.export ON a.id = export.id",
                "activation.export",
                "export",
            ),
        ] {
            let mut query = parse(sql);
            assert_eq!(
                unresolved_join_collections(&query, &local),
                BTreeSet::from([name.into()])
            );
            normalize_virtual_joins(&mut query, &local, &virtual_names);
            let Query::Select(select) = query else {
                panic!("select")
            };
            assert_eq!(select.joins[0].source.collection.as_deref(), Some(name));
            assert!(select.joins[0].source.class.is_none());
            assert_eq!(select.joins[0].alias.as_deref(), Some(binding));
        }
        let mut query = parse(
            "SELECT o.id FROM local o WHERE EXISTS (SELECT a.id FROM local a JOIN fx b ON a.id = b.id)",
        );
        assert_eq!(
            unresolved_join_collections(&query, &local),
            BTreeSet::from(["fx".into()])
        );
        normalize_virtual_joins(&mut query, &local, &virtual_names);
        assert!(referenced_collections(&query).contains("fx"));
    }
}
