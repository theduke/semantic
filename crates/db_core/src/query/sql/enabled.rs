/// SQL parser/printer bridge for `db_core::Query`.
///
/// TODO(sql-support): Gradually remove the unsupported operation list below by
/// implementing each feature end-to-end (parse -> AST -> execution/planning ->
/// printer), with tests per operation.
///
/// Currently unsupported operations and constructs:
///
/// - Statement-level:
///   - Any statement other than `SELECT`, `INSERT`, `UPDATE`, `DELETE`.
///   - Multiple statements in one SQL string.
///
/// - `SELECT`:
///   - `WITH` / CTEs.
///   - Set operations (`UNION`, `INTERSECT`, `EXCEPT`).
///   - window/named window clauses.
///   - `QUALIFY`, `PREWHERE`, `CLUSTER BY`, `DISTRIBUTE BY`, `SORT BY`.
///   - `SELECT ... INTO`, `EXCLUDE`, value-table mode.
///   - `FETCH`, locking clauses (`FOR UPDATE`, etc), query settings/format/pipe operators.
///   - Multiple base `FROM` sources in one `SELECT`.
///   - Note: `FROM` is optional only in collection-scoped parsing APIs.
///
/// - Joins:
///   - `NATURAL JOIN`.
///   - `JOIN` without explicit constraint.
///   - Join operators outside basic `INNER/LEFT/RIGHT/FULL`.
///   - `USING` with anything other than exactly two field paths.
///
/// - `UPDATE`:
///   - `UPDATE ... FROM`.
///   - `UPDATE` with joins in target table.
///   - Dialect-specific `UPDATE OR ...` forms.
///   - Tuple/multi-target assignments.
///   - Note: target table is optional only in collection-scoped parsing APIs.
///
/// - `DELETE`:
///   - `DELETE ... USING`.
///   - Multi-table `DELETE`.
///   - `DELETE ... ORDER BY`.
///   - Joined delete targets.
///   - Note: `FROM <table>` is optional only in collection-scoped parsing APIs.
///
/// - Expressions and functions:
///   - Most SQL functions except `COALESCE`.
///   - CASE variants other than a single `WHEN ... THEN ... ELSE ... END`.
///   - Unsupported unary/binary operators.
///   - Non-literal limit/offset expressions.
///
/// - Ordering / pagination:
///   - `ORDER BY ... NULLS FIRST/LAST`.
///   - `ORDER BY ... WITH FILL`.
///   - `LIMIT BY`.
///
/// - Names / paths:
///   - Multipart table names (schema-qualified, db-qualified) in current mapping.
///   - Object-name parts that are function-style segments.
///   - Indexed field paths in SQL printer output.
///
/// - Literal / value mapping:
///   - Many non-scalar SQL literal forms.
///   - Many non-scalar `semantic_data::Value` variants in SQL printer output.
use semantic_data::query::{
    AggregateOp, BinaryOp, CompareOp, JoinType, PatternMatchKind, SortDirection, UnaryOp,
};
use semantic_data::value::{FieldPath, PathSegment, Value};
use sqlparser::ast::{
    Assignment, AssignmentTarget, BinaryOperator, Expr as SqlExpr, FromTable, FunctionArguments,
    Insert as SqlInsert, Join, JoinConstraint, JoinOperator, LimitClause, ObjectName, Offset,
    OrderByExpr, OrderByKind, Query as SqlQuery, Select, SelectItem, SetExpr, Statement,
    TableAlias, TableFactor, TableObject, TableWithJoins, UnaryOperator, ValueWithSpan, Values,
};
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;
use thiserror::Error;

