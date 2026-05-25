use std::sync::Arc;

use semantic_data::{
    expr::{Expr, LiteralExpr},
    schema::{
        class::{class_constraint::ClassConstraint, class_type::ClassType},
        core::literal_value::LiteralValue,
    },
    value::{Object, Value},
};
use semantic_db_core::{DbError, EntityRecord, Query, SelectQuery, catalog::Catalog};

use crate::sql::{parse_collection_name, parse_entity_id, quote_ident, row_to_object};

/// Compiles semantic queries to parameterized SQL and executes them against PostgreSQL.
pub struct QueryCompiler {
    pool: deadpool_postgres::Pool,
}

impl QueryCompiler {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    /// Execute a SELECT query.
    ///
    /// 1. Generates SQL via `semantic_db_core::sql::query_to_sql()`.
    /// 2. Executes against Postgres.
    /// 3. Maps rows to semantic Objects.
    /// 4. Injects `type` and computed attributes.
    pub async fn execute_select(
        &self,
        select: &SelectQuery,
        catalog: Arc<Catalog>,
    ) -> Result<Vec<Object>, DbError> {
        let collection_name = select.collection_or_default().to_string();
        let (_schema, table_name) = parse_collection_name(&collection_name)?;

        let sql = self.compile_select(select)?;
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| DbError::Storage(format!("connection pool error: {}", e)))?;

        let rows = client
            .query(&sql, &[])
            .await
            .map_err(|e| DbError::Storage(format!("query error on sql '{}': {}", sql, e)))?;

        let mut objects: Vec<Object> = rows
            .iter()
            .map(|row| row_to_object(row, table_name))
            .collect::<Result<Vec<_>, _>>()?;

        // Inject type field and computed attributes for each object.
        for obj in &mut objects {
            obj.insert(
                semantic_db_core::catalog::OBJECT_TYPE_FIELD.to_string(),
                Value::String(collection_name.clone()),
            );
            semantic_db_core::inject_computed_attributes(&catalog, obj)
                .map_err(|e| DbError::InvalidQuery(e.to_string()))?;
        }

        Ok(objects)
    }

    /// Execute a `get()` by synthetic ID.
    ///
    /// Parses the ID to extract PK values, builds a parameterized SQL query,
    /// executes it, maps the row, and injects computed attributes.
    pub async fn execute_get(
        &self,
        collection_name: &str,
        id: &str,
        catalog: Arc<Catalog>,
    ) -> Result<Option<EntityRecord>, DbError> {
        let (schema, table_name) = parse_collection_name(collection_name)?;

        // Find the class to extract PK constraint metadata.
        let class_lid = catalog
            .class_ids(collection_name)
            .first()
            .copied()
            .ok_or_else(|| {
                DbError::InvalidQuery(format!(
                    "class not found for collection '{}'",
                    collection_name
                ))
            })?;
        let class = catalog.class_by_lid(class_lid).ok_or_else(|| {
            DbError::InvalidQuery(format!(
                "class not found for lid in collection '{}'",
                collection_name
            ))
        })?;

        let pk_columns = extract_pk_columns(&class.class)?;
        let pk_count = pk_columns.len();

        // Parse synthetic ID into PK values.
        let pk_values = parse_entity_id(id, table_name, pk_count)?;

        // Build parameterized SQL: SELECT * FROM schema."table" WHERE CAST("pk1" AS TEXT) = $1 AND ...
        // Casting to TEXT allows passing string PK values regardless of the actual column type.
        let params: Vec<String> = (1..=pk_count).map(|i| format!("${}", i)).collect();
        let conditions: Vec<String> = pk_columns
            .iter()
            .zip(params.iter())
            .map(|(col, param)| {
                let col_quoted = col.replace('"', "\"\"");
                format!("CAST(\"{}\" AS TEXT) = {}", col_quoted, param)
            })
            .collect();

        let sql = format!(
            "SELECT * FROM {}.\"{}\" WHERE {}",
            quote_ident(schema),
            table_name.replace('"', "\"\""),
            conditions.join(" AND ")
        );

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| DbError::Storage(format!("connection pool error: {}", e)))?;

        // Prepare statement for parameterized execution.
        let stmt = client
            .prepare(&sql)
            .await
            .map_err(|e| DbError::Storage(format!("prepare error: {}", e)))?;

        // Convert PK values to trait references for tokio-postgres.
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pk_values
            .iter()
            .map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&stmt, &param_refs)
            .await
            .map_err(|e| DbError::Storage(format!("query error: {}", e)))?;

        if rows.is_empty() {
            return Ok(None);
        }

        let mut obj = row_to_object(&rows[0], table_name)?;
        obj.insert(
            semantic_db_core::catalog::OBJECT_TYPE_FIELD.to_string(),
            Value::String(collection_name.to_string()),
        );

        semantic_db_core::inject_computed_attributes(&catalog, &mut obj)
            .map_err(|e| DbError::InvalidQuery(e.to_string()))?;

        Ok(Some(EntityRecord {
            id: id.to_string(),
            collection: collection_name.to_string(),
            object: obj,
        }))
    }

    /// Compile a SELECT query to SQL using the generic query-to-sql from db_core,
    /// then rewrite collection references to actual Postgres `schema."table"` syntax.
    fn compile_select(&self, select: &SelectQuery) -> Result<String, DbError> {
        let query = Query::Select(select.clone());
        let sql = semantic_db_core::sql::query_to_sql(&query)
            .map_err(|e| DbError::InvalidQuery(format!("failed to compile query to sql: {}", e)))?;

        // The generic SQL dialect emits the collection name as a bare identifier.
        // We rewrite `postgres:schema:table` -> `schema."table"`.
        rewrite_collection_refs(&sql)
    }
}

