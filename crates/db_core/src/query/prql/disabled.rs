use thiserror::Error;

use crate::{Query, SqlDialectKind};

const PRQL_FEATURE_DISABLED: &str = "prql support is disabled (enable feature `prql`)";

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedPrqlQuery {
    pub query: Query,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PrqlQueryError {
    #[error("prql compile error: {0}")]
    Compile(String),
    #[error("unsupported prql operation: {0}")]
    Unsupported(String),
    #[error("invalid prql query: {0}")]
    Invalid(String),
}

pub fn parse_prql_query(
    _prql: &str,
    _dialect: SqlDialectKind,
) -> Result<ParsedPrqlQuery, PrqlQueryError> {
    Err(PrqlQueryError::Unsupported(
        PRQL_FEATURE_DISABLED.to_string(),
    ))
}

pub fn parse_prql_query_for_collection(
    _prql: &str,
    _expected_collection: &str,
    _dialect: SqlDialectKind,
) -> Result<Query, PrqlQueryError> {
    Err(PrqlQueryError::Unsupported(
        PRQL_FEATURE_DISABLED.to_string(),
    ))
}