use crate::{
    DeleteQuery, Expr, FunctionArg, InsertQuery, InsertSource, JoinCondition, JoinQuery,
    JoinSource, Operand, OrderBy as DbOrderBy, Predicate, Query, QueryField, SelectQuery,
    UpdateQuery, evaluate_usize_expr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqlDialectKind {
    #[default]
    Generic,
    PostgreSql,
    MySql,
    SQLite,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSqlQuery {
    pub query: Query,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SqlQueryError {
    #[error("sql parse error: {0}")]
    Parse(String),
    #[error("unsupported sql operation: {0}")]
    Unsupported(String),
    #[error("invalid sql query: {0}")]
    Invalid(String),
}

pub fn parse_sql_query(
    sql: &str,
    dialect: SqlDialectKind,
) -> Result<ParsedSqlQuery, SqlQueryError> {
    let dialect = dialect_impl(dialect);
    let statements = Parser::parse_sql(dialect.as_ref(), sql)
        .map_err(|err| SqlQueryError::Parse(err.to_string()))?;

    if statements.len() != 1 {
        return Err(SqlQueryError::Invalid(
            "expected exactly one SQL statement".to_string(),
        ));
    }

    parse_statement(statements.into_iter().next().expect("checked len"))
}

pub fn parse_sql_query_for_collection(
    sql: &str,
    expected_collection: &str,
    dialect: SqlDialectKind,
) -> Result<Query, SqlQueryError> {
    let parsed = match parse_sql_query(sql, dialect) {
        Ok(parsed) => parsed,
        Err(SqlQueryError::Parse(_)) => {
            let Some(rewritten) = rewrite_implicit_collection_sql(sql, expected_collection) else {
                return parse_sql_query(sql, dialect).map(|parsed| parsed.query);
            };
            parse_sql_query(&rewritten, dialect)?
        }
        Err(err) => return Err(err),
    };
    let collection = parsed.query.collection().map(ToOwned::to_owned);
    if let Some(collection) = collection {
        if collection != expected_collection {
            return Err(SqlQueryError::Invalid(format!(
                "collection mismatch: SQL query targets '{}', expected '{}'",
                collection, expected_collection
            )));
        }
    }
    Ok(with_query_collection_if_missing(
        parsed.query,
        expected_collection.to_string(),
    ))
}

pub fn query_to_sql(query: &Query) -> Result<String, SqlQueryError> {
    match query {
        Query::Select(select) => select_to_sql(select, select.collection_or_default()),
        Query::Insert(insert) => insert_to_sql(insert, insert.collection_or_default()),
        Query::Update(update) => update_to_sql(update, update.collection_or_default()),
        Query::Delete(delete) => delete_to_sql(delete, delete.collection_or_default()),
    }
}

fn parse_statement(stmt: Statement) -> Result<ParsedSqlQuery, SqlQueryError> {
    match stmt {
        Statement::Query(query) => parse_select_stmt(*query),
        Statement::Insert(insert) => parse_insert_stmt(insert),
        Statement::Update(update) => parse_update_stmt(update),
        Statement::Delete(delete) => parse_delete_stmt(delete),
        other => Err(SqlQueryError::Unsupported(format!(
            "statement type '{}' is not supported",
            other
        ))),
    }
}

fn parse_select_stmt(query: SqlQuery) -> Result<ParsedSqlQuery, SqlQueryError> {
    if query.with.is_some() {
        return Err(SqlQueryError::Unsupported(
            "WITH queries are not supported".to_string(),
        ));
    }
    if query.fetch.is_some() {
        return Err(SqlQueryError::Unsupported(
            "FETCH clause is not supported".to_string(),
        ));
    }
    if !query.locks.is_empty() || query.for_clause.is_some() {
        return Err(SqlQueryError::Unsupported(
            "locking clauses are not supported".to_string(),
        ));
    }
    if query.settings.is_some() || query.format_clause.is_some() || !query.pipe_operators.is_empty()
    {
        return Err(SqlQueryError::Unsupported(
            "query settings/format/pipe operators are not supported".to_string(),
        ));
    }

    let SetExpr::Select(select) = *query.body else {
        return Err(SqlQueryError::Unsupported(
            "only SELECT query bodies are supported".to_string(),
        ));
    };
    parse_select(query.order_by, query.limit_clause, *select)
}

fn parse_select_subquery(query: SqlQuery) -> Result<SelectQuery, SqlQueryError> {
    let parsed = parse_select_stmt(query)?;
    match parsed.query {
        Query::Select(select) => Ok(select),
        _ => Err(SqlQueryError::Invalid(
            "expected SELECT subquery expression".to_string(),
        )),
    }
}

fn parse_select(
    order_by: Option<sqlparser::ast::OrderBy>,
    limit_clause: Option<LimitClause>,
    select: Select,
) -> Result<ParsedSqlQuery, SqlQueryError> {
    let distinct = parse_select_distinct(select.distinct.as_ref())?;
    if select.into.is_some()
        || !select.lateral_views.is_empty()
        || select.prewhere.is_some()
        || !select.connect_by.is_empty()
        || !select.cluster_by.is_empty()
        || !select.distribute_by.is_empty()
        || !select.sort_by.is_empty()
        || !select.named_window.is_empty()
        || select.qualify.is_some()
        || select.value_table_mode.is_some()
    {
        return Err(SqlQueryError::Unsupported(
            "advanced SELECT clauses are not supported".to_string(),
        ));
    }
    if select.exclude.is_some() {
        return Err(SqlQueryError::Unsupported(
            "SELECT EXCLUDE is not supported".to_string(),
        ));
    }

    if select.from.len() > 1 {
        return Err(SqlQueryError::Invalid(
            "SELECT must reference at most one base source".to_string(),
        ));
    }

    let (collection, source_alias, joins) = if select.from.is_empty() {
        (None, None, Vec::new())
    } else {
        let base = select.from.into_iter().next().expect("checked len");
        let (collection, source_alias, joins) = parse_from_clause(base)?;
        (Some(collection), source_alias, joins)
    };
    let projection = parse_projection(select.projection)?;
    let predicate = select.selection.map(parse_predicate).transpose()?;
    let group_by = parse_group_by(select.group_by)?;
    let having = select.having.map(parse_predicate).transpose()?;
    let order_by = parse_order_by(order_by)?;
    let (limit, offset) = parse_limit_clause(limit_clause)?;

    Ok(ParsedSqlQuery {
        query: Query::Select(SelectQuery {
            collection,
            source_alias,
            joins,
            predicate,
            projection,
            distinct,
            group_by,
            having,
            order_by,
            offset,
            limit,
        }),
    })
}

fn parse_select_distinct(
    distinct: Option<&sqlparser::ast::Distinct>,
) -> Result<bool, SqlQueryError> {
    let Some(distinct) = distinct else {
        return Ok(false);
    };
    match distinct {
        sqlparser::ast::Distinct::Distinct => Ok(true),
        other => Err(SqlQueryError::Unsupported(format!(
            "SELECT distinct mode '{other:?}' is not supported"
        ))),
    }
}

fn parse_group_by(group_by: sqlparser::ast::GroupByExpr) -> Result<Vec<Expr>, SqlQueryError> {
    match group_by {
        sqlparser::ast::GroupByExpr::Expressions(exprs, _) => {
            exprs.into_iter().map(parse_expr).collect()
        }
        other => Err(SqlQueryError::Unsupported(format!(
            "GROUP BY form '{other:?}' is not supported"
        ))),
    }
}

fn parse_insert_stmt(insert: SqlInsert) -> Result<ParsedSqlQuery, SqlQueryError> {
    if insert.optimizer_hint.is_some()
        || insert.or.is_some()
        || insert.ignore
        || insert.overwrite
        || !insert.assignments.is_empty()
        || insert.partitioned.is_some()
        || !insert.after_columns.is_empty()
        || insert.has_table_keyword
        || insert.on.is_some()
        || insert.replace_into
        || insert.priority.is_some()
        || insert.insert_alias.is_some()
        || insert.settings.is_some()
        || insert.format_clause.is_some()
        || insert.table_alias.is_some()
    {
        return Err(SqlQueryError::Unsupported(
            "advanced INSERT forms are not supported".to_string(),
        ));
    }

    let collection = parse_insert_table_name(insert.table)?;
    let columns = insert
        .columns
        .into_iter()
        .map(|ident| ident.value)
        .collect::<Vec<_>>();
    let returning = insert
        .returning
        .map(parse_projection)
        .transpose()?
        .unwrap_or_default();
    let source_query = insert.source.ok_or_else(|| {
        SqlQueryError::Unsupported("INSERT DEFAULT VALUES is not supported".to_string())
    })?;
    let source = parse_insert_source(*source_query)?;

    Ok(ParsedSqlQuery {
        query: Query::Insert(InsertQuery {
            collection: Some(collection),
            columns,
            source,
            returning,
        }),
    })
}

fn parse_insert_source(source: SqlQuery) -> Result<InsertSource, SqlQueryError> {
    let SqlQuery {
        with,
        body,
        order_by,
        limit_clause,
        fetch,
        locks,
        for_clause,
        settings,
        format_clause,
        pipe_operators,
    } = source;

    let has_query_clauses = with.is_some()
        || fetch.is_some()
        || !locks.is_empty()
        || for_clause.is_some()
        || settings.is_some()
        || format_clause.is_some()
        || !pipe_operators.is_empty();
    if has_query_clauses {
        return Err(SqlQueryError::Unsupported(
            "advanced INSERT source queries are not supported".to_string(),
        ));
    }

    match *body {
        SetExpr::Values(values) => {
            if order_by.is_some() || limit_clause.is_some() {
                return Err(SqlQueryError::Unsupported(
                    "ORDER BY / LIMIT are not supported for VALUES insert sources".to_string(),
                ));
            }
            parse_insert_values(values)
        }
        SetExpr::Select(select) => {
            let parsed = parse_select(order_by, limit_clause, *select)?;
            let Query::Select(select_query) = parsed.query else {
                return Err(SqlQueryError::Invalid(
                    "failed to parse INSERT SELECT source".to_string(),
                ));
            };
            Ok(InsertSource::Select(select_query))
        }
        other => Err(SqlQueryError::Unsupported(format!(
            "insert source '{}' is not supported",
            other
        ))),
    }
}

fn parse_insert_values(values: Values) -> Result<InsertSource, SqlQueryError> {
    let rows = values
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(parse_expr)
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(InsertSource::Values(rows))
}

fn parse_update_stmt(update: sqlparser::ast::Update) -> Result<ParsedSqlQuery, SqlQueryError> {
    if update.from.is_some() {
        return Err(SqlQueryError::Unsupported(
            "UPDATE .. FROM is not supported".to_string(),
        ));
    }
    if update.or.is_some() {
        return Err(SqlQueryError::Unsupported(
            "dialect-specific UPDATE OR variants are not supported".to_string(),
        ));
    }
    if !update.table.joins.is_empty() {
        return Err(SqlQueryError::Unsupported(
            "UPDATE with JOIN is not supported".to_string(),
        ));
    }
    let collection = parse_base_table_name(&update.table.relation)?;
    let assignments = update
        .assignments
        .into_iter()
        .map(parse_assignment)
        .collect::<Result<Vec<_>, _>>()?;
    let predicate = update.selection.map(parse_predicate).transpose()?;
    let returning = update
        .returning
        .map(parse_projection)
        .transpose()?
        .unwrap_or_default();
    let limit = update.limit.map(parse_expr).transpose()?;

    Ok(ParsedSqlQuery {
        query: Query::Update(UpdateQuery {
            collection: Some(collection),
            predicate,
            assignments,
            limit,
            returning,
        }),
    })
}

fn parse_delete_stmt(delete: sqlparser::ast::Delete) -> Result<ParsedSqlQuery, SqlQueryError> {
    if delete.using.is_some() {
        return Err(SqlQueryError::Unsupported(
            "DELETE .. USING is not supported".to_string(),
        ));
    }
    if !delete.tables.is_empty() {
        return Err(SqlQueryError::Unsupported(
            "multi-table DELETE is not supported".to_string(),
        ));
    }
    if !delete.order_by.is_empty() {
        return Err(SqlQueryError::Unsupported(
            "DELETE ORDER BY is not supported".to_string(),
        ));
    }

    let from_tables = match delete.from {
        FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => tables,
    };
    if from_tables.len() != 1 {
        return Err(SqlQueryError::Invalid(
            "DELETE must reference exactly one table".to_string(),
        ));
    }
    let table = from_tables.into_iter().next().expect("checked len");
    if !table.joins.is_empty() {
        return Err(SqlQueryError::Unsupported(
            "DELETE with JOIN is not supported".to_string(),
        ));
    }
    let collection = parse_base_table_name(&table.relation)?;
    let predicate = delete.selection.map(parse_predicate).transpose()?;
    let returning = delete
        .returning
        .map(parse_projection)
        .transpose()?
        .unwrap_or_default();
    let limit = delete.limit.map(parse_expr).transpose()?;

    Ok(ParsedSqlQuery {
        query: Query::Delete(DeleteQuery {
            collection: Some(collection),
            predicate,
            limit,
            returning,
        }),
    })
}

fn parse_from_clause(
    base: TableWithJoins,
) -> Result<(String, Option<String>, Vec<JoinQuery>), SqlQueryError> {
    let collection = parse_base_table_name(&base.relation)?;
    let source_alias = table_alias(&base.relation);
    let joins = base
        .joins
        .into_iter()
        .map(parse_join)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((collection, source_alias, joins))
}

fn parse_join(join: Join) -> Result<JoinQuery, SqlQueryError> {
    let source = parse_join_source(&join.relation)?;
    let alias = table_alias(&join.relation);

    let (join_type, constraint) = match join.join_operator {
        JoinOperator::Inner(c) | JoinOperator::Join(c) => (JoinType::Inner, c),
        JoinOperator::Left(c) | JoinOperator::LeftOuter(c) => (JoinType::Left, c),
        JoinOperator::Right(c) | JoinOperator::RightOuter(c) => (JoinType::Right, c),
        JoinOperator::FullOuter(c) => (JoinType::Full, c),
        other => {
            return Err(SqlQueryError::Unsupported(format!(
                "join operator '{other:?}' is not supported"
            )));
        }
    };

    let condition = match constraint {
        JoinConstraint::On(expr) => JoinCondition::OnPredicate(parse_predicate(expr)?),
        JoinConstraint::Using(fields) => {
            if fields.len() != 2 {
                return Err(SqlQueryError::Unsupported(
                    "USING requires exactly two field paths for now".to_string(),
                ));
            }
            let left = object_name_to_path(&fields[0])?;
            let right = object_name_to_path(&fields[1])?;
            JoinCondition::UsingFields { left, right }
        }
        JoinConstraint::Natural => {
            return Err(SqlQueryError::Unsupported(
                "NATURAL JOIN is not supported".to_string(),
            ));
        }
        JoinConstraint::None => {
            return Err(SqlQueryError::Unsupported(
                "JOIN without a constraint is not supported".to_string(),
            ));
        }
    };

    Ok(JoinQuery {
        source,
        alias,
        join_type,
        condition,
        predicate: None,
    })
}

fn parse_join_source(factor: &TableFactor) -> Result<JoinSource, SqlQueryError> {
    let name = parse_base_table_name(factor)?;
    let parts = name.split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        ["_"] => Ok(JoinSource {
            collection: None,
            class: None,
        }),
        [class] => Ok(JoinSource {
            collection: None,
            class: Some((*class).to_string()),
        }),
        [collection, "_"] => Ok(JoinSource {
            collection: Some((*collection).to_string()),
            class: None,
        }),
        [collection, class] => Ok(JoinSource {
            collection: Some((*collection).to_string()),
            class: Some((*class).to_string()),
        }),
        _ => Err(SqlQueryError::Unsupported(format!(
            "join source '{name}' is not supported; expected _, <class>, <collection>._, or <collection>.<class>"
        ))),
    }
}

fn parse_projection(items: Vec<SelectItem>) -> Result<Vec<QueryField>, SqlQueryError> {
    if items
        .iter()
        .any(|item| matches!(item, SelectItem::Wildcard(_)))
    {
        return Ok(Vec::new());
    }

    items
        .into_iter()
        .map(|item| match item {
            SelectItem::UnnamedExpr(expr) => Ok(QueryField {
                expr: Box::new(parse_expr(expr)?),
                alias: None,
            }),
            SelectItem::ExprWithAlias { expr, alias } => Ok(QueryField {
                expr: Box::new(parse_expr(expr)?),
                alias: Some(alias.value),
            }),
            SelectItem::QualifiedWildcard(_, _) => Err(SqlQueryError::Unsupported(
                "qualified wildcards are not supported".to_string(),
            )),
            SelectItem::Wildcard(_) => unreachable!("handled above"),
        })
        .collect()
}

fn parse_order_by(
    order_by: Option<sqlparser::ast::OrderBy>,
) -> Result<Vec<DbOrderBy>, SqlQueryError> {
    let Some(order_by) = order_by else {
        return Ok(Vec::new());
    };
    let OrderByKind::Expressions(items) = order_by.kind else {
        return Err(SqlQueryError::Unsupported(
            "non-expression ORDER BY forms are not supported".to_string(),
        ));
    };
    items.into_iter().map(parse_order_item).collect()
}

fn parse_order_item(item: OrderByExpr) -> Result<DbOrderBy, SqlQueryError> {
    if item.options.nulls_first.is_some() {
        return Err(SqlQueryError::Unsupported(
            "ORDER BY NULLS FIRST/LAST is not supported".to_string(),
        ));
    }
    if item.with_fill.is_some() {
        return Err(SqlQueryError::Unsupported(
            "ORDER BY WITH FILL is not supported".to_string(),
        ));
    }
    let direction = match item.options.asc {
        Some(false) => SortDirection::Desc,
        _ => SortDirection::Asc,
    };
    Ok(DbOrderBy {
        expr: parse_expr(item.expr)?,
        direction,
    })
}

fn parse_limit_clause(
    limit_clause: Option<LimitClause>,
) -> Result<(Option<Expr>, Expr), SqlQueryError> {
    let Some(limit_clause) = limit_clause else {
        return Ok((None, Expr::from(0usize)));
    };
    match limit_clause {
        LimitClause::LimitOffset {
            limit,
            offset,
            limit_by,
        } => {
            if !limit_by.is_empty() {
                return Err(SqlQueryError::Unsupported(
                    "LIMIT BY is not supported".to_string(),
                ));
            }
            let limit = limit.map(parse_expr).transpose()?;
            let offset = offset
                .map(parse_offset)
                .transpose()?
                .unwrap_or_else(|| Expr::from(0usize));
            Ok((limit, offset))
        }
        LimitClause::OffsetCommaLimit { offset, limit } => {
            Ok((Some(parse_expr(limit)?), parse_expr(offset)?))
        }
    }
}

fn parse_offset(offset: Offset) -> Result<Expr, SqlQueryError> {
    let _ = offset.rows;
    parse_expr(offset.value)
}

fn parse_assignment(assign: Assignment) -> Result<crate::Assignment, SqlQueryError> {
    let path = match assign.target {
        AssignmentTarget::ColumnName(name) => object_name_to_path(&name)?,
        AssignmentTarget::Tuple(_) => {
            return Err(SqlQueryError::Unsupported(
                "tuple assignments are not supported".to_string(),
            ));
        }
    };
    Ok(crate::Assignment {
        path,
        value: parse_expr(assign.value)?,
    })
}

fn parse_predicate(expr: SqlExpr) -> Result<Predicate, SqlQueryError> {
    let parsed = parse_expr(expr)?;
    expr_to_predicate(parsed)
}

fn expr_to_predicate(parsed: Expr) -> Result<Predicate, SqlQueryError> {
    match parsed {
        Expr::Binary { op, left, right } => match op {
            BinaryOp::And => Ok(Predicate::And(vec![
                expr_to_predicate(*left)?,
                expr_to_predicate(*right)?,
            ])),
            BinaryOp::Or => Ok(Predicate::Or(vec![
                expr_to_predicate(*left)?,
                expr_to_predicate(*right)?,
            ])),
            BinaryOp::Eq => predicate_compare_or_expr(CompareOp::Eq, *left, *right),
            BinaryOp::NotEq => predicate_compare_or_expr(CompareOp::NotEq, *left, *right),
            BinaryOp::Lt => predicate_compare_or_expr(CompareOp::Lt, *left, *right),
            BinaryOp::Lte => predicate_compare_or_expr(CompareOp::Lte, *left, *right),
            BinaryOp::Gt => predicate_compare_or_expr(CompareOp::Gt, *left, *right),
            BinaryOp::Gte => predicate_compare_or_expr(CompareOp::Gte, *left, *right),
            _ => Ok(Predicate::Expr(Expr::Binary { op, left, right })),
        },
        Expr::Unary {
            op: UnaryOp::Not,
            expr,
        } => Ok(Predicate::Not(Box::new(expr_to_predicate(*expr)?))),
        Expr::Operand(Operand::Field(path)) => Ok(Predicate::Exists(path)),
        other => Ok(Predicate::Expr(other)),
    }
}

fn predicate_compare(op: CompareOp, left: Expr, right: Expr) -> Result<Predicate, SqlQueryError> {
    let left = expr_to_operand(left)?;
    let right = expr_to_operand(right)?;
    Ok(Predicate::Compare { op, left, right })
}

fn predicate_compare_or_expr(
    op: CompareOp,
    left: Expr,
    right: Expr,
) -> Result<Predicate, SqlQueryError> {
    let left_keep = left.clone();
    let right_keep = right.clone();
    match predicate_compare(op, left, right) {
        Ok(pred) => Ok(pred),
        Err(_) => Ok(Predicate::Expr(Expr::Binary {
            op: compare_to_binary(op),
            left: Box::new(left_keep),
            right: Box::new(right_keep),
        })),
    }
}

fn compare_to_binary(op: CompareOp) -> BinaryOp {
    match op {
        CompareOp::Eq => BinaryOp::Eq,
        CompareOp::NotEq => BinaryOp::NotEq,
        CompareOp::Lt => BinaryOp::Lt,
        CompareOp::Lte => BinaryOp::Lte,
        CompareOp::Gt => BinaryOp::Gt,
        CompareOp::Gte => BinaryOp::Gte,
    }
}

fn expr_to_operand(expr: Expr) -> Result<Operand, SqlQueryError> {
    match expr {
        Expr::Operand(operand) => Ok(operand),
        other => Err(SqlQueryError::Unsupported(format!(
            "predicate side must be a field or literal, found '{other:?}'"
        ))),
    }
}

fn parse_expr(expr: SqlExpr) -> Result<Expr, SqlQueryError> {
    match expr {
        SqlExpr::Identifier(ident) => Ok(Expr::Operand(Operand::Field(FieldPath::from_fields([
            ident.value,
        ])))),
        SqlExpr::CompoundIdentifier(idents) => {
            Ok(Expr::Operand(Operand::Field(idents_to_path(&idents)?)))
        }
        SqlExpr::Value(value) => Ok(Expr::Operand(Operand::Literal(parse_literal(value)?))),
        SqlExpr::Nested(expr) => parse_expr(*expr),
        SqlExpr::UnaryOp { op, expr } => Ok(Expr::Unary {
            op: match op {
                UnaryOperator::Not => UnaryOp::Not,
                UnaryOperator::Minus => UnaryOp::Neg,
                other => {
                    return Err(SqlQueryError::Unsupported(format!(
                        "unary operator '{other:?}' is not supported"
                    )));
                }
            },
            expr: Box::new(parse_expr(*expr)?),
        }),
        SqlExpr::BinaryOp { left, op, right } => {
            let left_expr = parse_expr(*left)?;
            let right_expr = parse_expr(*right)?;
            match op {
                BinaryOperator::PGRegexMatch => Ok(Expr::RegexMatch {
                    expr: Box::new(left_expr),
                    pattern: Box::new(right_expr),
                    case_insensitive: false,
                    negated: false,
                }),
                BinaryOperator::PGRegexIMatch => Ok(Expr::RegexMatch {
                    expr: Box::new(left_expr),
                    pattern: Box::new(right_expr),
                    case_insensitive: true,
                    negated: false,
                }),
                BinaryOperator::PGRegexNotMatch => Ok(Expr::RegexMatch {
                    expr: Box::new(left_expr),
                    pattern: Box::new(right_expr),
                    case_insensitive: false,
                    negated: true,
                }),
                BinaryOperator::PGRegexNotIMatch => Ok(Expr::RegexMatch {
                    expr: Box::new(left_expr),
                    pattern: Box::new(right_expr),
                    case_insensitive: true,
                    negated: true,
                }),
                _ => Ok(Expr::Binary {
                    op: parse_binary_op(op)?,
                    left: Box::new(left_expr),
                    right: Box::new(right_expr),
                }),
            }
        }
        SqlExpr::InList {
            expr,
            list,
            negated,
        } => Ok(Expr::InList {
            expr: Box::new(parse_expr(*expr)?),
            list: list
                .into_iter()
                .map(parse_expr)
                .collect::<Result<Vec<_>, _>>()?,
            negated,
        }),
        SqlExpr::InSubquery {
            expr,
            subquery,
            negated,
        } => Ok(Expr::InSubquery {
            expr: Box::new(parse_expr(*expr)?),
            query: Box::new(parse_select_subquery(*subquery)?),
            negated,
        }),
        SqlExpr::Between {
            expr,
            negated,
            low,
            high,
        } => Ok(Expr::Between {
            expr: Box::new(parse_expr(*expr)?),
            low: Box::new(parse_expr(*low)?),
            high: Box::new(parse_expr(*high)?),
            negated,
        }),
        SqlExpr::Like {
            negated,
            expr,
            pattern,
            escape_char,
            ..
        } => {
            if escape_char.is_some() {
                return Err(SqlQueryError::Unsupported(
                    "LIKE ESCAPE is not supported".to_string(),
                ));
            }
            Ok(Expr::PatternMatch {
                kind: PatternMatchKind::Like,
                expr: Box::new(parse_expr(*expr)?),
                pattern: Box::new(parse_expr(*pattern)?),
                case_insensitive: false,
                negated,
            })
        }
        SqlExpr::ILike {
            negated,
            expr,
            pattern,
            escape_char,
            ..
        } => {
            if escape_char.is_some() {
                return Err(SqlQueryError::Unsupported(
                    "ILIKE ESCAPE is not supported".to_string(),
                ));
            }
            Ok(Expr::PatternMatch {
                kind: PatternMatchKind::Like,
                expr: Box::new(parse_expr(*expr)?),
                pattern: Box::new(parse_expr(*pattern)?),
                case_insensitive: true,
                negated,
            })
        }
        SqlExpr::SimilarTo {
            negated,
            expr,
            pattern,
            escape_char,
        } => {
            if escape_char.is_some() {
                return Err(SqlQueryError::Unsupported(
                    "SIMILAR TO ESCAPE is not supported".to_string(),
                ));
            }
            Ok(Expr::PatternMatch {
                kind: PatternMatchKind::SimilarTo,
                expr: Box::new(parse_expr(*expr)?),
                pattern: Box::new(parse_expr(*pattern)?),
                case_insensitive: false,
                negated,
            })
        }
        SqlExpr::IsNull(expr) => Ok(Expr::IsNull {
            expr: Box::new(parse_expr(*expr)?),
            negated: false,
        }),
        SqlExpr::IsNotNull(expr) => Ok(Expr::IsNull {
            expr: Box::new(parse_expr(*expr)?),
            negated: true,
        }),
        SqlExpr::Exists { subquery, negated } => Ok(Expr::Exists {
            query: Box::new(parse_select_subquery(*subquery)?),
            negated,
        }),
        SqlExpr::Function(function) => parse_function_expr(function),
        SqlExpr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            if operand.is_some() || conditions.len() != 1 {
                return Err(SqlQueryError::Unsupported(
                    "CASE forms other than a single WHEN are not supported".to_string(),
                ));
            }
            let when = conditions.into_iter().next().expect("checked len");
            let else_expr = else_result.ok_or_else(|| {
                SqlQueryError::Unsupported("CASE requires ELSE expression".to_string())
            })?;
            Ok(Expr::IfElse {
                cond: Box::new(parse_expr(when.condition)?),
                then_expr: Box::new(parse_expr(when.result)?),
                else_expr: Box::new(parse_expr(*else_expr)?),
            })
        }
        other => Err(SqlQueryError::Unsupported(format!(
            "expression '{other}' is not supported"
        ))),
    }
}

fn parse_function_expr(function: sqlparser::ast::Function) -> Result<Expr, SqlQueryError> {
    if function.over.is_some() || !function.within_group.is_empty() || function.filter.is_some() {
        return Err(SqlQueryError::Unsupported(format!(
            "window/filter modifiers are not supported for function '{}'",
            function.name
        )));
    }
    let mut args = Vec::new();
    let FunctionArguments::List(argument_list) = function.args else {
        return Err(SqlQueryError::Unsupported(format!(
            "function '{}' argument form is not supported",
            function.name
        )));
    };
    for arg in argument_list.args {
        match arg {
            sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Expr(expr)) => {
                args.push(FunctionArg::Expr(parse_expr(expr)?));
            }
            sqlparser::ast::FunctionArg::Unnamed(sqlparser::ast::FunctionArgExpr::Wildcard) => {
                args.push(FunctionArg::Wildcard);
            }
            _ => {
                return Err(SqlQueryError::Unsupported(format!(
                    "function '{}' only supports plain expression or '*' args",
                    function.name
                )));
            }
        }
    }
    let distinct = matches!(
        argument_list.duplicate_treatment,
        Some(sqlparser::ast::DuplicateTreatment::Distinct)
    );
    if let Some(op) = aggregate_op_from_name(&function.name.to_string()) {
        let arg = if args.len() == 1 {
            args.into_iter().next().expect("checked len")
        } else if args.is_empty() && matches!(op, AggregateOp::Count) {
            FunctionArg::Wildcard
        } else {
            return Err(SqlQueryError::Unsupported(format!(
                "aggregate '{}' expects exactly one argument",
                function.name
            )));
        };
        return Ok(Expr::Aggregate {
            op,
            distinct,
            arg: Box::new(arg),
        });
    }
    if function.name.to_string().eq_ignore_ascii_case("coalesce") {
        let mut exprs = Vec::new();
        for arg in args {
            let FunctionArg::Expr(expr) = arg else {
                return Err(SqlQueryError::Unsupported(
                    "COALESCE does not support wildcard args".to_string(),
                ));
            };
            exprs.push(expr);
        }
        Ok(Expr::Coalesce(exprs))
    } else {
        Ok(Expr::Function {
            name: function.name.to_string(),
            args,
        })
    }
}

