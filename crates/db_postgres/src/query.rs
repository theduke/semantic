use std::sync::Arc;

use futures::{StreamExt, stream};
use semantic_data::{
    expr::{Expr, LiteralExpr},
    schema::class::{class_constraint::ClassConstraint, class_type::ClassType},
    value::{Object, Value},
};
use semantic_db_core::{
    ALL_COLLECTION_ALIAS, AsyncPhysicalDataSource, CoreError, DbError, DynObject, EntityRecord,
    ExecutionOptions, Optimizer, PlanPair, QueryContext, SelectQuery, SendableRecordBatchStream,
    SourceRef, canonicalize_select_query, catalog::Catalog, execute_physical_plan_collect,
    format_output_rows,
};

use crate::sql::{
    encode_entity_id, parse_collection_name, parse_entity_id, quote_ident, row_to_object,
};

/// Compiles semantic queries to parameterized SQL and executes them against PostgreSQL.
pub struct QueryCompiler {
    pool: deadpool_postgres::Pool,
    allow_legacy_ids: bool,
}

impl QueryCompiler {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self {
            pool,
            allow_legacy_ids: true,
        }
    }

    pub fn new_with_legacy_ids(pool: deadpool_postgres::Pool, allow_legacy_ids: bool) -> Self {
        Self {
            pool,
            allow_legacy_ids,
        }
    }

    /// Execute a SELECT through the shared semantic optimizer and executor.
    pub async fn execute_select(
        &self,
        select: &SelectQuery,
        catalog: Arc<Catalog>,
    ) -> Result<Vec<Object>, DbError> {
        let collection_name = select.collection_or_default().to_string();
        let query = canonical_select(select, catalog.as_ref())?;
        let pair = Optimizer::core().optimize_query(
            &query,
            Some(collection_name),
            None,
            &QueryContext::new(catalog.clone()),
        );
        let rows = execute_physical_plan_collect(
            pair.physical,
            Arc::new(PostgresDiscoveryDataSource {
                pool: self.pool.clone(),
                catalog: catalog.clone(),
            }),
            QueryContext::new(catalog.clone()),
            ExecutionOptions::default(),
        )
        .await
        .map_err(|error| DbError::InvalidQuery(error.to_string()))?;
        let mut rows = format_output_rows(catalog.as_ref(), rows, query.field_format);
        inject_stable_ids(catalog.as_ref(), &mut rows)?;
        Ok(rows)
    }

    pub fn plan_select(
        &self,
        select: &SelectQuery,
        catalog: Arc<Catalog>,
    ) -> Result<PlanPair, DbError> {
        let collection_name = select.collection_or_default().to_string();
        let query = canonical_select(select, catalog.as_ref())?;
        Ok(Optimizer::core().optimize_query(
            &query,
            Some(collection_name),
            None,
            &QueryContext::new(catalog),
        ))
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
        if !self.allow_legacy_ids && !id.starts_with("pg1.") {
            return Err(DbError::InvalidQuery(
                "legacy PostgreSQL entity IDs are disabled".to_string(),
            ));
        }

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

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| DbError::Storage(format!("connection pool error: {}", e)))?;
        let pk_types = discover_pk_types(&client, schema, table_name, &pk_columns).await?;
        let projection = discovery_projection(&client, schema, table_name).await?;

        // Cast the text transport parameter to the native PK type, preserving
        // native comparison/index semantics without casting the indexed column.
        let params: Vec<String> = (1..=pk_count).map(|i| format!("${}", i)).collect();
        let conditions: Vec<String> = pk_columns
            .iter()
            .zip(params.iter())
            .map(|(column, param)| {
                let native_type = pk_types.get(column).expect("all PK types were discovered");
                format!("{} = {}::text::{}", quote_ident(column), param, native_type)
            })
            .collect();

        let sql = format!(
            "SELECT {projection} FROM {}.\"{}\" WHERE {}",
            quote_ident(schema),
            table_name.replace('"', "\"\""),
            conditions.join(" AND ")
        );

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

        let mut obj = row_to_object(&rows[0], collection_name)?;
        normalize_discovered_refs(catalog.as_ref(), &class.class, &mut obj)?;
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
}

