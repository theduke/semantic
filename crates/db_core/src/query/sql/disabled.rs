use thiserror::Error;

use crate::Query;

const SQL_FEATURE_DISABLED: &str = "sql support is disabled (enable feature `sql`)";

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
    _sql: &str,
    _dialect: SqlDialectKind,
) -> Result<ParsedSqlQuery, SqlQueryError> {
    Err(SqlQueryError::Unsupported(SQL_FEATURE_DISABLED.to_string()))
}

pub fn parse_sql_query_for_collection(
    _sql: &str,
    _expected_collection: &str,
    _dialect: SqlDialectKind,
) -> Result<Query, SqlQueryError> {
    Err(SqlQueryError::Unsupported(SQL_FEATURE_DISABLED.to_string()))
}

pub fn query_to_sql(_query: &Query) -> Result<String, SqlQueryError> {
    Err(SqlQueryError::Unsupported(SQL_FEATURE_DISABLED.to_string()))
}