fn aggregate_op_from_name(name: &str) -> Option<AggregateOp> {
    if name.eq_ignore_ascii_case("count") {
        Some(AggregateOp::Count)
    } else if name.eq_ignore_ascii_case("sum") {
        Some(AggregateOp::Sum)
    } else if name.eq_ignore_ascii_case("avg") {
        Some(AggregateOp::Avg)
    } else if name.eq_ignore_ascii_case("min") {
        Some(AggregateOp::Min)
    } else if name.eq_ignore_ascii_case("max") {
        Some(AggregateOp::Max)
    } else {
        None
    }
}

fn parse_binary_op(op: BinaryOperator) -> Result<BinaryOp, SqlQueryError> {
    match op {
        BinaryOperator::Plus => Ok(BinaryOp::Add),
        BinaryOperator::Minus => Ok(BinaryOp::Sub),
        BinaryOperator::Multiply => Ok(BinaryOp::Mul),
        BinaryOperator::Divide => Ok(BinaryOp::Div),
        BinaryOperator::Modulo => Ok(BinaryOp::Mod),
        BinaryOperator::StringConcat => Ok(BinaryOp::Concat),
        BinaryOperator::And => Ok(BinaryOp::And),
        BinaryOperator::Or => Ok(BinaryOp::Or),
        BinaryOperator::Eq => Ok(BinaryOp::Eq),
        BinaryOperator::NotEq => Ok(BinaryOp::NotEq),
        BinaryOperator::Lt => Ok(BinaryOp::Lt),
        BinaryOperator::LtEq => Ok(BinaryOp::Lte),
        BinaryOperator::Gt => Ok(BinaryOp::Gt),
        BinaryOperator::GtEq => Ok(BinaryOp::Gte),
        other => Err(SqlQueryError::Unsupported(format!(
            "binary operator '{other:?}' is not supported"
        ))),
    }
}