async fn discover_pk_types(
    client: &tokio_postgres::Client,
    schema: &str,
    table: &str,
    pk_columns: &[String],
) -> Result<std::collections::BTreeMap<String, String>, DbError> {
    let rows = client
        .query(
            "SELECT a.attname,
                    pg_catalog.quote_ident(tn.nspname) || '.' || pg_catalog.quote_ident(t.typname)
             FROM pg_catalog.pg_attribute a
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_type t ON t.oid = a.atttypid
             JOIN pg_catalog.pg_namespace tn ON tn.oid = t.typnamespace
             WHERE n.nspname = $1 AND c.relname = $2 AND a.attname = ANY($3)",
            &[&schema, &table, &pk_columns],
        )
        .await
        .map_err(|error| DbError::Storage(format!("PK type discovery failed: {error}")))?;
    let types = rows
        .into_iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect::<std::collections::BTreeMap<_, _>>();
    if types.len() != pk_columns.len() {
        return Err(DbError::InvalidQuery(format!(
            "could not resolve every native primary-key type for '{schema}.{table}'"
        )));
    }
    Ok(types)
}

fn inject_stable_ids(catalog: &Catalog, rows: &mut [Object]) -> Result<(), DbError> {
    for row in rows {
        let Some(collection_name) = row
            .get(semantic_db_core::catalog::OBJECT_TYPE_FIELD)
            .and_then(Value::as_str)
            .map(ToString::to_string)
        else {
            continue;
        };
        let Some(class_lid) = catalog.class_ids(&collection_name).first().copied() else {
            continue;
        };
        let Some(class) = catalog.class_by_lid(class_lid) else {
            continue;
        };
        let pk_columns = extract_pk_columns(&class.class)?;
        let values = pk_columns
            .iter()
            .map(|column| {
                row.get(column)
                    .or_else(|| row.get(&format!("{collection_name}:{column}")))
                    .cloned()
            })
            .collect::<Option<Vec<_>>>();
        if let Some(values) = values {
            row.insert("id", Value::String(encode_entity_id(&values)?));
        }
    }
    Ok(())
}

fn canonical_select(select: &SelectQuery, catalog: &Catalog) -> Result<SelectQuery, DbError> {
    let collection_name = select.collection_or_default();
    if collection_name == ALL_COLLECTION_ALIAS {
        return Ok(select.clone());
    }
    let collection = catalog.collection_by_name(collection_name).ok_or_else(|| {
        DbError::UnknownCollectionByName {
            name: collection_name.to_string(),
        }
    })?;
    canonicalize_select_query(select, catalog, collection).map_err(Into::into)
}

struct PostgresDiscoveryDataSource {
    pool: deadpool_postgres::Pool,
    catalog: Arc<Catalog>,
}

impl AsyncPhysicalDataSource for PostgresDiscoveryDataSource {
    fn scan_stream(&self, source: SourceRef) -> SendableRecordBatchStream {
        let pool = self.pool.clone();
        let catalog = self.catalog.clone();
        stream::once(async move {
            load_source(pool, catalog, source)
                .await
                .map(|rows| {
                    rows.into_iter()
                        .map(|row| Box::new(row) as DynObject)
                        .collect()
                })
                .map_err(|error| CoreError::new(error.to_string()))
        })
        .boxed()
    }
}

async fn load_source(
    pool: deadpool_postgres::Pool,
    catalog: Arc<Catalog>,
    source: SourceRef,
) -> Result<Vec<Object>, DbError> {
    let source_name = source
        .source_name
        .or_else(|| {
            source
                .collection_id
                .and_then(|id| catalog.collection_by_lid(id))
                .map(|collection| collection.name.clone())
        })
        .ok_or_else(|| DbError::InvalidQuery("physical scan has no collection".to_string()))?;
    if source_name == ALL_COLLECTION_ALIAS {
        let names = catalog
            .collections()
            .map(|(_, collection)| collection)
            .filter(|collection| !collection.internal && collection.name.starts_with("postgres:"))
            .map(|collection| collection.name.clone())
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        for name in names {
            rows.extend(load_collection(&pool, catalog.as_ref(), &name).await?);
        }
        return Ok(rows);
    }
    load_collection(&pool, catalog.as_ref(), &source_name).await
}

