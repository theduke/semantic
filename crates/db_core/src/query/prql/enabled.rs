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
    let query = parse_prql_query_with_sql_fallback(prql, dialect)?;
    Ok(ParsedPrqlQuery { query })
}

pub fn parse_prql_query_for_collection(
    prql: &str,
    expected_collection: &str,
    dialect: SqlDialectKind,
) -> Result<Query, PrqlQueryError> {
    parse_prql_query_for_collection_with_sql_fallback(prql, expected_collection, dialect)
}

fn parse_prql_query_with_sql_fallback(
    prql: &str,
    dialect: SqlDialectKind,
) -> Result<Query, PrqlQueryError> {
    let maybe_rewritten = rewrite_implicit_collection_prql(prql, crate::DEFAULT_COLLECTION);
    let source = maybe_rewritten.as_deref().unwrap_or(prql);
    match compile_prql_to_sql(source) {
        Ok(compiled_sql) => {
            let parsed = sql::parse_sql_query(&compiled_sql, dialect).map_err(map_sql_error)?;
            Ok(with_query_collection_if_missing(
                parsed.query,
                crate::DEFAULT_COLLECTION.to_string(),
            ))
        }
        Err(prql_error) => {
            if let Ok(parsed) = sql::parse_sql_query(prql, dialect) {
                return Ok(parsed.query);
            }
            match sql::parse_sql_query_for_collection(prql, crate::DEFAULT_COLLECTION, dialect) {
                Ok(query) => Ok(query),
                Err(sql_error) => Err(PrqlQueryError::Invalid(format!(
                    "failed to parse query as PRQL ({prql_error}) or SQL ({sql_error})"
                ))),
            }
        }
    }
}

fn parse_prql_query_for_collection_with_sql_fallback(
    prql: &str,
    expected_collection: &str,
    dialect: SqlDialectKind,
) -> Result<Query, PrqlQueryError> {
    let maybe_rewritten = rewrite_implicit_collection_prql(prql, expected_collection);
    let source = maybe_rewritten.as_deref().unwrap_or(prql);
    match compile_prql_to_sql(source) {
        Ok(compiled_sql) => {
            let parsed = sql::parse_sql_query(&compiled_sql, dialect).map_err(map_sql_error)?;
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
        Err(prql_error) => {
            match sql::parse_sql_query_for_collection(prql, expected_collection, dialect) {
                Ok(query) => Ok(query),
                Err(sql_error) => Err(PrqlQueryError::Invalid(format!(
                    "failed to parse query as PRQL ({prql_error}) or SQL ({sql_error})"
                ))),
            }
        }
    }
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
        Query::Ddl(ddl) => Query::Ddl(ddl),
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

    #[test]
    fn parse_collection_scoped_sql_fallback_for_update() {
        let query = parse_prql_query_for_collection(
            "UPDATE SET score = 10 WHERE id = 'a'",
            "items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Update(update) = query else {
            panic!("expected update");
        };
        assert_eq!(update.collection.as_deref(), Some("items"));
        assert_eq!(update.assignments.len(), 1);
    }

    #[test]
    fn parse_collection_scoped_sql_fallback_for_delete() {
        let query = parse_prql_query_for_collection(
            "DELETE WHERE id = 'a'",
            "items",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Delete(delete) = query else {
            panic!("expected delete");
        };
        assert_eq!(delete.collection.as_deref(), Some("items"));
        assert!(delete.predicate.is_some());
    }

    #[test]
    fn parse_sql_fallback_insert() {
        let parsed = parse_prql_query(
            "INSERT INTO items (id, kind, score) VALUES ('a', 'music', 1) RETURNING id",
            SqlDialectKind::Generic,
        )
        .unwrap();
        let Query::Insert(insert) = parsed.query else {
            panic!("expected insert");
        };
        assert_eq!(insert.collection.as_deref(), Some("items"));
        assert_eq!(insert.columns, vec!["id", "kind", "score"]);
    }
}