fn parse_literal(value: ValueWithSpan) -> Result<Value, SqlQueryError> {
    use sqlparser::ast::Value as SqlValue;
    match value.value {
        SqlValue::Null => Ok(Value::Null),
        SqlValue::Boolean(v) => Ok(Value::Bool(v)),
        SqlValue::Number(num, _) => {
            if let Ok(v) = num.parse::<i64>() {
                Ok(Value::I64(v))
            } else {
                num.parse::<f64>()
                    .map(|v| Value::F64(v.into()))
                    .map_err(|_| SqlQueryError::Invalid(format!("invalid numeric literal '{num}'")))
            }
        }
        SqlValue::SingleQuotedString(s)
        | SqlValue::DoubleQuotedString(s)
        | SqlValue::TripleSingleQuotedString(s)
        | SqlValue::TripleDoubleQuotedString(s)
        | SqlValue::EscapedStringLiteral(s)
        | SqlValue::UnicodeStringLiteral(s)
        | SqlValue::SingleQuotedByteStringLiteral(s)
        | SqlValue::DoubleQuotedByteStringLiteral(s)
        | SqlValue::TripleSingleQuotedByteStringLiteral(s)
        | SqlValue::TripleDoubleQuotedByteStringLiteral(s)
        | SqlValue::SingleQuotedRawStringLiteral(s)
        | SqlValue::DoubleQuotedRawStringLiteral(s)
        | SqlValue::TripleSingleQuotedRawStringLiteral(s)
        | SqlValue::TripleDoubleQuotedRawStringLiteral(s) => Ok(Value::String(s)),
        other => Err(SqlQueryError::Unsupported(format!(
            "literal '{other:?}' is not supported"
        ))),
    }
}