async fn load_collection(
    pool: &deadpool_postgres::Pool,
    catalog: &Catalog,
    collection_name: &str,
) -> Result<Vec<Object>, DbError> {
    let (schema, table) = parse_collection_name(collection_name)?;
    let client = pool
        .get()
        .await
        .map_err(|error| DbError::Storage(format!("connection pool error: {error}")))?;
    let projection = discovery_projection(&client, schema, table).await?;
    let sql = format!(
        "SELECT {projection} FROM {}.{}",
        quote_ident(schema),
        quote_ident(table)
    );
    let rows = client
        .query(&sql, &[])
        .await
        .map_err(|error| DbError::Storage(format!("query error: {error}")))?;
    let class_lid = catalog
        .class_ids(collection_name)
        .first()
        .copied()
        .ok_or_else(|| DbError::InvalidQuery(format!("class not found for '{collection_name}'")))?;
    let class = catalog
        .class_by_lid(class_lid)
        .ok_or_else(|| DbError::InvalidQuery(format!("class not found for '{collection_name}'")))?;
    let pk_columns = extract_pk_columns(&class.class)?;
    rows.iter()
        .map(|row| {
            let mut object = row_to_object(row, collection_name)?;
            normalize_discovered_refs(catalog, &class.class, &mut object)?;
            object.insert(
                semantic_db_core::catalog::OBJECT_TYPE_FIELD,
                Value::String(collection_name.to_string()),
            );
            semantic_db_core::inject_computed_attributes(catalog, &mut object)
                .map_err(|error| DbError::InvalidQuery(error.to_string()))?;
            let values = pk_columns
                .iter()
                .map(|column| object.get(&format!("{collection_name}:{column}")).cloned())
                .collect::<Option<Vec<_>>>();
            if let Some(values) = values {
                object.insert("id", Value::String(encode_entity_id(&values)?));
            }
            Ok(object)
        })
        .collect()
}

fn normalize_discovered_refs(
    catalog: &Catalog,
    class: &ClassType,
    object: &mut Object,
) -> Result<(), DbError> {
    for class_attribute in class.attributes.values() {
        let Some(attribute) = catalog.attribute_by_id(&class_attribute.attribute.id) else {
            continue;
        };
        if !matches!(
            attribute.attribute.ty.kind,
            semantic_data::schema::TypeKind::Ref(_)
        ) {
            continue;
        }
        let Some(value) = object.get(&attribute.attribute.id).cloned() else {
            continue;
        };
        if !matches!(value, Value::Null | Value::Void) {
            object.insert(
                attribute.attribute.id.clone(),
                Value::String(encode_entity_id(&[value])?),
            );
        }
    }
    Ok(())
}

async fn discovery_projection(
    client: &tokio_postgres::Client,
    schema: &str,
    table: &str,
) -> Result<String, DbError> {
    let rows = client
        .query(
            "SELECT a.attname, t.typtype::text, bn.nspname, bt.typname
             FROM pg_catalog.pg_attribute a
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_type t ON t.oid = a.atttypid
             LEFT JOIN pg_catalog.pg_type bt ON bt.oid = t.typbasetype
             LEFT JOIN pg_catalog.pg_namespace bn ON bn.oid = bt.typnamespace
             WHERE n.nspname = $1 AND c.relname = $2
               AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum",
            &[&schema, &table],
        )
        .await
        .map_err(|error| {
            DbError::Storage(format!("column projection discovery failed: {error}"))
        })?;
    let columns = rows
        .into_iter()
        .map(|row| {
            let name: String = row.get(0);
            let type_kind: String = row.get(1);
            match type_kind.as_str() {
                "e" => format!("{}::text AS {}", quote_ident(&name), quote_ident(&name)),
                "d" => {
                    let base_schema: String = row.get(2);
                    let base_type: String = row.get(3);
                    format!(
                        "{}::{}.{} AS {}",
                        quote_ident(&name),
                        quote_ident(&base_schema),
                        quote_ident(&base_type),
                        quote_ident(&name)
                    )
                }
                _ => quote_ident(&name),
            }
        })
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return Err(DbError::InvalidQuery(format!(
            "PostgreSQL table '{schema}.{table}' has no readable columns"
        )));
    }
    Ok(columns.join(", "))
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
                value: Value::String(s),
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
