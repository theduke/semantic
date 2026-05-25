use std::collections::BTreeMap;

use semantic_data::expr::{
    BinaryExpr, BinaryOperator, CallArg, CallExpr, Callee, Expr, FieldAccessExpr, LiteralExpr,
    RefExpr,
};
use semantic_data::schema::core::literal_value::LiteralValue;
use semantic_data::schema::core::meta::Meta;
use semantic_data::schema::core::type_kind::TypeKind;
use semantic_data::schema::core::type_node::Type;
use semantic_data::schema::primitives::any_type::AnyType;
use semantic_data::schema::primitives::string_type::StringType;
use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassConstraint, ClassType,
};
use semantic_db_core::DbError;
use semantic_db_core::catalog::{Catalog, CollectionKind, IntegrityMode, PRIMARY_ID_FIELD};
use tokio_postgres::Client;

use crate::sql::pg_type_to_semantic;

/// Description tag for PK metadata constraints stored on classes.
pub const PG_PK_CONSTRAINT_DESC: &str = "__pg_pk";

// ---------------------------------------------------------------------------
// Intermediary types
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct DiscoveredColumn {
    name: String,
    data_type: String,
    is_nullable: bool,
    #[allow(dead_code)]
    column_default: Option<String>,
    #[allow(dead_code)]
    character_maximum_length: Option<i32>,
    #[allow(dead_code)]
    numeric_precision: Option<i32>,
    numeric_scale: Option<i32>,
}