fn parse_base_table_name(factor: &TableFactor) -> Result<String, SqlQueryError> {
    match factor {
        TableFactor::Table { name, .. } => object_name_to_string(name),
        other => Err(SqlQueryError::Unsupported(format!(
            "table factor '{other}' is not supported"
        ))),
    }
}

fn parse_insert_table_name(table: TableObject) -> Result<String, SqlQueryError> {
    match table {
        TableObject::TableName(name) => object_name_to_string(&name),
        TableObject::TableFunction(function) => Err(SqlQueryError::Unsupported(format!(
            "table function '{}' is not supported for INSERT target",
            function
        ))),
    }
}

fn table_alias(factor: &TableFactor) -> Option<String> {
    match factor {
        TableFactor::Table { alias, .. } => alias_name(alias),
        _ => None,
    }
}

fn alias_name(alias: &Option<TableAlias>) -> Option<String> {
    alias.as_ref().map(|a| a.name.value.clone())
}

fn idents_to_path(idents: &[sqlparser::ast::Ident]) -> Result<FieldPath, SqlQueryError> {
    if idents.is_empty() {
        return Err(SqlQueryError::Invalid("empty identifier path".to_string()));
    }
    Ok(FieldPath::from(
        idents
            .iter()
            .map(|ident| PathSegment::Field(ident.value.clone()))
            .collect::<Vec<_>>(),
    ))
}

