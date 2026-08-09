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
///   - Join operators outside `INNER/LEFT/RIGHT/FULL/CROSS`.
///   - `JOIN ... USING` (coalesced output semantics are not represented).
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
///   - Functions other than the executable aggregate, casing, coalescing, and
///     relationship functions recognized below.
///   - `SIMILAR TO` (SQL regex semantics are not implemented).
///   - `LIKE ANY` / `ILIKE ANY`.
///   - Source-less and correlated subqueries. Fields inside subqueries must be
///     explicitly qualified by an inner source binding because schema-free SQL
///     parsing cannot distinguish an unqualified inner field from an outer ref.
///   - Subqueries in DML expressions.
///   - Unsupported unary/binary operators.
///   - Non-constant DML limit expressions.
///
/// - Ordering / pagination:
///   - `ORDER BY ... NULLS FIRST/LAST`.
///   - `ORDER BY ... WITH FILL`.
///   - Aggregate `ORDER BY` expressions that are not projected (hidden
///     aggregate sort outputs are not represented by the current query AST).
///   - Positional ordering/grouping against wildcard projections.
///   - Bare `GROUP BY` identifiers that match a differently-named output alias;
///     use the projection ordinal or repeat the source expression instead.
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
    AggregateOp, BinaryOp, FieldFormat, JoinType, PatternMatchKind, SortDirection, UnaryOp,
};
use semantic_data::schema::{
    AnyType, AttributeType, BoolType, BytesType, FloatWidth, IntWidth, Meta, NumberType,
    StringType, Type, TypeKind, UIntWidth,
};
use semantic_data::value::{FieldPath, PathSegment, Value};
use sqlparser::ast::{
    Assignment, AssignmentTarget, BinaryOperator, Expr as SqlExpr, FromTable, FunctionArguments,
    Insert as SqlInsert, Join, JoinConstraint, JoinOperator, LimitClause, ObjectName, Offset,
    OrderByExpr, OrderByKind, Query as SqlQuery, Select, SelectItem,
    SelectItemQualifiedWildcardKind, SetExpr, Statement, TableAlias, TableFactor, TableObject,
    TableWithJoins, UnaryOperator, ValueWithSpan, Values,
};
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;
use thiserror::Error;

use crate::{
    DdlBatch, DdlOperation, DdlQuery, DeleteQuery, Expr, FunctionArg, InsertQuery, InsertSource,
    JoinCondition, JoinQuery, JoinSource, Operand, OrderBy as DbOrderBy, Query, QueryField,
    SelectQuery, UpdateQuery, evaluate_usize_expr,
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
    let parsed = parse_sql_query_raw(sql, dialect)?;
    if matches!(&parsed.query, Query::Select(select) if select.collection.is_none()) {
        return Err(SqlQueryError::Invalid(
            "SELECT requires a FROM source outside collection-scoped parsing".to_string(),
        ));
    }
    Ok(parsed)
}

fn parse_sql_query_raw(
    sql: &str,
    dialect: SqlDialectKind,
) -> Result<ParsedSqlQuery, SqlQueryError> {
    if let Some(parsed) = parse_create_attribute_sql(sql)? {
        return Ok(parsed);
    }

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
    let parsed = match parse_sql_query_raw(sql, dialect) {
        Ok(parsed) => parsed,
        Err(SqlQueryError::Parse(_)) => {
            let Some(rewritten) = rewrite_implicit_collection_sql(sql, expected_collection) else {
                return parse_sql_query_raw(sql, dialect).map(|parsed| parsed.query);
            };
            parse_sql_query_raw(&rewritten, dialect)?
        }
        Err(err) => return Err(err),
    };
    if matches!(parsed.query, Query::Ddl(_)) {
        return Err(SqlQueryError::Unsupported(
            "collection-scoped parsing does not support DDL statements".to_string(),
        ));
    }
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
        Query::Ddl(_) => Err(SqlQueryError::Unsupported(
            "DDL AST query is not representable in SQL printer".to_string(),
        )),
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

fn parse_create_attribute_sql(sql: &str) -> Result<Option<ParsedSqlQuery>, SqlQueryError> {
    let Some(after_create) = strip_keyword(sql, "create") else {
        return Ok(None);
    };
    if strip_keyword(after_create, "attribute").is_none() {
        return Ok(None);
    }

    let tokens = tokenize_sql_ddl(sql)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    if tokens.len() < 4 {
        return Ok(None);
    }
    if !tokens[0].eq_ignore_ascii_case("create") || !tokens[1].eq_ignore_ascii_case("attribute") {
        return Ok(None);
    }
    if tokens.len() != 5 {
        return Err(SqlQueryError::Invalid(
            "CREATE ATTRIBUTE expects: CREATE ATTRIBUTE <id> TYPE <type>".to_string(),
        ));
    }
    if !tokens[3].eq_ignore_ascii_case("type") {
        return Err(SqlQueryError::Invalid(
            "CREATE ATTRIBUTE requires TYPE clause".to_string(),
        ));
    }
    let id = tokens[2].clone();
    let ty = parse_sql_attribute_type(&tokens[4])?;
    Ok(Some(ParsedSqlQuery {
        query: Query::Ddl(DdlQuery {
            batch: DdlBatch::new().with_op(DdlOperation::UpsertAttribute {
                attribute: AttributeType {
                    id: id.clone(),
                    name: id,
                    ty,
                    constraints: vec![],
                    meta: Meta::default(),
                },
            }),
        }),
    }))
}

fn tokenize_sql_ddl(sql: &str) -> Result<Vec<String>, SqlQueryError> {
    let mut tokens = Vec::new();
    let mut chars = sql.trim().chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if ch == ';' {
            chars.next();
            while let Some(rem) = chars.peek().copied() {
                if rem.is_whitespace() {
                    chars.next();
                } else {
                    return Err(SqlQueryError::Invalid(
                        "unexpected content after ';'".to_string(),
                    ));
                }
            }
            break;
        }
        if ch == '"' {
            chars.next();
            let mut value = String::new();
            let mut closed = false;
            while let Some(next) = chars.next() {
                if next == '"' {
                    if matches!(chars.peek(), Some('"')) {
                        chars.next();
                        value.push('"');
                        continue;
                    }
                    closed = true;
                    break;
                }
                value.push(next);
            }
            if !closed {
                return Err(SqlQueryError::Invalid(
                    "unterminated quoted identifier".to_string(),
                ));
            }
            tokens.push(value);
            continue;
        }
        let mut value = String::new();
        while let Some(next) = chars.peek().copied() {
            if next.is_whitespace() || next == ';' {
                break;
            }
            value.push(next);
            chars.next();
        }
        if !value.is_empty() {
            tokens.push(value);
        }
    }
    Ok(tokens)
}