/// Rewrite collection-name table references like `postgres:public:users`
/// to actual Postgres table references `public."users"`.
fn rewrite_collection_refs(sql: &str) -> Result<String, DbError> {
    // Simple scanner: find every `postgres:` token and parse schema:table.
    let needle = "postgres:";
    let mut result = String::with_capacity(sql.len());
    let mut pos = 0;

    while let Some(start) = sql[pos..].find(needle) {
        // Copy everything before the match.
        result.push_str(&sql[pos..pos + start]);
        let after = pos + start + needle.len();

        // Read schema name up to next ':'
        let rest = &sql[after..];
        let sep = rest.find(':').ok_or_else(|| {
            DbError::InvalidQuery(format!("malformed collection reference in SQL: {}", sql))
        })?;
        let schema = &rest[..sep];
        let after_schema = after + sep + 1;

        // Read table name up to next non-identifier character
        let rest2 = &sql[after_schema..];
        let table_len = rest2
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '"')
            .unwrap_or(rest2.len());
        let table = &rest2[..table_len];

        // Emit: schema."table"
        result.push_str(schema);
        result.push_str(".\"");
        result.push_str(table);
        result.push('"');

        pos = after_schema + table_len;
    }

    result.push_str(&sql[pos..]);
    Ok(result)
}

/// Extract PK column names from a class's constraints.
///
/// Looks for a `ClassConstraint::MultiFieldExpr` with `description == "__pg_pk"`
/// whose expression contains a JSON array of column names.
pub fn extract_pk_columns(class: &ClassType) -> Result<Vec<String>, DbError> {
    let constraint = class
        .constraints
        .iter()
        .find(|c| {
            matches!(
                c,
                ClassConstraint::MultiFieldExpr {
                    description: Some(d),
                    ..
                } if d == "__pg_pk"
            )
        })
        .ok_or_else(|| {
            DbError::InvalidQuery(format!(
                "class '{}' has no PK constraint (__pg_pk)",
                class.id
            ))
        })?;

    let json_str = match constraint {
        ClassConstraint::MultiFieldExpr { expr, .. } => match expr {
            Expr::Literal(LiteralExpr {
                value: LiteralValue::String(s),
            }) => s.clone(),
            _ => {
                return Err(DbError::InvalidQuery(
                    "PK constraint expression is not a string literal".into(),
                ));
            }
        },
        _ => unreachable!(),
    };

    serde_json::from_str(&json_str)
        .map_err(|e| DbError::InvalidQuery(format!("invalid PK constraint JSON: {}", e)))
}