fn object_name_to_path(name: &ObjectName) -> Result<FieldPath, SqlQueryError> {
    let mut idents = Vec::with_capacity(name.0.len());
    for part in &name.0 {
        let ident = part.as_ident().ok_or_else(|| {
            SqlQueryError::Unsupported("object name function parts are not supported".to_string())
        })?;
        idents.push(ident.clone());
    }
    idents_to_path(&idents)
}

fn object_name_to_string(name: &ObjectName) -> Result<String, SqlQueryError> {
    if name.0.is_empty() {
        return Err(SqlQueryError::Invalid(
            "empty object name is not supported".to_string(),
        ));
    }
    let mut parts = Vec::with_capacity(name.0.len());
    for part in &name.0 {
        let ident = part.as_ident().ok_or_else(|| {
            SqlQueryError::Unsupported("object name function parts are not supported".to_string())
        })?;
        parts.push(ident.value.clone());
    }
    Ok(parts.join("."))
}

fn rewrite_implicit_collection_sql(sql: &str, collection: &str) -> Option<String> {
    let trimmed = sql.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(after_update) = strip_keyword(trimmed, "update") {
        if starts_with_keyword(after_update, "set") {
            return Some(format!("UPDATE {collection} {after_update}"));
        }
        return None;
    }

    if let Some(after_delete) = strip_keyword(trimmed, "delete") {
        if starts_with_keyword(after_delete, "from") {
            return None;
        }
        return Some(format!("DELETE FROM {collection} {after_delete}"));
    }

    None
}

fn with_query_collection_if_missing(query: Query, collection: String) -> Query {
    match query {
        Query::Select(mut select) => {
            if select.collection.is_none() {
                select.collection = Some(collection);
            }
            Query::Select(select)
        }
        Query::Insert(mut insert) => {
            if insert.collection.is_none() {
                insert.collection = Some(collection);
            }
            Query::Insert(insert)
        }
        Query::Update(mut update) => {
            if update.collection.is_none() {
                update.collection = Some(collection);
            }
            Query::Update(update)
        }
        Query::Delete(mut delete) => {
            if delete.collection.is_none() {
                delete.collection = Some(collection);
            }
            Query::Delete(delete)
        }
    }
}

fn strip_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let trimmed = input.trim_start();
    if !starts_with_keyword(trimmed, keyword) {
        return None;
    }
    let rem = &trimmed[keyword.len()..];
    Some(rem.trim_start())
}

fn starts_with_keyword(input: &str, keyword: &str) -> bool {
    let input = input.trim_start();
    if input.len() < keyword.len() {
        return false;
    }
    let (head, tail) = input.split_at(keyword.len());
    if !head.eq_ignore_ascii_case(keyword) {
        return false;
    }
    match tail.chars().next() {
        None => true,
        Some(ch) => !(ch.is_ascii_alphanumeric() || ch == '_'),
    }
}

fn select_to_sql(query: &SelectQuery, collection: &str) -> Result<String, SqlQueryError> {
    let mut sql = String::new();
    sql.push_str("SELECT ");
    if query.distinct {
        sql.push_str("DISTINCT ");
    }
    if query.projection.is_empty() {
        sql.push('*');
    } else {
        let mut first = true;
        for item in &query.projection {
            if !first {
                sql.push_str(", ");
            }
            first = false;
            sql.push_str(&expr_to_sql(&item.expr)?);
            if let Some(alias) = &item.alias {
                sql.push_str(" AS ");
                sql.push_str(alias);
            }
        }
    }

    sql.push_str(" FROM ");
    sql.push_str(collection);
    if let Some(alias) = &query.source_alias {
        sql.push_str(" AS ");
        sql.push_str(alias);
    }

    for join in &query.joins {
        sql.push(' ');
        sql.push_str(match join.join_type {
            JoinType::Inner => "INNER JOIN ",
            JoinType::Left => "LEFT JOIN ",
            JoinType::Right => "RIGHT JOIN ",
            JoinType::Full => "FULL OUTER JOIN ",
        });
        sql.push_str(&join.source.relation_name());
        if let Some(alias) = &join.alias {
            sql.push_str(" AS ");
            sql.push_str(alias);
        }
        sql.push_str(" ON ");
        let condition_sql = match &join.condition {
            JoinCondition::OnPredicate(predicate) => predicate_to_sql(predicate)?,
            JoinCondition::UsingFields { left, right } => {
                format!("{} = {}", path_to_sql(left)?, path_to_sql(right)?)
            }
        };
        if let Some(predicate) = &join.predicate {
            sql.push_str(&format!(
                "({condition_sql}) AND ({})",
                predicate_to_sql(predicate)?
            ));
        } else {
            sql.push_str(&condition_sql);
        }
    }

    if let Some(predicate) = &query.predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&predicate_to_sql(predicate)?);
    }

    if !query.group_by.is_empty() {
        sql.push_str(" GROUP BY ");
        for (idx, expr) in query.group_by.iter().enumerate() {
            if idx > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&expr_to_sql(expr)?);
        }
    }
    if let Some(having) = &query.having {
        sql.push_str(" HAVING ");
        sql.push_str(&predicate_to_sql(having)?);
    }

    if !query.order_by.is_empty() {
        sql.push_str(" ORDER BY ");
        let mut first = true;
        for item in &query.order_by {
            if !first {
                sql.push_str(", ");
            }
            first = false;
            sql.push_str(&expr_to_sql(&item.expr)?);
            sql.push_str(match item.direction {
                SortDirection::Asc => " ASC",
                SortDirection::Desc => " DESC",
            });
        }
    }

    if let Some(limit) = &query.limit {
        sql.push_str(" LIMIT ");
        sql.push_str(&expr_to_sql(limit)?);
    }
    if evaluate_usize_expr(&query.offset) != Some(0) {
        sql.push_str(" OFFSET ");
        sql.push_str(&expr_to_sql(&query.offset)?);
    }
    Ok(sql)
}

fn insert_to_sql(query: &InsertQuery, collection: &str) -> Result<String, SqlQueryError> {
    let mut sql = String::new();
    sql.push_str("INSERT INTO ");
    sql.push_str(collection);
    if !query.columns.is_empty() {
        sql.push_str(" (");
        sql.push_str(&query.columns.join(", "));
        sql.push(')');
    }
    sql.push(' ');
    match &query.source {
        InsertSource::Objects(_) => {
            return Err(SqlQueryError::Unsupported(
                "object insert source is not representable in SQL text".to_string(),
            ));
        }
        InsertSource::Values(rows) => {
            sql.push_str("VALUES ");
            for (row_idx, row) in rows.iter().enumerate() {
                if row_idx > 0 {
                    sql.push_str(", ");
                }
                sql.push('(');
                for (idx, expr) in row.iter().enumerate() {
                    if idx > 0 {
                        sql.push_str(", ");
                    }
                    sql.push_str(&expr_to_sql(expr)?);
                }
                sql.push(')');
            }
        }
        InsertSource::Select(select) => {
            sql.push_str(&select_to_sql(select, select.collection_or_default())?);
        }
    }
    if !query.returning.is_empty() {
        sql.push_str(" RETURNING ");
        sql.push_str(&projection_to_sql(&query.returning)?);
    }
    Ok(sql)
}