fn parse_sql_attribute_type(value: &str) -> Result<Type, SqlQueryError> {
    let kind = if value.eq_ignore_ascii_case("bool") || value.eq_ignore_ascii_case("boolean") {
        TypeKind::Bool(BoolType)
    } else if value.eq_ignore_ascii_case("string") || value.eq_ignore_ascii_case("text") {
        TypeKind::String(StringType {
            format: None,
            normalization: None,
        })
    } else if value.eq_ignore_ascii_case("bytes") {
        TypeKind::Bytes(BytesType { encoding: None })
    } else if value.eq_ignore_ascii_case("any") {
        TypeKind::Any(AnyType)
    } else if value.eq_ignore_ascii_case("uuid") {
        TypeKind::Uuid
    } else if value.eq_ignore_ascii_case("json") {
        TypeKind::Json
    } else if value.eq_ignore_ascii_case("i8") {
        TypeKind::Number(NumberType::Int(IntWidth::I8))
    } else if value.eq_ignore_ascii_case("i16") {
        TypeKind::Number(NumberType::Int(IntWidth::I16))
    } else if value.eq_ignore_ascii_case("i32") {
        TypeKind::Number(NumberType::Int(IntWidth::I32))
    } else if value.eq_ignore_ascii_case("i64") {
        TypeKind::Number(NumberType::Int(IntWidth::I64))
    } else if value.eq_ignore_ascii_case("u8") {
        TypeKind::Number(NumberType::UInt(UIntWidth::U8))
    } else if value.eq_ignore_ascii_case("u16") {
        TypeKind::Number(NumberType::UInt(UIntWidth::U16))
    } else if value.eq_ignore_ascii_case("u32") {
        TypeKind::Number(NumberType::UInt(UIntWidth::U32))
    } else if value.eq_ignore_ascii_case("u64") {
        TypeKind::Number(NumberType::UInt(UIntWidth::U64))
    } else if value.eq_ignore_ascii_case("f32") {
        TypeKind::Number(NumberType::Float(FloatWidth::F32))
    } else if value.eq_ignore_ascii_case("f64") {
        TypeKind::Number(NumberType::Float(FloatWidth::F64))
    } else {
        return Err(SqlQueryError::Unsupported(format!(
            "unsupported CREATE ATTRIBUTE type '{value}'"
        )));
    };

    Ok(Type {
        kind,
        constraints: vec![],
        annotations: vec![],
    })
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
    if query.settings.is_some() || !query.pipe_operators.is_empty() {
        return Err(SqlQueryError::Unsupported(
            "query settings/format/pipe operators are not supported".to_string(),
        ));
    }
    let field_format = query
        .format_clause
        .as_ref()
        .map(|format_clause| parse_field_format_clause(&format!("{format_clause}")))
        .transpose()?
        .unwrap_or(FieldFormat::Plain);

    let SetExpr::Select(select) = *query.body else {
        return Err(SqlQueryError::Unsupported(
            "only SELECT query bodies are supported".to_string(),
        ));
    };
    parse_select(query.order_by, query.limit_clause, *select, field_format)
}

fn parse_select_subquery(query: SqlQuery) -> Result<SelectQuery, SqlQueryError> {
    let parsed = parse_select_stmt(query)?;
    match parsed.query {
        Query::Select(select) => {
            validate_subquery_scope(&select)?;
            Ok(select)
        }
        _ => Err(SqlQueryError::Invalid(
            "expected SELECT subquery expression".to_string(),
        )),
    }
}

fn validate_subquery_scope(select: &SelectQuery) -> Result<(), SqlQueryError> {
    let collection = select.collection.as_ref().ok_or_else(|| {
        SqlQueryError::Unsupported("source-less subqueries are not supported".to_string())
    })?;
    let mut bindings = Vec::new();
    if let Some(alias) = &select.source_alias {
        // SQL aliases hide the original relation name within the query block.
        bindings.push(alias.clone());
    } else {
        bindings.push(collection.clone());
        if let Some(last) = collection.rsplit('.').next()
            && last != collection
        {
            bindings.push(last.to_string());
        }
    }
    for join in &select.joins {
        bindings.push(
            join.alias
                .clone()
                .unwrap_or_else(|| join.source.default_binding()),
        );
    }

    if select_has_scope_path(
        select,
        &|path| path_has_external_binding(path, &bindings),
        &|path| wildcard_has_external_binding(path, &bindings),
    ) {
        return Err(SqlQueryError::Unsupported(
            "correlated subqueries are not supported".to_string(),
        ));
    }
    if select_has_scope_path(select, &path_is_unqualified_field, &|_| false) {
        return Err(SqlQueryError::Unsupported(
            "unqualified field references in subqueries are not supported; qualify fields with the inner source binding"
                .to_string(),
        ));
    }
    Ok(())
}

fn path_has_external_binding(path: &FieldPath, bindings: &[String]) -> bool {
    path.segments().len() >= 2
        && !bindings
            .iter()
            .any(|binding| path_starts_with_binding(path, binding))
}

fn wildcard_has_external_binding(path: &FieldPath, bindings: &[String]) -> bool {
    !path.segments().is_empty()
        && !bindings
            .iter()
            .any(|binding| path_starts_with_binding(path, binding))
}

fn path_starts_with_binding(path: &FieldPath, binding: &str) -> bool {
    let mut segments = path.segments().iter();
    binding
        .split('.')
        .all(|part| matches!(segments.next(), Some(PathSegment::Field(field)) if field == part))
}

fn path_is_unqualified_field(path: &FieldPath) -> bool {
    matches!(path.segments(), [PathSegment::Field(_)])
}

fn select_has_scope_path(
    select: &SelectQuery,
    expr_path_matches: &impl Fn(&FieldPath) -> bool,
    wildcard_path_matches: &impl Fn(&FieldPath) -> bool,
) -> bool {
    select.projection.iter().any(|field| {
        expr_has_scope_path(&field.expr, expr_path_matches)
            || field.wildcard.as_ref().is_some_and(wildcard_path_matches)
    }) || select
        .predicate
        .as_ref()
        .is_some_and(|expr| expr_has_scope_path(expr, expr_path_matches))
        || select
            .group_by
            .iter()
            .any(|expr| expr_has_scope_path(expr, expr_path_matches))
        || select
            .having
            .as_ref()
            .is_some_and(|expr| expr_has_scope_path(expr, expr_path_matches))
        || select
            .order_by
            .iter()
            .any(|order| expr_has_scope_path(&order.expr, expr_path_matches))
        || select.joins.iter().any(|join| {
            let condition_matches = match &join.condition {
                JoinCondition::OnExpr(expr) => expr_has_scope_path(expr, expr_path_matches),
                JoinCondition::UsingFields { left, right } => {
                    expr_path_matches(left) || expr_path_matches(right)
                }
            };
            condition_matches
                || join
                    .predicate
                    .as_ref()
                    .is_some_and(|expr| expr_has_scope_path(expr, expr_path_matches))
        })
}

fn expr_has_scope_path(expr: &Expr, path_matches: &impl Fn(&FieldPath) -> bool) -> bool {
    match expr {
        Expr::Operand(Operand::Field(path)) => path_matches(path),
        Expr::Operand(Operand::Literal(_)) | Expr::Subquery(_) | Expr::Exists { .. } => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => {
            expr_has_scope_path(expr, path_matches)
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
        } => expr_has_scope_path(left, path_matches) || expr_has_scope_path(right, path_matches),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_has_scope_path(cond, path_matches)
                || expr_has_scope_path(then_expr, path_matches)
                || expr_has_scope_path(else_expr, path_matches)
        }
        Expr::Coalesce(items) => items
            .iter()
            .any(|expr| expr_has_scope_path(expr, path_matches)),
        Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            FunctionArg::Expr(expr) => expr_has_scope_path(expr, path_matches),
            FunctionArg::Wildcard => false,
        }),
        Expr::Aggregate { arg, .. } => match arg.as_ref() {
            FunctionArg::Expr(expr) => expr_has_scope_path(expr, path_matches),
            FunctionArg::Wildcard => false,
        },
        Expr::InList { expr, list, .. } => {
            expr_has_scope_path(expr, path_matches)
                || list
                    .iter()
                    .any(|expr| expr_has_scope_path(expr, path_matches))
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_has_scope_path(expr, path_matches)
                || expr_has_scope_path(low, path_matches)
                || expr_has_scope_path(high, path_matches)
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            expr_has_scope_path(relation, path_matches)
                || expr_has_scope_path(source, path_matches)
                || expr_has_scope_path(target, path_matches)
                || max_depth
                    .as_deref()
                    .is_some_and(|expr| expr_has_scope_path(expr, path_matches))
        }
    }
}