#[derive(Debug)]
struct DiscoveredTable {
    schema: String,
    name: String,
    #[allow(dead_code)]
    table_type: String,
    columns: Vec<DiscoveredColumn>,
    pk_columns: Vec<String>,
    unique_constraints: Vec<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan an existing Postgres database via `information_schema` and build an
/// in-memory `Catalog` representing the discovered tables.
pub async fn discover_catalog(client: &Client, schemas: &[String]) -> Result<Catalog, DbError> {
    let tables = discover_tables(client, schemas).await?;

    let mut catalog = Catalog::new();

    // Register the canonical synthetic "id" attribute first, BEFORE
    // any other attributes or classes.
    catalog.register_attribute(AttributeType {
        id: PRIMARY_ID_FIELD.to_string(),
        name: "ID".into(),
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

    // Register the global "postgres:id" attribute (raw Postgres id column).
    catalog.register_attribute(AttributeType {
        id: "postgres:id".into(),
        name: "Raw Postgres ID".into(),
        ty: Type {
            kind: TypeKind::Any(AnyType),
            constraints: vec![],
            annotations: vec![],
        },
        constraints: vec![],
        meta: Meta::default(),
    });

    for table in &tables {
        register_table(&mut catalog, table)?;
    }

    Ok(catalog)
}

// ---------------------------------------------------------------------------
// Discovery helpers
// ---------------------------------------------------------------------------

async fn discover_tables(
    client: &Client,
    schemas: &[String],
) -> Result<Vec<DiscoveredTable>, DbError> {
    // Schemas come from configuration (not user input) so it's safe to inline.
    if schemas.is_empty() {
        return Ok(Vec::new());
    }

    let quoted_schemas: Vec<String> = schemas
        .iter()
        .map(|s| format!("'{}'", s.replace('\'', "''")))
        .collect();
    let in_clause = quoted_schemas.join(", ");

    let sql = format!(
        "SELECT table_schema, table_name, table_type
         FROM information_schema.tables
         WHERE table_schema IN ({in_clause})
           AND table_type = 'BASE TABLE'
           AND table_name NOT LIKE '\\_semantic\\_%'"
    );

    let table_rows = client
        .query(&sql, &[])
        .await
        .map_err(|e| DbError::Storage(e.to_string()))?;

    let mut tables = Vec::with_capacity(table_rows.len());

    for row in &table_rows {
        let schema: String = row.get(0);
        let name: String = row.get(1);
        let table_type: String = row.get(2);

        let columns = discover_columns(client, &schema, &name).await?;
        let pk_columns = discover_pk_columns(client, &schema, &name).await?;
        let unique_constraints = discover_unique_constraints(client, &schema, &name).await?;

        tables.push(DiscoveredTable {
            schema,
            name,
            table_type,
            columns,
            pk_columns,
            unique_constraints,
        });
    }

    Ok(tables)
}

async fn discover_columns(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<DiscoveredColumn>, DbError> {
    let rows = client
        .query(
            "SELECT column_name, data_type, is_nullable, column_default,
                    character_maximum_length, numeric_precision, numeric_scale
             FROM information_schema.columns
             WHERE table_schema = $1 AND table_name = $2
             ORDER BY ordinal_position",
            &[&schema, &table],
        )
        .await
        .map_err(|e| DbError::Storage(e.to_string()))?;

    let mut columns = Vec::with_capacity(rows.len());
    for row in &rows {
        columns.push(DiscoveredColumn {
            name: row.get(0),
            data_type: row.get(1),
            is_nullable: row.get::<_, &str>(2) == "YES",
            column_default: row.get(3),
            character_maximum_length: row.get(4),
            numeric_precision: row.get(5),
            numeric_scale: row.get(6),
        });
    }
    Ok(columns)
}

async fn discover_pk_columns(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<String>, DbError> {
    let rows = client
        .query(
            "SELECT kcu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_catalog = kcu.constraint_catalog
              AND tc.constraint_schema  = kcu.constraint_schema
              AND tc.constraint_name    = kcu.constraint_name
             WHERE tc.table_schema = $1
               AND tc.table_name   = $2
               AND tc.constraint_type = 'PRIMARY KEY'
             ORDER BY kcu.ordinal_position",
            &[&schema, &table],
        )
        .await
        .map_err(|e| DbError::Storage(e.to_string()))?;

    Ok(rows.iter().map(|r| r.get(0)).collect())
}

async fn discover_unique_constraints(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<Vec<String>>, DbError> {
    let rows = client
        .query(
            "SELECT kcu.constraint_name, kcu.column_name
             FROM information_schema.table_constraints tc
             JOIN information_schema.key_column_usage kcu
               ON tc.constraint_catalog = kcu.constraint_catalog
              AND tc.constraint_schema  = kcu.constraint_schema
              AND tc.constraint_name    = kcu.constraint_name
             WHERE tc.table_schema = $1
               AND tc.table_name   = $2
               AND tc.constraint_type = 'UNIQUE'
             ORDER BY kcu.constraint_name, kcu.ordinal_position",
            &[&schema, &table],
        )
        .await
        .map_err(|e| DbError::Storage(e.to_string()))?;

    // Group columns by constraint name.
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &rows {
        let constraint_name: String = row.get(0);
        let column_name: String = row.get(1);
        grouped
            .entry(constraint_name)
            .or_default()
            .push(column_name);
    }
    Ok(grouped.into_values().collect())
}

// ---------------------------------------------------------------------------
// Catalog registration
// ---------------------------------------------------------------------------

fn register_table(catalog: &mut Catalog, table: &DiscoveredTable) -> Result<(), DbError> {
    let class_id = format!("postgres:{}:{}", table.schema, table.name);
    let collection_name = class_id.clone();

    // Register each column as an attribute.
    let mut class_attrs = BTreeMap::new();

    for col in &table.columns {
        let attr_id = format!("postgres:{}:{}", table.name, col.name);
        let ty = pg_type_to_semantic(&col.data_type, col.is_nullable, col.numeric_scale);

        catalog.register_attribute(AttributeType {
            id: attr_id.clone(),
            name: col.name.clone(),
            ty,
            constraints: vec![],
            meta: Meta::default(),
        });

        class_attrs.insert(
            attr_id.clone(),
            ClassAttribute {
                attribute: AttributeRef { id: attr_id },
                required: !col.is_nullable,
                computed: None,
                constraints: vec![],
                meta: Meta::default(),
            },
        );
    }

    // Add the synthetic computed "id" attribute.
    let synthetic_id = build_synthetic_id_attr(&table.name, &table.pk_columns);
    class_attrs.insert(PRIMARY_ID_FIELD.to_string(), synthetic_id);

    // PK metadata constraint stored as JSON on the class.
    let pk_json = serde_json::to_string(&table.pk_columns)
        .map_err(|e| DbError::Storage(format!("serializing PK columns: {e}")))?;
    let pk_constraint = ClassConstraint::MultiFieldExpr {
        expr: Expr::Literal(LiteralExpr {
            value: literal_value_from_json(&pk_json),
        }),
        description: Some(PG_PK_CONSTRAINT_DESC.into()),
    };

    // Build and register the class.
    let class = ClassType {
        id: class_id,
        name: table.name.clone(),
        inherits: None,
        extends: vec![],
        attributes: class_attrs,
        constraints: vec![pk_constraint],
        meta: Meta::default(),
    };
    catalog.upsert_class(class)?;

    // Register the collection.
    catalog.upsert_collection(
        collection_name.clone(),
        CollectionKind::Schema,
        IntegrityMode::StrictRegisteredSchema,
    )?;

    // Register indexes for PK columns.
    if let Some(collection_schema) = catalog.collection_by_name(&collection_name) {
        let collection_lid = collection_schema.lid;

        for pk_col in &table.pk_columns {
            let field = format!("postgres:{}:{}", table.name, pk_col);
            let index_name = format!("__pg_pk_{}", pk_col);
            catalog.upsert_index(index_name, collection_lid, field, true)?;
        }

        // Register indexes for single-column unique constraints.
        for unique_cols in &table.unique_constraints {
            if unique_cols.len() == 1 {
                let field = format!("postgres:{}:{}", table.name, unique_cols[0]);
                let index_name = format!("__pg_uniq_{}", unique_cols[0]);
                catalog.upsert_index(index_name, collection_lid, field, true)?;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Synthetic ID construction
// ---------------------------------------------------------------------------

/// Build a computed `ClassAttribute` for the synthetic `id` field.
///
/// Single PK:  `stringify(table_name + "-" + self.postgres:table:pk_col)`
/// Composite:  `stringify(table_name + "-" + pk_a + "::" + pk_b + ...)`
fn build_synthetic_id_attr(table_name: &str, pk_columns: &[String]) -> ClassAttribute {
    let prefix = Expr::Literal(LiteralExpr {
        value: LiteralValue::String(format!("{}-", table_name)),
    });

    let concat = pk_columns
        .iter()
        .enumerate()
        .fold(prefix, |acc, (i, pk_col)| {
            let pk_field = Expr::FieldAccess(Box::new(FieldAccessExpr {
                target: Expr::Ref(RefExpr::Identifier("self".into())),
                field: format!("postgres:{}:{}", table_name, pk_col),
            }));

            if i == 0 {
                Expr::Binary(Box::new(BinaryExpr {
                    op: BinaryOperator::Concat,
                    left: acc,
                    right: pk_field,
                }))
            } else {
                let sep = Expr::Binary(Box::new(BinaryExpr {
                    op: BinaryOperator::Concat,
                    left: acc,
                    right: Expr::Literal(LiteralExpr {
                        value: LiteralValue::String("::".into()),
                    }),
                }));
                Expr::Binary(Box::new(BinaryExpr {
                    op: BinaryOperator::Concat,
                    left: sep,
                    right: pk_field,
                }))
            }
        });

    let stringify_call = Expr::Call(Box::new(CallExpr {
        callee: Callee::Name(vec!["stringify".into()]),
        args: vec![CallArg::Positional(concat)],
        over: None,
    }));

    ClassAttribute {
        attribute: AttributeRef {
            id: PRIMARY_ID_FIELD.to_string(),
        },
        required: true,
        computed: Some(stringify_call),
        constraints: vec![],
        meta: Meta::default(),
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Create a `LiteralValue::String` from a JSON string.
fn literal_value_from_json(json: &str) -> LiteralValue {
    LiteralValue::String(json.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_synthetic_id_attr_single_pk() {
        let attr = build_synthetic_id_attr("users", &["id".to_string()]);
        assert_eq!(attr.attribute.id, PRIMARY_ID_FIELD);
        assert!(attr.required);
        assert!(attr.computed.is_some());
    }

    #[test]
    fn test_build_synthetic_id_attr_composite_pk() {
        let attr = build_synthetic_id_attr("orders", &["a".to_string(), "b".to_string()]);
        assert!(attr.computed.is_some());
    }

    #[test]
    fn test_build_synthetic_id_attr_empty_pk() {
        let attr = build_synthetic_id_attr("orphan", &[]);
        assert!(attr.computed.is_some());
    }

    #[test]
    fn test_pg_pk_constraint_desc() {
        assert_eq!(PG_PK_CONSTRAINT_DESC, "__pg_pk");
    }
}