fn update_to_sql(query: &UpdateQuery, collection: &str) -> Result<String, SqlQueryError> {
    let mut sql = String::new();
    sql.push_str("UPDATE ");
    sql.push_str(collection);
    sql.push_str(" SET ");
    let mut first = true;
    for assignment in &query.assignments {
        if !first {
            sql.push_str(", ");
        }
        first = false;
        sql.push_str(&path_to_sql(&assignment.path)?);
        sql.push_str(" = ");
        sql.push_str(&expr_to_sql(&assignment.value)?);
    }
    if let Some(predicate) = &query.predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&predicate_to_sql(predicate)?);
    }
    if let Some(limit) = &query.limit {
        sql.push_str(" LIMIT ");
        sql.push_str(&expr_to_sql(limit)?);
    }
    if !query.returning.is_empty() {
        sql.push_str(" RETURNING ");
        sql.push_str(&projection_to_sql(&query.returning)?);
    }
    Ok(sql)
}

fn delete_to_sql(query: &DeleteQuery, collection: &str) -> Result<String, SqlQueryError> {
    let mut sql = String::new();
    sql.push_str("DELETE FROM ");
    sql.push_str(collection);
    if let Some(predicate) = &query.predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&predicate_to_sql(predicate)?);
    }
    if let Some(limit) = &query.limit {
        sql.push_str(" LIMIT ");
        sql.push_str(&expr_to_sql(limit)?);
    }
    if !query.returning.is_empty() {
        sql.push_str(" RETURNING ");
        sql.push_str(&projection_to_sql(&query.returning)?);
    }
    Ok(sql)
}

fn projection_to_sql(projection: &[QueryField]) -> Result<String, SqlQueryError> {
    if projection.is_empty() {
        return Ok("*".to_string());
    }
    let mut out = String::new();
    let mut first = true;
    for field in projection {
        if !first {
            out.push_str(", ");
        }
        first = false;
        out.push_str(&expr_to_sql(&field.expr)?);
        if let Some(alias) = &field.alias {
            out.push_str(" AS ");
            out.push_str(alias);
        }
    }
    Ok(out)
}

fn predicate_to_sql(predicate: &Predicate) -> Result<String, SqlQueryError> {
    match predicate {
        Predicate::Compare { op, left, right } => Ok(format!(
            "{} {} {}",
            operand_to_sql(left)?,
            compare_to_sql(*op),
            operand_to_sql(right)?
        )),
        Predicate::Expr(expr) => expr_to_sql(expr),
        Predicate::Exists(path) => Ok(path_to_sql(path)?),
        Predicate::And(items) => join_predicates(items, " AND "),
        Predicate::Or(items) => join_predicates(items, " OR "),
        Predicate::Not(inner) => Ok(format!("NOT ({})", predicate_to_sql(inner)?)),
    }
}

fn join_predicates(items: &[Predicate], sep: &str) -> Result<String, SqlQueryError> {
    let mut out = String::new();
    let mut first = true;
    for item in items {
        if !first {
            out.push_str(sep);
        }
        first = false;
        out.push('(');
        out.push_str(&predicate_to_sql(item)?);
        out.push(')');
    }
    Ok(out)
}

fn expr_to_sql(expr: &Expr) -> Result<String, SqlQueryError> {
    match expr {
        Expr::Operand(operand) => operand_to_sql(operand),
        Expr::Unary { op, expr } => Ok(match op {
            UnaryOp::Not => format!("NOT ({})", expr_to_sql(expr)?),
            UnaryOp::Neg => format!("-({})", expr_to_sql(expr)?),
        }),
        Expr::Binary { op, left, right } => Ok(format!(
            "({}) {} ({})",
            expr_to_sql(left)?,
            binary_to_sql(*op),
            expr_to_sql(right)?
        )),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => Ok(format!(
            "CASE WHEN {} THEN {} ELSE {} END",
            expr_to_sql(cond)?,
            expr_to_sql(then_expr)?,
            expr_to_sql(else_expr)?
        )),
        Expr::Coalesce(items) => {
            let mut out = String::from("COALESCE(");
            let mut first = true;
            for item in items {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                out.push_str(&expr_to_sql(item)?);
            }
            out.push(')');
            Ok(out)
        }
        Expr::Function { name, args } => {
            let mut out = String::new();
            out.push_str(name);
            out.push('(');
            for (idx, arg) in args.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&function_arg_to_sql(arg)?);
            }
            out.push(')');
            Ok(out)
        }
        Expr::Aggregate { op, distinct, arg } => {
            let name = match op {
                AggregateOp::Count => "COUNT",
                AggregateOp::Sum => "SUM",
                AggregateOp::Avg => "AVG",
                AggregateOp::Min => "MIN",
                AggregateOp::Max => "MAX",
            };
            let mut out = String::new();
            out.push_str(name);
            out.push('(');
            if *distinct {
                out.push_str("DISTINCT ");
            }
            out.push_str(&function_arg_to_sql(arg)?);
            out.push(')');
            Ok(out)
        }
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let mut out = format!(
                "{} {}IN (",
                expr_to_sql(expr)?,
                if *negated { "NOT " } else { "" }
            );
            for (idx, item) in list.iter().enumerate() {
                if idx > 0 {
                    out.push_str(", ");
                }
                out.push_str(&expr_to_sql(item)?);
            }
            out.push(')');
            Ok(out)
        }
        Expr::InSubquery {
            expr,
            query,
            negated,
        } => Ok(format!(
            "{} {}IN ({})",
            expr_to_sql(expr)?,
            if *negated { "NOT " } else { "" },
            select_to_sql(query, query.collection_or_default())?
        )),
        Expr::Between {
            expr,
            low,
            high,
            negated,
        } => Ok(format!(
            "{} {}BETWEEN {} AND {}",
            expr_to_sql(expr)?,
            if *negated { "NOT " } else { "" },
            expr_to_sql(low)?,
            expr_to_sql(high)?
        )),
        Expr::PatternMatch {
            kind,
            expr,
            pattern,
            case_insensitive,
            negated,
        } => {
            let op = match (kind, *case_insensitive) {
                (PatternMatchKind::Like, false) => "LIKE",
                (PatternMatchKind::Like, true) => "ILIKE",
                (PatternMatchKind::SimilarTo, _) => "SIMILAR TO",
            };
            Ok(format!(
                "{} {}{} {}",
                expr_to_sql(expr)?,
                if *negated { "NOT " } else { "" },
                op,
                expr_to_sql(pattern)?
            ))
        }
        Expr::RegexMatch {
            expr,
            pattern,
            case_insensitive,
            negated,
        } => {
            let op = match (*case_insensitive, *negated) {
                (false, false) => "~",
                (true, false) => "~*",
                (false, true) => "!~",
                (true, true) => "!~*",
            };
            Ok(format!(
                "{} {} {}",
                expr_to_sql(expr)?,
                op,
                expr_to_sql(pattern)?
            ))
        }
        Expr::IsNull { expr, negated } => Ok(format!(
            "{} IS {}NULL",
            expr_to_sql(expr)?,
            if *negated { "NOT " } else { "" }
        )),
        Expr::Exists { query, negated } => Ok(format!(
            "{}EXISTS ({})",
            if *negated { "NOT " } else { "" },
            select_to_sql(query, query.collection_or_default())?
        )),
    }
}

