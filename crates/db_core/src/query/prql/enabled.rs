use thiserror::Error;

use crate::{Query, SqlDialectKind, sql};

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
    prql: &str,
    dialect: SqlDialectKind,
) -> Result<ParsedPrqlQuery, PrqlQueryError> {
    let maybe_rewritten = rewrite_implicit_collection_prql(prql, crate::DEFAULT_COLLECTION);
    let source = maybe_rewritten.as_deref().unwrap_or(prql);
    let sql = compile_prql_to_sql(source)?;
    let parsed = sql::parse_sql_query(&sql, dialect).map_err(map_sql_error)?;
    let query =
        with_query_collection_if_missing(parsed.query, crate::DEFAULT_COLLECTION.to_string());
    Ok(ParsedPrqlQuery { query })
}

pub fn parse_prql_query_for_collection(
    prql: &str,
    expected_collection: &str,
    dialect: SqlDialectKind,
) -> Result<Query, PrqlQueryError> {
    let maybe_rewritten = rewrite_implicit_collection_prql(prql, expected_collection);
    let source = maybe_rewritten.as_deref().unwrap_or(prql);
    let sql = compile_prql_to_sql(source)?;
    let parsed = sql::parse_sql_query(&sql, dialect).map_err(map_sql_error)?;
    let collection = parsed.query.collection().map(ToOwned::to_owned);
    if let Some(collection) = collection {
        if collection != expected_collection {
            return Err(PrqlQueryError::Invalid(format!(
                "collection mismatch: PRQL query targets '{}', expected '{}'",
                collection, expected_collection
            )));
        }
    }
    Ok(with_query_collection_if_missing(
        parsed.query,
        expected_collection.to_string(),
    ))
}

fn compile_prql_to_sql(prql: &str) -> Result<String, PrqlQueryError> {
    let options = prqlc::Options::default().with_target(prqlc::Target::Sql(None));
    prqlc::compile(prql, &options).map_err(|err| PrqlQueryError::Compile(err.to_string()))
}

fn map_sql_error(error: sql::SqlQueryError) -> PrqlQueryError {
    match error {
        sql::SqlQueryError::Parse(message) => {
            PrqlQueryError::Invalid(format!("compiled SQL parse failed: {message}"))
        }
        sql::SqlQueryError::Unsupported(message) => PrqlQueryError::Unsupported(message),
        sql::SqlQueryError::Invalid(message) => PrqlQueryError::Invalid(message),
    }
}

fn rewrite_implicit_collection_prql(prql: &str, collection: &str) -> Option<String> {
    let trimmed = prql.trim();
    if trimmed.is_empty() || starts_with_keyword(trimmed, "from") {
        return None;
    }
    let pipeline = trimmed.trim_start_matches('|').trim_start();
    Some(format!("from {collection}\n{pipeline}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prql_with_from() {
        let parsed = parse_prql_query(
            "from items | filter kind == \"music\" | select {id, score}",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = parsed.query else {
            panic!("expected select");
        };
        assert_eq!(select.collection.as_deref(), Some("items"));
    }

    #[test]
    fn parse_collection_scoped_prql_without_from() {
        let query = parse_prql_query_for_collection(
            "filter kind == \"music\" | select {id}",
            "items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Select(select) = query else {
            panic!("expected select");
        };
        assert_eq!(select.collection.as_deref(), Some("items"));
    }
}