fn parse_select(
    order_by: Option<sqlparser::ast::OrderBy>,
    limit_clause: Option<LimitClause>,
    select: Select,
    field_format: FieldFormat,
) -> Result<ParsedSqlQuery, SqlQueryError> {
    let distinct = parse_select_distinct(select.distinct.as_ref())?;
    if select.optimizer_hint.is_some()
        || select.select_modifiers.is_some()
        || select.top.is_some()
        || select.top_before_distinct
        || select.flavor != sqlparser::ast::SelectFlavor::Standard
        || select.window_before_qualify
        || select.into.is_some()
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
    let projection = parse_projection(select.projection, true)?;
    let predicate = select.selection.map(parse_expr).transpose()?;
    let group_by = parse_group_by(select.group_by, &projection)?;
    let having = select.having.map(parse_expr).transpose()?;
    let (limit, offset) = parse_limit_clause(limit_clause)?;
    let group_bindings = base_group_bindings(
        collection.as_deref(),
        source_alias.as_deref(),
        joins.is_empty(),
    );
    let order_by = validate_and_resolve_select_semantics(
        &projection,
        predicate.as_ref(),
        &joins,
        &group_by,
        having.as_ref(),
        limit.as_ref(),
        &offset,
        &group_bindings,
        parse_order_by(order_by)?,
    )?;

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
            field_format,
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

fn parse_group_by(
    group_by: sqlparser::ast::GroupByExpr,
    projection: &[QueryField],
) -> Result<Vec<Expr>, SqlQueryError> {
    match group_by {
        sqlparser::ast::GroupByExpr::Expressions(exprs, modifiers) if modifiers.is_empty() => exprs
            .into_iter()
            .map(parse_expr)
            .map(|expr| expr.and_then(|expr| resolve_group_by_expr(expr, projection)))
            .collect(),
        other => Err(SqlQueryError::Unsupported(format!(
            "GROUP BY form '{other:?}' is not supported"
        ))),
    }
}

fn resolve_group_by_expr(expr: Expr, projection: &[QueryField]) -> Result<Expr, SqlQueryError> {
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
        .map(|items| parse_projection(items, false))
        .transpose()?
        .unwrap_or_default();
    validate_dml_projection(&returning)?;
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
            field_format: FieldFormat::Plain,
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
            let parsed = parse_select(order_by, limit_clause, *select, FieldFormat::Plain)?;
            let Query::Select(select_query) = parsed.query else {
                return Err(SqlQueryError::Invalid(
                    "failed to parse INSERT SELECT source".to_string(),
                ));
            };
            if select_query.collection.is_none() {
                return Err(SqlQueryError::Invalid(
                    "INSERT SELECT source requires a FROM source".to_string(),
                ));
            }
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
                .map(parse_dml_expr)
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(InsertSource::Values(rows))
}

fn parse_update_stmt(update: sqlparser::ast::Update) -> Result<ParsedSqlQuery, SqlQueryError> {
    if update.optimizer_hint.is_some() {
        return Err(SqlQueryError::Unsupported(
            "UPDATE optimizer hints are not supported".to_string(),
        ));
    }
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
    if table_alias(&update.table.relation).is_some() {
        return Err(SqlQueryError::Unsupported(
            "UPDATE target aliases are not supported".to_string(),
        ));
    }
    let collection = parse_base_table_name(&update.table.relation)?;
    let assignments = update
        .assignments
        .into_iter()
        .map(parse_assignment)
        .collect::<Result<Vec<_>, _>>()?;
    let predicate = update.selection.map(parse_dml_expr).transpose()?;
    let returning = update
        .returning
        .map(|items| parse_projection(items, false))
        .transpose()?
        .unwrap_or_default();
    validate_dml_projection(&returning)?;
    let limit = parse_dml_limit(update.limit)?;

    Ok(ParsedSqlQuery {
        query: Query::Update(UpdateQuery {
            collection: Some(collection),
            predicate,
            assignments,
            limit,
            returning,
            field_format: FieldFormat::Plain,
        }),
    })
}

fn parse_delete_stmt(delete: sqlparser::ast::Delete) -> Result<ParsedSqlQuery, SqlQueryError> {
    if delete.optimizer_hint.is_some() {
        return Err(SqlQueryError::Unsupported(
            "DELETE optimizer hints are not supported".to_string(),
        ));
    }
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
    if table_alias(&table.relation).is_some() {
        return Err(SqlQueryError::Unsupported(
            "DELETE target aliases are not supported".to_string(),
        ));
    }
    let collection = parse_base_table_name(&table.relation)?;
    let predicate = delete.selection.map(parse_dml_expr).transpose()?;
    let returning = delete
        .returning
        .map(|items| parse_projection(items, false))
        .transpose()?
        .unwrap_or_default();
    validate_dml_projection(&returning)?;
    let limit = parse_dml_limit(delete.limit)?;

    Ok(ParsedSqlQuery {
        query: Query::Delete(DeleteQuery {
            collection: Some(collection),
            predicate,
            limit,
            returning,
            field_format: FieldFormat::Plain,
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
    if join.global {
        return Err(SqlQueryError::Unsupported(
            "GLOBAL JOIN is not supported".to_string(),
        ));
    }
    let source = parse_join_source(&join.relation)?;
    let alias = table_alias(&join.relation);

    let (join_type, constraint, cross_join) = match join.join_operator {
        JoinOperator::Inner(c) | JoinOperator::Join(c) => (JoinType::Inner, c, false),
        JoinOperator::Left(c) | JoinOperator::LeftOuter(c) => (JoinType::Left, c, false),
        JoinOperator::Right(c) | JoinOperator::RightOuter(c) => (JoinType::Right, c, false),
        JoinOperator::FullOuter(c) => (JoinType::Full, c, false),
        JoinOperator::CrossJoin(JoinConstraint::None) => {
            (JoinType::Inner, JoinConstraint::None, true)
        }
        JoinOperator::CrossJoin(other) => {
            return Err(SqlQueryError::Unsupported(format!(
                "CROSS JOIN constraint '{other:?}' is not supported"
            )));
        }
        other => {
            return Err(SqlQueryError::Unsupported(format!(
                "join operator '{other:?}' is not supported"
            )));
        }
    };

    let condition = match constraint {
        JoinConstraint::On(expr) => JoinCondition::OnExpr(parse_expr(expr)?),
        JoinConstraint::Using(_) => {
            return Err(SqlQueryError::Unsupported(
                "JOIN USING is not supported because coalesced output semantics are not represented"
                    .to_string(),
            ));
        }
        JoinConstraint::Natural => {
            return Err(SqlQueryError::Unsupported(
                "NATURAL JOIN is not supported".to_string(),
            ));
        }
        JoinConstraint::None => {
            if cross_join {
                JoinCondition::OnExpr(Expr::Operand(Operand::Literal(Value::Bool(true))))
            } else {
                return Err(SqlQueryError::Unsupported(
                    "JOIN without a constraint is not supported".to_string(),
                ));
            }
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

fn parse_projection(
    items: Vec<SelectItem>,
    collapse_single_unqualified_wildcard: bool,
) -> Result<Vec<QueryField>, SqlQueryError> {
    if collapse_single_unqualified_wildcard
        && items.len() == 1
        && let SelectItem::Wildcard(options) = &items[0]
    {
        validate_wildcard_options(options)?;
        return Ok(Vec::new());
    }

    items
        .into_iter()
        .map(|item| match item {
            SelectItem::UnnamedExpr(expr) => Ok(QueryField {
                expr: Box::new(parse_expr(expr)?),
                alias: None,
                wildcard: None,
            }),
            SelectItem::ExprWithAlias { expr, alias } => Ok(QueryField {
                expr: Box::new(parse_expr(expr)?),
                alias: Some(alias.value),
                wildcard: None,
            }),
            SelectItem::QualifiedWildcard(
                SelectItemQualifiedWildcardKind::ObjectName(name),
                options,
            ) => {
                validate_wildcard_options(&options)?;
                let path = object_name_to_path(&name)?;
                Ok(QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(path.clone()))),
                    alias: None,
                    wildcard: Some(path),
                })
            }
            SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::Expr(_), _) => {
                Err(SqlQueryError::Unsupported(
                    "expression-qualified wildcards are not supported".to_string(),
                ))
            }
            SelectItem::Wildcard(options) => {
                validate_wildcard_options(&options)?;
                Ok(QueryField {
                    expr: Box::new(Expr::Operand(Operand::Literal(Value::Null))),
                    alias: None,
                    wildcard: Some(FieldPath::new()),
                })
            }
        })
        .collect()
}

fn validate_wildcard_options(
    options: &sqlparser::ast::WildcardAdditionalOptions,
) -> Result<(), SqlQueryError> {
    if options.opt_ilike.is_some()
        || options.opt_exclude.is_some()
        || options.opt_except.is_some()
        || options.opt_replace.is_some()
        || options.opt_rename.is_some()
    {
        return Err(SqlQueryError::Unsupported(
            "wildcard projection modifiers are not supported".to_string(),
        ));
    }
    Ok(())
}

fn parse_order_by(
    order_by: Option<sqlparser::ast::OrderBy>,
) -> Result<Vec<DbOrderBy>, SqlQueryError> {
    let Some(order_by) = order_by else {
        return Ok(Vec::new());
    };
    if order_by.interpolate.is_some() {
        return Err(SqlQueryError::Unsupported(
            "ORDER BY INTERPOLATE is not supported".to_string(),
        ));
    }
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

fn validate_and_resolve_select_semantics(
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

fn validate_unique_projection_keys(projection: &[QueryField]) -> Result<(), SqlQueryError> {
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

fn base_group_bindings(
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

fn projection_output_key(field: &QueryField) -> Option<String> {
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

fn field_expr_final_name(expr: &Expr) -> Option<&str> {
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

fn resolve_nonaggregate_order(
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

fn resolve_aggregate_order(
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

fn order_ordinal(expr: &Expr) -> Option<usize> {
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

fn single_field_name(expr: &Expr) -> Option<&str> {
    let Expr::Operand(Operand::Field(path)) = expr else {
        return None;
    };
    let [PathSegment::Field(name)] = path.segments() else {
        return None;
    };
    Some(name)
}

fn expr_contains_nested_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Aggregate { arg, .. } => match arg.as_ref() {
            FunctionArg::Expr(expr) => expr_contains_aggregate(expr),
            FunctionArg::Wildcard => false,
        },
        Expr::Operand(_) | Expr::Subquery(_) | Expr::Exists { .. } => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => {
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

fn group_expr_equivalent(left: &Expr, right: &Expr, bindings: &[String]) -> bool {
    if left == right {
        return true;
    }
    normalize_group_expr(left, bindings) == normalize_group_expr(right, bindings)
}

fn normalize_group_expr(expr: &Expr, bindings: &[String]) -> Expr {
    let normalize = |expr: &Expr| normalize_group_expr(expr, bindings);
    match expr {
        Expr::Operand(Operand::Field(path)) => {
            Expr::Operand(Operand::Field(normalize_group_path(path, bindings)))
        }
        Expr::Operand(Operand::Literal(value)) => Expr::Operand(Operand::Literal(value.clone())),
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

fn normalize_group_path(path: &FieldPath, bindings: &[String]) -> FieldPath {
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

fn validate_grouped_expr(
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
        Expr::Operand(Operand::Literal(_)) | Expr::Subquery(_) | Expr::Exists { .. } => Ok(()),
        Expr::Operand(Operand::Field(path)) => Err(SqlQueryError::Invalid(format!(
            "field '{}' in {clause} must appear in GROUP BY or be inside an aggregate",
            path_to_sql(path).unwrap_or_else(|_| format!("{path:?}"))
        ))),
        Expr::Aggregate { .. } => Ok(()),
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => {
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

fn parse_dml_limit(limit: Option<SqlExpr>) -> Result<Option<Expr>, SqlQueryError> {
    let limit = limit.map(parse_dml_expr).transpose()?;
    if limit
        .as_ref()
        .is_some_and(|expr| evaluate_usize_expr(expr).is_none())
    {
        return Err(SqlQueryError::Invalid(
            "DML LIMIT must be a non-negative constant integer".to_string(),
        ));
    }
    Ok(limit)
}

fn parse_dml_expr(expr: SqlExpr) -> Result<Expr, SqlQueryError> {
    let expr = parse_expr(expr)?;
    if expr_contains_subquery(&expr) {
        return Err(SqlQueryError::Unsupported(
            "subqueries are not supported in DML expressions".to_string(),
        ));
    }
    if expr_contains_aggregate(&expr) {
        return Err(SqlQueryError::Unsupported(
            "aggregate expressions are not supported in DML expressions".to_string(),
        ));
    }
    Ok(expr)
}

fn validate_dml_projection(projection: &[QueryField]) -> Result<(), SqlQueryError> {
    if projection
        .iter()
        .any(|field| expr_contains_subquery(&field.expr))
    {
        return Err(SqlQueryError::Unsupported(
            "subqueries are not supported in DML RETURNING expressions".to_string(),
        ));
    }
    if projection
        .iter()
        .any(|field| expr_contains_aggregate(&field.expr))
    {
        return Err(SqlQueryError::Unsupported(
            "aggregate expressions are not supported in DML RETURNING expressions".to_string(),
        ));
    }
    Ok(())
}

fn expr_contains_subquery(expr: &Expr) -> bool {
    match expr {
        Expr::Subquery(_) | Expr::Exists { .. } => true,
        Expr::Operand(_) => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => expr_contains_subquery(expr),
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
        } => expr_contains_subquery(left) || expr_contains_subquery(right),
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            expr_contains_subquery(cond)
                || expr_contains_subquery(then_expr)
                || expr_contains_subquery(else_expr)
        }
        Expr::Coalesce(items) => items.iter().any(expr_contains_subquery),
        Expr::InList { expr, list, .. } => {
            expr_contains_subquery(expr) || list.iter().any(expr_contains_subquery)
        }
        Expr::Function { args, .. } => args.iter().any(|arg| match arg {
            FunctionArg::Expr(expr) => expr_contains_subquery(expr),
            FunctionArg::Wildcard => false,
        }),
        Expr::Aggregate { arg, .. } => match arg.as_ref() {
            FunctionArg::Expr(expr) => expr_contains_subquery(expr),
            FunctionArg::Wildcard => false,
        },
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_subquery(expr)
                || expr_contains_subquery(low)
                || expr_contains_subquery(high)
        }
        Expr::RelationExists {
            relation,
            source,
            target,
            max_depth,
            ..
        } => {
            expr_contains_subquery(relation)
                || expr_contains_subquery(source)
                || expr_contains_subquery(target)
                || max_depth.as_deref().is_some_and(expr_contains_subquery)
        }
    }
}

fn expr_contains_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::Aggregate { .. } => true,
        Expr::Operand(_) | Expr::Subquery(_) | Expr::Exists { .. } => false,
        Expr::Unary { expr, .. } | Expr::IsNull { expr, .. } => expr_contains_aggregate(expr),
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
        value: parse_dml_expr(assign.value)?,
    })
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
        } => {
            let in_expr = Expr::Binary {
                op: BinaryOp::In,
                left: Box::new(parse_expr(*expr)?),
                right: Box::new(Expr::Subquery(Box::new(parse_select_subquery(*subquery)?))),
            };
            if negated {
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(in_expr),
                })
            } else {
                Ok(in_expr)
            }
        }
        SqlExpr::Subquery(subquery) => {
            Ok(Expr::Subquery(Box::new(parse_select_subquery(*subquery)?)))
        }
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
            any,
            expr,
            pattern,
            escape_char,
        } => {
            if any {
                return Err(SqlQueryError::Unsupported(
                    "LIKE ANY is not supported".to_string(),
                ));
            }
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
            any,
            expr,
            pattern,
            escape_char,
        } => {
            if any {
                return Err(SqlQueryError::Unsupported(
                    "ILIKE ANY is not supported".to_string(),
                ));
            }
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
        SqlExpr::SimilarTo { .. } => Err(SqlQueryError::Unsupported(
            "SIMILAR TO is not supported because its SQL semantics are not implemented".to_string(),
        )),
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
            if conditions.is_empty() {
                return Err(SqlQueryError::Unsupported(
                    "CASE requires at least one WHEN branch".to_string(),
                ));
            }
            let operand = operand.map(|operand| parse_expr(*operand)).transpose()?;
            let mut lowered = else_result
                .map(|expr| parse_expr(*expr))
                .transpose()?
                .unwrap_or_else(|| Expr::Operand(Operand::Literal(Value::Null)));
            for when in conditions.into_iter().rev() {
                let condition = parse_expr(when.condition)?;
                let condition = if let Some(operand) = &operand {
                    Expr::Binary {
                        op: BinaryOp::Eq,
                        left: Box::new(operand.clone()),
                        right: Box::new(condition),
                    }
                } else {
                    condition
                };
                lowered = Expr::IfElse {
                    cond: Box::new(condition),
                    then_expr: Box::new(parse_expr(when.result)?),
                    else_expr: Box::new(lowered),
                };
            }
            Ok(lowered)
        }
        other => Err(SqlQueryError::Unsupported(format!(
            "expression '{other}' is not supported"
        ))),
    }
}

fn parse_function_expr(function: sqlparser::ast::Function) -> Result<Expr, SqlQueryError> {
    if function.uses_odbc_syntax
        || !matches!(function.parameters, FunctionArguments::None)
        || function.null_treatment.is_some()
        || function.over.is_some()
        || !function.within_group.is_empty()
        || function.filter.is_some()
    {
        return Err(SqlQueryError::Unsupported(format!(
            "parameters, ODBC syntax, and execution modifiers are not supported for function '{}'",
            function.name
        )));
    }
    if function.name.0.len() != 1 || function.name.0[0].as_ident().is_none() {
        return Err(SqlQueryError::Unsupported(
            "qualified or function-style function names are not supported".to_string(),
        ));
    }
    let function_name = function.name.to_string();
    let mut args = Vec::new();
    let FunctionArguments::List(argument_list) = function.args else {
        return Err(SqlQueryError::Unsupported(format!(
            "function '{}' argument form is not supported",
            function.name
        )));
    };
    if !argument_list.clauses.is_empty() {
        return Err(SqlQueryError::Unsupported(format!(
            "function '{}' argument clauses are not supported",
            function.name
        )));
    }
    let duplicate_treatment = argument_list.duplicate_treatment;
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
        duplicate_treatment,
        Some(sqlparser::ast::DuplicateTreatment::Distinct)
    );
    if let Some(op) = aggregate_op_from_name(&function_name) {
        if args.len() != 1 {
            return Err(SqlQueryError::Unsupported(format!(
                "aggregate '{}' expects exactly one argument",
                function.name
            )));
        }
        let arg = args.into_iter().next().expect("checked len");
        if matches!(arg, FunctionArg::Wildcard) && !matches!(op, AggregateOp::Count) {
            return Err(SqlQueryError::Unsupported(format!(
                "aggregate '{}' does not support a wildcard argument",
                function.name
            )));
        }
        if distinct && matches!(arg, FunctionArg::Wildcard) {
            return Err(SqlQueryError::Unsupported(
                "COUNT(DISTINCT *) is not supported".to_string(),
            ));
        }
        return Ok(Expr::Aggregate {
            op,
            distinct,
            arg: Box::new(arg),
        });
    }
    if duplicate_treatment.is_some() {
        return Err(SqlQueryError::Unsupported(format!(
            "duplicate treatment is only supported for aggregate functions, not '{}'",
            function.name
        )));
    }
    if function_name.eq_ignore_ascii_case("coalesce") {
        if args.is_empty() {
            return Err(SqlQueryError::Unsupported(
                "COALESCE expects at least one argument".to_string(),
            ));
        }
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
    } else if function_name.eq_ignore_ascii_case("lower")
        || function_name.eq_ignore_ascii_case("upper")
    {
        if args.len() != 1 || !matches!(args.first(), Some(FunctionArg::Expr(_))) {
            return Err(SqlQueryError::Unsupported(format!(
                "function '{}' expects exactly one expression argument",
                function.name
            )));
        }
        Ok(Expr::Function {
            name: function_name,
            args,
        })
    } else if function_name.eq_ignore_ascii_case("has_relation") {
        parse_relation_function_expr(args, false)
    } else if function_name.eq_ignore_ascii_case("has_relation_path") {
        parse_relation_function_expr(args, true)
    } else {
        Err(SqlQueryError::Unsupported(format!(
            "function '{}' is not executable",
            function.name
        )))
    }
}

fn parse_relation_function_expr(
    args: Vec<FunctionArg>,
    transitive: bool,
) -> Result<Expr, SqlQueryError> {
    if args.len() < 3 || args.len() > 4 {
        return Err(SqlQueryError::Unsupported(
            "relationship function expects 3 or 4 arguments".to_string(),
        ));
    }
    let mut args = args.into_iter();
    let take_expr = |arg: FunctionArg| -> Result<Expr, SqlQueryError> {
        match arg {
            FunctionArg::Expr(expr) => Ok(expr),
            FunctionArg::Wildcard => Err(SqlQueryError::Unsupported(
                "relationship function does not support wildcard args".to_string(),
            )),
        }
    };
    let relation = take_expr(args.next().expect("validated len"))?;
    let source = take_expr(args.next().expect("validated len"))?;
    let target = take_expr(args.next().expect("validated len"))?;
    let max_depth = args.next().map(take_expr).transpose()?.map(Box::new);
    if !transitive && max_depth.is_some() {
        return Err(SqlQueryError::Unsupported(
            "has_relation does not support max_depth; use has_relation_path".to_string(),
        ));
    }
    Ok(Expr::RelationExists {
        relation: Box::new(relation),
        source: Box::new(source),
        target: Box::new(target),
        transitive,
        max_depth,
    })
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

fn parse_field_format_clause(clause: &str) -> Result<FieldFormat, SqlQueryError> {
    let normalized = clause.trim().to_ascii_lowercase();
    let value = normalized
        .strip_prefix("format")
        .map(str::trim)
        .unwrap_or(normalized.as_str());
    match value {
        "qualified" => Ok(FieldFormat::Qualified),
        "underscore" => Ok(FieldFormat::Underscore),
        "plain" => Ok(FieldFormat::Plain),
        _ => Err(SqlQueryError::Unsupported(format!(
            "unsupported output field format '{clause}'"
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
        TableFactor::Table {
            name,
            alias,
            args,
            with_hints,
            version,
            with_ordinality,
            partitions,
            json_path,
            sample,
            index_hints,
        } => {
            if alias
                .as_ref()
                .is_some_and(|alias| !alias.columns.is_empty())
                || args.is_some()
                || !with_hints.is_empty()
                || version.is_some()
                || *with_ordinality
                || !partitions.is_empty()
                || json_path.is_some()
                || sample.is_some()
                || !index_hints.is_empty()
            {
                return Err(SqlQueryError::Unsupported(
                    "table functions, alias columns, hints, versions, partitions, JSON paths, sampling, and index hints are not supported"
                        .to_string(),
                ));
            }
            object_name_to_string(name)
        }
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
        Query::Ddl(ddl) => Query::Ddl(ddl),
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
    sql.push_str(&projection_to_sql(&query.projection)?);

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
            JoinCondition::OnExpr(predicate) => expr_to_sql(predicate)?,
            JoinCondition::UsingFields { left, right } => {
                format!("{} = {}", path_to_sql(left)?, path_to_sql(right)?)
            }
        };
        if let Some(predicate) = &join.predicate {
            sql.push_str(&format!(
                "({condition_sql}) AND ({})",
                expr_to_sql(predicate)?
            ));
        } else {
            sql.push_str(&condition_sql);
        }
    }

    if let Some(predicate) = &query.predicate {
        sql.push_str(" WHERE ");
        sql.push_str(&expr_to_sql(predicate)?);
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
        sql.push_str(&expr_to_sql(having)?);
    }

    if !query.order_by.is_empty() {
        sql.push_str(" ORDER BY ");
        let mut first = true;
        for item in &query.order_by {
            if !first {
                sql.push_str(", ");
            }
            first = false;
            sql.push_str(&select_order_expr_to_sql(query, &item.expr)?);
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
    if query.field_format != FieldFormat::Plain {
        let format_name = match query.field_format {
            FieldFormat::Qualified => "qualified",
            FieldFormat::Underscore => "underscore",
            FieldFormat::Plain => "plain",
        };
        sql.push_str(" FORMAT ");
        sql.push_str(format_name);
    }
    Ok(sql)
}

fn select_order_expr_to_sql(query: &SelectQuery, expr: &Expr) -> Result<String, SqlQueryError> {
    let aggregate_query = !query.group_by.is_empty()
        || query.having.is_some()
        || query
            .projection
            .iter()
            .any(|field| expr_contains_aggregate(&field.expr));
    if aggregate_query && let Some(name) = single_field_name(expr) {
        let mut matches = query.projection.iter().filter(|field| {
            field.alias.is_none() && projection_output_key(field).as_deref() == Some(name)
        });
        if let Some(field) = matches.next()
            && matches.next().is_none()
        {
            return expr_to_sql(&field.expr);
        }
    }
    expr_to_sql(expr)
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
        sql.push_str(&expr_to_sql(predicate)?);
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
        sql.push_str(&expr_to_sql(predicate)?);
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
        if let Some(path) = &field.wildcard {
            if path.segments().is_empty()
                && !matches!(field.expr.as_ref(), Expr::Operand(Operand::Field(source)) if !source.segments().is_empty())
            {
                out.push('*');
            } else {
                let wildcard_source = wildcard_to_sql(path, &field.expr)?;
                out.push_str(&wildcard_source);
                out.push_str(".*");
            }
        } else {
            out.push_str(&expr_to_sql(&field.expr)?);
        }
        if let Some(alias) = &field.alias {
            out.push_str(" AS ");
            out.push_str(alias);
        }
    }
    Ok(out)
}

fn wildcard_to_sql(path: &FieldPath, expr: &Expr) -> Result<String, SqlQueryError> {
    if path.segments().is_empty()
        && let Expr::Operand(Operand::Field(source_path)) = expr
    {
        if source_path.segments().is_empty() {
            return Ok(String::new());
        }
        return path_to_sql(source_path);
    }
    path_to_sql(path)
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
        Expr::Subquery(query) => Ok(format!(
            "({})",
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
        Expr::RelationExists {
            relation,
            source,
            target,
            transitive,
            max_depth,
        } => {
            let mut out = String::new();
            out.push_str(if *transitive {
                "has_relation_path("
            } else {
                "has_relation("
            });
            out.push_str(&expr_to_sql(relation)?);
            out.push_str(", ");
            out.push_str(&expr_to_sql(source)?);
            out.push_str(", ");
            out.push_str(&expr_to_sql(target)?);
            if let Some(max_depth) = max_depth {
                out.push_str(", ");
                out.push_str(&expr_to_sql(max_depth)?);
            }
            out.push(')');
            Ok(out)
        }
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
        BinaryOp::In => "IN",
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
    fn parse_and_print_qualified_wildcard_projection() {
        let parsed = parse_sql_query(
            "SELECT child.*, n.order AS directory_order FROM nodes AS n INNER JOIN entities AS child ON n.to = child.id",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(select.projection.len(), 2);
        assert_eq!(
            select.projection[0].wildcard,
            Some(FieldPath::from_fields(["child"]))
        );
        assert_eq!(
            select.projection[1].alias.as_deref(),
            Some("directory_order")
        );

        let sql = query_to_sql(&Query::Select(select)).unwrap();
        assert!(sql.starts_with("SELECT child.*, n.order AS directory_order FROM nodes AS n"));
    }

    #[test]
    fn prints_canonical_base_binding_wildcard() {
        let query = Query::Select(
            SelectQuery::new()
                .with_collection("entities")
                .with_source_alias("d")
                .with_projection(vec![QueryField {
                    expr: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields(["d"])))),
                    alias: None,
                    wildcard: Some(FieldPath::new()),
                }]),
        );

        let sql = query_to_sql(&query).unwrap();
        assert_eq!(sql, "SELECT d.* FROM entities AS d");
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
    fn resolves_nonaggregate_order_aliases_and_ordinals() {
        let parsed = parse_sql_query(
            "SELECT score + 1 AS rank, id FROM items ORDER BY rank DESC, 2 ASC, hidden DESC",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert!(matches!(
            select.order_by[0].expr,
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        ));
        assert!(matches!(
            &select.order_by[1].expr,
            Expr::Operand(Operand::Field(path)) if path == &FieldPath::from_fields(["id"])
        ));
        assert!(matches!(
            &select.order_by[2].expr,
            Expr::Operand(Operand::Field(path)) if path == &FieldPath::from_fields(["hidden"])
        ));

        for sql in [
            "SELECT id FROM items ORDER BY 0",
            "SELECT id FROM items ORDER BY 2",
            "SELECT * FROM items ORDER BY 1",
            "SELECT *, score FROM items ORDER BY 2",
        ] {
            assert!(parse_sql_query(sql, SqlDialectKind::Generic).is_err());
        }
    }

    #[test]
    fn resolves_aggregate_order_aliases_ordinals_and_expressions() {
        for order in ["total", "2", "SUM(score)"] {
            let sql = format!(
                "SELECT kind, SUM(score) AS total FROM items GROUP BY kind HAVING SUM(score) > 0 ORDER BY {order} DESC"
            );
            let parsed = parse_sql_query(&sql, SqlDialectKind::Generic).unwrap();
            let Query::Select(select) = parsed.query else {
                panic!("expected select");
            };
            assert!(matches!(
                &select.order_by[0].expr,
                Expr::Operand(Operand::Field(path))
                    if path == &FieldPath::from_fields(["total"])
            ));
        }

        let parsed = parse_sql_query(
            "SELECT kind AS category, COUNT(*) AS n FROM items GROUP BY kind ORDER BY kind",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert!(matches!(
            &select.order_by[0].expr,
            Expr::Operand(Operand::Field(path))
                if path == &FieldPath::from_fields(["category"])
        ));

        for sql in [
            "SELECT value, COUNT(*) AS n FROM items GROUP BY value ORDER BY value",
            "SELECT SUM(score) AS value FROM items ORDER BY value",
        ] {
            parse_sql_query(sql, SqlDialectKind::Generic).unwrap();
        }
    }

    #[test]
    fn aggregate_order_does_not_resolve_synthetic_output_names() {
        for sql in [
            "SELECT SUM(score) FROM items ORDER BY value",
            "SELECT score + 1 FROM items GROUP BY score + 1 ORDER BY value",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Invalid(message))
                    if message.contains("must reference a projected expression, alias, or ordinal")
            ));
        }
    }

    #[test]
    fn aggregate_order_round_trips_without_exposing_internal_output_keys() {
        for sql in [
            "SELECT SUM(score) FROM items ORDER BY SUM(score)",
            "SELECT SUM(score) FROM items ORDER BY 1",
            "SELECT kind + 1 FROM items GROUP BY kind + 1 ORDER BY kind + 1",
            "SELECT kind + 1 FROM items GROUP BY kind + 1 ORDER BY 1",
        ] {
            let parsed = parse_sql_query(sql, SqlDialectKind::Generic).unwrap();
            let printed = query_to_sql(&parsed.query).unwrap();
            assert!(!printed.contains("ORDER BY value"));
            parse_sql_query(&printed, SqlDialectKind::PostgreSql).unwrap();
        }
    }

    #[test]
    fn resolves_group_by_ordinals_and_rejects_ambiguous_output_aliases() {
        let parsed = parse_sql_query(
            "SELECT score AS kind, COUNT(*) AS n FROM items GROUP BY 1 ORDER BY 1",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(
            select.group_by,
            vec![Expr::Operand(Operand::Field(FieldPath::from_fields([
                "score",
            ])))]
        );

        parse_sql_query(
            "SELECT kind, COUNT(*) AS n FROM items GROUP BY 1 ORDER BY 1",
            SqlDialectKind::Generic,
        )
        .unwrap();
        for sql in [
            "SELECT kind AS kind, COUNT(*) AS n FROM items GROUP BY kind",
            "SELECT items.kind AS kind, COUNT(*) AS n FROM items GROUP BY kind",
            "SELECT score AS kind, COUNT(*) AS n FROM items GROUP BY score",
        ] {
            parse_sql_query(sql, SqlDialectKind::Generic).unwrap();
        }
        for sql in [
            "SELECT score AS kind, COUNT(*) AS n FROM items GROUP BY kind",
            "SELECT score + 1 AS bucket, COUNT(*) AS n FROM items GROUP BY bucket",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Invalid(message))
                    if message.contains("ambiguous with a projection alias")
                        && message.contains("ordinal or repeat")
            ));
        }
        for sql in [
            "SELECT kind, COUNT(*) AS n FROM items GROUP BY 0",
            "SELECT kind, COUNT(*) AS n FROM items GROUP BY 3",
            "SELECT kind, COUNT(*) AS n FROM items GROUP BY 2",
            "SELECT *, kind FROM items GROUP BY 1",
            "SELECT COUNT(*) AS kind FROM items GROUP BY kind",
        ] {
            assert!(parse_sql_query(sql, SqlDialectKind::Generic).is_err());
        }
    }

    #[test]
    fn normalizes_base_qualified_grouped_fields() {
        for sql in [
            "SELECT items.kind, COUNT(*) AS n FROM items GROUP BY kind",
            "SELECT kind, COUNT(*) AS n FROM items GROUP BY items.kind",
            "SELECT i.kind, COUNT(*) AS n FROM items AS i GROUP BY kind",
            "SELECT items.score + 1 AS bucket, COUNT(*) AS n FROM items GROUP BY score + 1",
        ] {
            parse_sql_query(sql, SqlDialectKind::Generic).unwrap();
        }
    }

    #[test]
    fn rejects_illegal_aggregate_placement_and_nesting() {
        for sql in [
            "SELECT id FROM items WHERE SUM(score) > 0",
            "SELECT COUNT(*) AS n FROM items AS i JOIN other AS o ON SUM(i.score) = o.score",
            "SELECT kind FROM items GROUP BY SUM(score)",
            "SELECT COUNT(*) AS n FROM items LIMIT COUNT(*)",
            "SELECT COUNT(SUM(score)) AS n FROM items",
            "SELECT COALESCE(COUNT(SUM(score)), 0) AS n FROM items",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Invalid(_))
            ));
        }
    }

    #[test]
    fn validates_grouped_fields_in_projection_having_and_order() {
        parse_sql_query(
            "SELECT kind, score, COUNT(*) AS n FROM items GROUP BY kind, score HAVING score > 0 ORDER BY score",
            SqlDialectKind::Generic,
        )
        .unwrap();
        for sql in [
            "SELECT kind, score, COUNT(*) AS n FROM items GROUP BY kind",
            "SELECT kind, COUNT(*) AS n FROM items GROUP BY kind HAVING score > 0",
            "SELECT kind, COUNT(*) AS n FROM items GROUP BY kind ORDER BY score",
            "SELECT kind, COUNT(*) AS n FROM items",
            "SELECT COUNT(*) AS n FROM items GROUP BY kind ORDER BY kind",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Invalid(_))
            ));
        }
    }

    #[test]
    fn rejects_aggregate_wildcards_and_duplicate_output_keys() {
        for sql in [
            "SELECT *, COUNT(*) AS n FROM items",
            "SELECT * FROM items GROUP BY kind",
            "SELECT id, id FROM items",
            "SELECT id, score AS id FROM items",
            "SELECT 1, 2 FROM items",
            "SELECT COUNT(*), SUM(score) FROM items",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Invalid(_))
            ));
        }
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
    fn source_less_select_requires_collection_scope() {
        assert!(matches!(
            parse_sql_query("SELECT 1", SqlDialectKind::Generic),
            Err(SqlQueryError::Invalid(message)) if message.contains("requires a FROM source")
        ));

        let query =
            parse_sql_query_for_collection("SELECT 1", "items", SqlDialectKind::Generic).unwrap();
        assert_eq!(query.collection(), Some("items"));
    }

    #[test]
    fn mixed_wildcard_projection_is_not_discarded() {
        let parsed =
            parse_sql_query("SELECT *, score AS s FROM items", SqlDialectKind::Generic).unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(select.projection.len(), 2);
        assert_eq!(select.projection[0].wildcard, Some(FieldPath::new()));
        assert_eq!(select.projection[1].alias.as_deref(), Some("s"));
        assert_eq!(
            query_to_sql(&Query::Select(select)).unwrap(),
            "SELECT *, score AS s FROM items"
        );
    }

    #[test]
    fn dml_returning_wildcard_is_explicit() {
        let parsed = parse_sql_query(
            "UPDATE items SET score = 1 RETURNING *",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Update(update) = &parsed.query else {
            panic!("expected update");
        };
        assert_eq!(update.returning.len(), 1);
        assert_eq!(update.returning[0].wildcard, Some(FieldPath::new()));
        assert!(query_to_sql(&parsed.query).unwrap().contains("RETURNING *"));
    }

    #[test]
    fn dml_limit_must_be_a_non_negative_constant_integer() {
        assert!(matches!(
            parse_sql_query(
                "UPDATE items SET score = 1 LIMIT score",
                SqlDialectKind::MySql,
            ),
            Err(SqlQueryError::Invalid(message)) if message.contains("DML LIMIT")
        ));
        assert!(matches!(
            parse_sql_query(
                "DELETE FROM items LIMIT -1",
                SqlDialectKind::MySql,
            ),
            Err(SqlQueryError::Invalid(message)) if message.contains("DML LIMIT")
        ));
        parse_sql_query(
            "UPDATE items SET score = 1 LIMIT 2 + 3",
            SqlDialectKind::MySql,
        )
        .unwrap();
    }

    #[test]
    fn dml_expressions_reject_subqueries() {
        assert!(matches!(
            parse_sql_query(
                "UPDATE items SET score = (SELECT score FROM other_items)",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("subqueries")
        ));
        assert!(matches!(
            parse_sql_query(
                "DELETE FROM items WHERE EXISTS (SELECT id FROM other_items)",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("subqueries")
        ));
    }

    #[test]
    fn dml_expressions_reject_aggregates() {
        for sql in [
            "UPDATE items SET score = SUM(score)",
            "DELETE FROM items WHERE COUNT(*) > 0",
            "UPDATE items SET score = 1 RETURNING SUM(score)",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Unsupported(message)) if message.contains("aggregate")
            ));
        }
    }

    #[test]
    fn rejects_ignored_select_and_table_fields() {
        assert!(matches!(
            parse_sql_query("SELECT SQL_NO_CACHE id FROM items", SqlDialectKind::MySql,),
            Err(SqlQueryError::Unsupported(_))
        ));
        assert!(matches!(
            parse_sql_query("SELECT * FROM items AS i(id)", SqlDialectKind::Generic,),
            Err(SqlQueryError::Unsupported(_))
        ));
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

    #[test]
    fn rejects_using_and_parses_cross_join() {
        assert!(matches!(
            parse_sql_query(
                "SELECT i.id FROM items AS i JOIN users AS u USING (id)",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("JOIN USING")
        ));

        let parsed = parse_sql_query(
            "SELECT i.id FROM items AS i CROSS JOIN tags AS t",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert!(matches!(
            &select.joins[0].condition,
            JoinCondition::OnExpr(Expr::Operand(Operand::Literal(Value::Bool(true))))
        ));
    }

    #[test]
    fn parses_multi_branch_and_simple_case() {
        let parsed = parse_sql_query(
            "SELECT CASE WHEN score > 10 THEN 'high' WHEN score > 0 THEN 'low' ELSE 'none' END AS band, CASE kind WHEN 'music' THEN 1 WHEN 'video' THEN 2 ELSE 0 END AS rank FROM items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert!(matches!(
            select.projection[0].expr.as_ref(),
            Expr::IfElse { else_expr, .. }
                if matches!(else_expr.as_ref(), Expr::IfElse { .. })
        ));
        assert!(matches!(
            select.projection[1].expr.as_ref(),
            Expr::IfElse { cond, else_expr, .. }
                if matches!(cond.as_ref(), Expr::Binary { op: BinaryOp::Eq, .. })
                    && matches!(else_expr.as_ref(), Expr::IfElse { .. })
        ));
    }

    #[test]
    fn only_executable_functions_and_valid_arities_are_accepted() {
        parse_sql_query(
            "SELECT LOWER(name) AS lowered, COALESCE(title, name) AS chosen FROM items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        parse_sql_query(
            "SELECT COUNT(*) AS count FROM items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        for sql in [
            "SELECT mystery(name) FROM items",
            "SELECT LOWER(name, title) FROM items",
            "SELECT COALESCE() FROM items",
            "SELECT COUNT() FROM items",
            "SELECT SUM(*) FROM items",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn rejects_similar_to_until_semantics_are_implemented() {
        assert!(matches!(
            parse_sql_query(
                "SELECT id FROM items WHERE name SIMILAR TO 'a%'",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("SIMILAR TO")
        ));
    }

    #[test]
    fn rejects_like_any_flags() {
        for sql in [
            "SELECT id FROM items WHERE name LIKE ANY ('a%', 'b%')",
            "SELECT id FROM items WHERE name ILIKE ANY ('a%', 'b%')",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::Generic),
                Err(SqlQueryError::Unsupported(message)) if message.contains("ANY")
            ));
        }
    }

    #[test]
    fn postgresql_dialect_remains_fail_closed_without_backend_capability() {
        for sql in [
            "SELECT date_trunc('day', created_at) FROM items",
            "SELECT id FROM items WHERE name SIMILAR TO 'a%'",
        ] {
            assert!(matches!(
                parse_sql_query(sql, SqlDialectKind::PostgreSql),
                Err(SqlQueryError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn parse_in_subquery_as_binary_in() {
        let parsed = parse_sql_query(
            "SELECT id FROM items WHERE id IN (SELECT o.id FROM other_items AS o)",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        let predicate = select.predicate.expect("predicate expected");
        assert!(matches!(
            predicate,
            Expr::Binary {
                op: BinaryOp::In,
                right,
                ..
            } if matches!(right.as_ref(), Expr::Subquery(_))
        ));
    }

    #[test]
    fn rejects_source_less_and_correlated_subqueries() {
        assert!(matches!(
            parse_sql_query(
                "SELECT id FROM items WHERE id = (SELECT 1)",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("source-less subqueries")
        ));
        assert!(matches!(
            parse_sql_query(
                "SELECT i.id FROM items AS i WHERE EXISTS (SELECT o.id FROM other_items AS o WHERE o.id = i.id)",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("correlated subqueries")
        ));
        assert!(matches!(
            parse_sql_query(
                "SELECT i.id FROM items AS i WHERE EXISTS (SELECT i.* FROM other_items AS o)",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("correlated subqueries")
        ));
        assert!(matches!(
            parse_sql_query(
                "SELECT i.id FROM items AS i WHERE EXISTS (SELECT o.id FROM other_items AS o WHERE outer_only = 1)",
                SqlDialectKind::Generic,
            ),
            Err(SqlQueryError::Unsupported(message)) if message.contains("unqualified field references")
        ));
        for dialect in [SqlDialectKind::Generic, SqlDialectKind::PostgreSql] {
            assert!(matches!(
                parse_sql_query(
                    "SELECT other_items.id FROM items AS other_items WHERE EXISTS (SELECT inner_items.id FROM public.other_items AS inner_items WHERE inner_items.id = other_items.id)",
                    dialect,
                ),
                Err(SqlQueryError::Unsupported(message)) if message.contains("correlated subqueries")
            ));
        }
    }

    #[test]
    fn accepts_multipart_self_qualified_subquery_fields() {
        for dialect in [SqlDialectKind::Generic, SqlDialectKind::PostgreSql] {
            parse_sql_query(
                "SELECT id FROM items WHERE id IN (SELECT public.other_items.id FROM public.other_items WHERE public.other_items.active = TRUE)",
                dialect,
            )
            .unwrap();
            parse_sql_query(
                "SELECT id FROM items WHERE id IN (SELECT inner_items.id FROM public.other_items AS inner_items WHERE inner_items.active = TRUE)",
                dialect,
            )
            .unwrap();
        }
    }

    #[test]
    fn ordinary_sql_literals_with_semicolons_reach_sqlparser() {
        let parsed =
            parse_sql_query("SELECT ';' AS marker FROM items", SqlDialectKind::Generic).unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(select.projection[0].alias.as_deref(), Some("marker"));
    }

    #[test]
    fn parse_create_attribute_statement() {
        let parsed = parse_sql_query(
            r#"CREATE ATTRIBUTE "age" TYPE u32"#,
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Ddl(ddl) = parsed.query else {
            panic!("expected ddl query");
        };
        assert_eq!(ddl.batch.operations.len(), 1);
        let crate::DdlOperation::UpsertAttribute { attribute } = &ddl.batch.operations[0] else {
            panic!("expected upsert attribute");
        };
        assert_eq!(attribute.id, "age");
        assert_eq!(attribute.name, "age");
        assert!(matches!(
            attribute.ty.kind,
            TypeKind::Number(NumberType::UInt(UIntWidth::U32))
        ));
    }
}