fn function_arg_to_sql(arg: &FunctionArg) -> Result<String, SqlQueryError> {
    match arg {
        FunctionArg::Expr(expr) => expr_to_sql(expr),
        FunctionArg::Wildcard => Ok("*".to_string()),
    }
}

fn operand_to_sql(operand: &Operand) -> Result<String, SqlQueryError> {
    match operand {
        Operand::Field(path) => path_to_sql(path),
        Operand::Literal(value) => value_to_sql(value),
    }
}

fn path_to_sql(path: &FieldPath) -> Result<String, SqlQueryError> {
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

fn value_to_sql(value: &Value) -> Result<String, SqlQueryError> {
    match value {
        Value::Null => Ok("NULL".to_string()),
        Value::Bool(v) => Ok(if *v { "TRUE" } else { "FALSE" }.to_string()),
        Value::I8(v) => Ok(v.to_string()),
        Value::I16(v) => Ok(v.to_string()),
        Value::I32(v) => Ok(v.to_string()),
        Value::I64(v) => Ok(v.to_string()),
        Value::I128(v) => Ok(v.to_string()),
        Value::U8(v) => Ok(v.to_string()),
        Value::U16(v) => Ok(v.to_string()),
        Value::U32(v) => Ok(v.to_string()),
        Value::U64(v) => Ok(v.to_string()),
        Value::U128(v) => Ok(v.to_string()),
        Value::F32(v) => Ok(v.into_inner().to_string()),
        Value::F64(v) => Ok(v.into_inner().to_string()),
        Value::String(v) => Ok(format!("'{}'", v.replace('\'', "''"))),
        other => Err(SqlQueryError::Unsupported(format!(
            "literal value '{other:?}' is not supported in SQL printer"
        ))),
    }
}

fn compare_to_sql(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "=",
        CompareOp::NotEq => "!=",
        CompareOp::Lt => "<",
        CompareOp::Lte => "<=",
        CompareOp::Gt => ">",
        CompareOp::Gte => ">=",
    }
}

fn binary_to_sql(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Concat => "||",
        BinaryOp::And => "AND",
        BinaryOp::Or => "OR",
        BinaryOp::Eq => "=",
        BinaryOp::NotEq => "!=",
        BinaryOp::Lt => "<",
        BinaryOp::Lte => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Gte => ">=",
    }
}

fn dialect_impl(dialect: SqlDialectKind) -> Box<dyn Dialect> {
    match dialect {
        SqlDialectKind::Generic => Box::new(GenericDialect {}),
        SqlDialectKind::PostgreSql => Box::new(PostgreSqlDialect {}),
        SqlDialectKind::MySql => Box::new(MySqlDialect {}),
        SqlDialectKind::SQLite => Box::new(SQLiteDialect {}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_select() {
        let parsed = parse_sql_query(
            "SELECT id, score AS s FROM items WHERE kind = 'music' ORDER BY score DESC LIMIT 5 OFFSET 2",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(select.collection.as_deref(), Some("items"));
        assert_eq!(select.projection.len(), 2);
        assert_eq!(select.order_by.len(), 1);
        assert!(matches!(
            select.limit,
            Some(Expr::Operand(Operand::Literal(Value::I64(5))))
        ));
        assert!(matches!(
            select.offset,
            Expr::Operand(Operand::Literal(Value::I64(2)))
        ));
    }

    #[test]
    fn parse_select_order_by_expression() {
        let parsed = parse_sql_query(
            "SELECT id FROM items ORDER BY score + 1 DESC",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(select.order_by.len(), 1);
        assert!(matches!(
            select.order_by[0].expr,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));

        let sql = query_to_sql(&Query::Select(select)).unwrap();
        assert!(sql.contains("ORDER BY (score) + (1) DESC"));
    }

    #[test]
    fn parse_update_and_print() {
        let parsed = parse_sql_query(
            "UPDATE items SET score = score + 1 WHERE id = 'a' RETURNING score",
            SqlDialectKind::Generic,
        )
        .unwrap();
        assert_eq!(parsed.query.collection(), Some("items"));
        let sql = query_to_sql(&parsed.query).unwrap();
        assert!(sql.starts_with("UPDATE items SET"));
    }

    #[test]
    fn parse_insert_values_and_print() {
        let parsed = parse_sql_query(
            "INSERT INTO items (id, kind, score) VALUES ('a', 'music', 1), ('b', 'video', 2) RETURNING id",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Insert(insert) = parsed.query else {
            panic!("expected insert");
        };
        assert_eq!(insert.collection.as_deref(), Some("items"));
        assert_eq!(insert.columns, vec!["id", "kind", "score"]);
        let InsertSource::Values(rows) = &insert.source else {
            panic!("expected values source");
        };
        assert_eq!(rows.len(), 2);

        let sql = query_to_sql(&Query::Insert(insert)).unwrap();
        assert!(sql.starts_with("INSERT INTO items (id, kind, score) VALUES"));
        assert!(sql.contains("RETURNING id"));
    }

    #[test]
    fn parse_insert_select_source() {
        let parsed = parse_sql_query(
            "INSERT INTO items (id, kind) SELECT id, kind FROM source_items WHERE score > 0",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Insert(insert) = parsed.query else {
            panic!("expected insert");
        };
        let InsertSource::Select(select) = insert.source else {
            panic!("expected select source");
        };
        assert_eq!(select.collection.as_deref(), Some("source_items"));
        assert_eq!(select.projection.len(), 2);
    }

    #[test]
    fn parse_collection_scoped_select_without_from() {
        let query = parse_sql_query_for_collection(
            "SELECT * WHERE kind = 'music'",
            "items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = query else {
            panic!("expected select");
        };
        assert!(select.predicate.is_some());
    }

    #[test]
    fn parse_select_from_all_collection_alias() {
        let query = parse_sql_query("SELECT id FROM all", SqlDialectKind::Generic).unwrap();
        let Query::Select(select) = query.query else {
            panic!("expected select");
        };
        assert_eq!(select.collection.as_deref(), Some("all"));
    }

    #[test]
    fn parse_collection_scoped_update_without_table() {
        let query = parse_sql_query_for_collection(
            "UPDATE SET score = 10 WHERE id = 'a'",
            "items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Update(update) = query else {
            panic!("expected update");
        };
        assert_eq!(update.assignments.len(), 1);
    }

    #[test]
    fn parse_collection_scoped_delete_without_from() {
        let query = parse_sql_query_for_collection(
            "DELETE WHERE id = 'a'",
            "items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Delete(delete) = query else {
            panic!("expected delete");
        };
        assert!(delete.predicate.is_some());
    }

    #[test]
    fn parse_join_source_variants() {
        let parsed = parse_sql_query(
            "SELECT * FROM items AS s JOIN _ AS c ON s.id = c.id JOIN Song AS song ON s.id = song.id JOIN other._ AS o ON s.id = o.id JOIN other.User AS u ON s.id = u.id",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(select.joins.len(), 4);
        assert_eq!(select.joins[0].source.collection.as_deref(), None);
        assert_eq!(select.joins[0].source.class.as_deref(), None);
        assert_eq!(select.joins[1].source.collection.as_deref(), None);
        assert_eq!(select.joins[1].source.class.as_deref(), Some("Song"));
        assert_eq!(select.joins[2].source.collection.as_deref(), Some("other"));
        assert_eq!(select.joins[2].source.class.as_deref(), None);
        assert_eq!(select.joins[3].source.collection.as_deref(), Some("other"));
        assert_eq!(select.joins[3].source.class.as_deref(), Some("User"));
    }
}
