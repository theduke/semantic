use std::collections::BTreeMap;

use semantic_data::expr::{
    BinaryExpr, BinaryOperator, CallArg, CallExpr, Callee, Expr, FieldAccessExpr, LiteralExpr,
    RefExpr,
};
use semantic_data::schema::core::meta::Meta;
use semantic_data::schema::core::type_kind::TypeKind;
use semantic_data::schema::core::type_node::Type;
use semantic_data::schema::primitives::any_type::AnyType;
use semantic_data::schema::primitives::string_type::StringType;
use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassConstraint, ClassType, IndexKind,
    RelationIndexingMode, RelationMode, RelationType, TypeRef,
};
use semantic_data::value::Value;
use semantic_db_core::DbError;
use semantic_db_core::catalog::{Catalog, CollectionKind, IntegrityMode, PRIMARY_ID_FIELD};
use tokio_postgres::Client;

use crate::config::PostgresIdentityPolicy;
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
    is_generated: bool,
    generation_expression: Option<String>,
    is_identity: bool,
    identity_generation: Option<String>,
    domain_schema: Option<String>,
    domain_name: Option<String>,
    udt_schema: String,
    udt_name: String,
}

#[derive(Debug, Clone)]
struct DiscoveredIndex {
    name: String,
    columns: Vec<String>,
    unique: bool,
    access_method: String,
}

#[derive(Debug, Clone)]
struct DiscoveredForeignKey {
    name: String,
    source_columns: Vec<String>,
    target_schema: String,
    target_table: String,
    target_columns: Vec<String>,
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
    indexes: Vec<DiscoveredIndex>,
    foreign_keys: Vec<DiscoveredForeignKey>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan an existing Postgres database via `information_schema` and build an
/// in-memory `Catalog` representing the discovered tables.
pub async fn discover_catalog(client: &Client, schemas: &[String]) -> Result<Catalog, DbError> {
    discover_catalog_with_identity_policy(client, schemas, PostgresIdentityPolicy::PrimaryKey).await
}

pub async fn discover_catalog_with_identity_policy(
    client: &Client,
    schemas: &[String],
    identity_policy: PostgresIdentityPolicy,
) -> Result<Catalog, DbError> {
    let mut tables = discover_tables(client, schemas).await?;

    if identity_policy == PostgresIdentityPolicy::PrimaryKeyOrUniqueNotNull {
        for table in &mut tables {
            if !table.pk_columns.is_empty() {
                continue;
            }
            table.pk_columns = table
                .unique_constraints
                .iter()
                .find(|columns| {
                    columns.iter().all(|column| {
                        table
                            .columns
                            .iter()
                            .find(|candidate| candidate.name == *column)
                            .is_some_and(|candidate| !candidate.is_nullable)
                    })
                })
                .cloned()
                .unwrap_or_default();
        }
    }

    let missing_identity = tables
        .iter()
        .filter(|table| table.pk_columns.is_empty())
        .map(|table| format!("{}.{}", table.schema, table.name))
        .collect::<Vec<_>>();
    if !missing_identity.is_empty() {
        return Err(DbError::InvalidQuery(format!(
            "PostgreSQL discovery requires a primary key; tables without one: {}",
            missing_identity.join(", ")
        )));
    }

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
        register_table(&mut catalog, table, &tables)?;
    }
    for table in &tables {
        register_foreign_keys(&mut catalog, table, &tables)?;
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
    if schemas.is_empty() {
        return Ok(Vec::new());
    }

    let table_rows = client
        .query(
            "SELECT n.nspname, c.relname, c.relkind::text
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = ANY($1)
               AND c.relkind IN ('r', 'p')
               AND c.relname NOT LIKE '\\_semantic\\_%'
             ORDER BY n.nspname, c.relname",
            &[&schemas],
        )
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
        let indexes = discover_indexes(client, &schema, &name).await?;
        let foreign_keys = discover_foreign_keys(client, &schema, &name).await?;

        tables.push(DiscoveredTable {
            schema,
            name,
            table_type,
            columns,
            pk_columns,
            unique_constraints,
            indexes,
            foreign_keys,
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
                    character_maximum_length, numeric_precision, numeric_scale,
                    is_generated, generation_expression, is_identity,
                    identity_generation, domain_schema, domain_name, udt_schema, udt_name
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
            is_generated: row.get::<_, &str>(7) != "NEVER",
            generation_expression: row.get(8),
            is_identity: row.get::<_, &str>(9) == "YES",
            identity_generation: row.get(10),
            domain_schema: row.get(11),
            domain_name: row.get(12),
            udt_schema: row.get(13),
            udt_name: row.get(14),
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
            "SELECT a.attname
             FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class c ON c.oid = i.indrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
             JOIN pg_catalog.pg_attribute a
               ON a.attrelid = c.oid AND a.attnum = k.attnum
             WHERE n.nspname = $1 AND c.relname = $2 AND i.indisprimary
             ORDER BY k.ord",
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
            "SELECT idx.relname, a.attname
             FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class c ON c.oid = i.indrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
             JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
             JOIN pg_catalog.pg_attribute a
               ON a.attrelid = c.oid AND a.attnum = k.attnum
             WHERE n.nspname = $1 AND c.relname = $2
               AND i.indisunique AND NOT i.indisprimary
               AND i.indpred IS NULL AND i.indexprs IS NULL
             ORDER BY idx.relname, k.ord",
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

async fn discover_indexes(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<DiscoveredIndex>, DbError> {
    let rows = client
        .query(
            "SELECT idx.relname, i.indisunique, am.amname, a.attname
             FROM pg_catalog.pg_index i
             JOIN pg_catalog.pg_class c ON c.oid = i.indrelid
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_catalog.pg_class idx ON idx.oid = i.indexrelid
             JOIN pg_catalog.pg_am am ON am.oid = idx.relam
             JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) ON true
             JOIN pg_catalog.pg_attribute a
               ON a.attrelid = c.oid AND a.attnum = k.attnum
             WHERE n.nspname = $1 AND c.relname = $2
               AND NOT i.indisprimary AND i.indisvalid AND i.indisready
               AND i.indpred IS NULL AND i.indexprs IS NULL
             ORDER BY idx.relname, k.ord",
            &[&schema, &table],
        )
        .await
        .map_err(|error| DbError::Storage(error.to_string()))?;
    let mut grouped = BTreeMap::<String, DiscoveredIndex>::new();
    for row in rows {
        let name: String = row.get(0);
        let unique: bool = row.get(1);
        let access_method: String = row.get(2);
        let column: String = row.get(3);
        grouped
            .entry(name.clone())
            .or_insert_with(|| DiscoveredIndex {
                name,
                columns: Vec::new(),
                unique,
                access_method,
            })
            .columns
            .push(column);
    }
    Ok(grouped.into_values().collect())
}

async fn discover_foreign_keys(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Vec<DiscoveredForeignKey>, DbError> {
    let rows = client
        .query(
            "SELECT con.conname, src.attname, tn.nspname, tc.relname, dst.attname
             FROM pg_catalog.pg_constraint con
             JOIN pg_catalog.pg_class sc ON sc.oid = con.conrelid
             JOIN pg_catalog.pg_namespace sn ON sn.oid = sc.relnamespace
             JOIN pg_catalog.pg_class tc ON tc.oid = con.confrelid
             JOIN pg_catalog.pg_namespace tn ON tn.oid = tc.relnamespace
             JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY
                  AS keys(src_attnum, dst_attnum, ord) ON true
             JOIN pg_catalog.pg_attribute src
               ON src.attrelid = sc.oid AND src.attnum = keys.src_attnum
             JOIN pg_catalog.pg_attribute dst
               ON dst.attrelid = tc.oid AND dst.attnum = keys.dst_attnum
             WHERE con.contype = 'f' AND sn.nspname = $1 AND sc.relname = $2
             ORDER BY con.conname, keys.ord",
            &[&schema, &table],
        )
        .await
        .map_err(|error| DbError::Storage(error.to_string()))?;
    let mut grouped = BTreeMap::<String, DiscoveredForeignKey>::new();
    for row in rows {
        let name: String = row.get(0);
        let source_column: String = row.get(1);
        let target_schema: String = row.get(2);
        let target_table: String = row.get(3);
        let target_column: String = row.get(4);
        let foreign_key = grouped
            .entry(name.clone())
            .or_insert_with(|| DiscoveredForeignKey {
                name,
                source_columns: Vec::new(),
                target_schema,
                target_table,
                target_columns: Vec::new(),
            });
        foreign_key.source_columns.push(source_column);
        foreign_key.target_columns.push(target_column);
    }
    Ok(grouped.into_values().collect())
}

// ---------------------------------------------------------------------------
// Catalog registration
// ---------------------------------------------------------------------------

fn register_table(
    catalog: &mut Catalog,
    table: &DiscoveredTable,
    all_tables: &[DiscoveredTable],
) -> Result<(), DbError> {
    let class_id = format!("postgres:{}:{}", table.schema, table.name);
    let collection_name = class_id.clone();

    // Register each column as an attribute.
    let mut class_attrs = BTreeMap::new();

    for col in &table.columns {
        let attr_id = format!("postgres:{}:{}:{}", table.schema, table.name, col.name);
        let foreign_key = table.foreign_keys.iter().find(|foreign_key| {
            foreign_key.source_columns.as_slice() == [col.name.as_str()]
                && foreign_key.target_columns.len() == 1
                && foreign_key_targets_identity(foreign_key, all_tables)
        });
        let ty = if let Some(foreign_key) = foreign_key {
            Type {
                kind: TypeKind::Ref(TypeRef {
                    name: format!(
                        "postgres:{}:{}",
                        foreign_key.target_schema, foreign_key.target_table
                    ),
                    args: vec![],
                }),
                constraints: vec![],
                annotations: vec![],
            }
        } else {
            pg_type_to_semantic(&col.data_type, col.is_nullable, col.numeric_scale)
        };
        let mut meta = Meta::default();
        meta.annotations.insert(
            "postgres.udt".to_string(),
            format!("{}.{}", col.udt_schema, col.udt_name),
        );
        if let (Some(domain_schema), Some(domain_name)) = (&col.domain_schema, &col.domain_name) {
            meta.annotations.insert(
                "postgres.domain".to_string(),
                format!("{domain_schema}.{domain_name}"),
            );
        }
        if col.data_type == "USER-DEFINED" {
            meta.annotations.insert(
                "postgres.enum_or_user_defined".to_string(),
                "true".to_string(),
            );
        }
        if col.is_generated {
            meta.annotations
                .insert("postgres.generated".to_string(), "true".to_string());
            if let Some(expression) = &col.generation_expression {
                meta.annotations.insert(
                    "postgres.generation_expression".to_string(),
                    expression.clone(),
                );
            }
        }
        if col.is_identity {
            meta.annotations
                .insert("postgres.identity".to_string(), "true".to_string());
            if let Some(generation) = &col.identity_generation {
                meta.annotations.insert(
                    "postgres.identity_generation".to_string(),
                    generation.clone(),
                );
            }
        }
        if col.is_generated || col.is_identity {
            meta.annotations
                .insert("postgres.read_only".to_string(), "true".to_string());
        }

        catalog.register_attribute(AttributeType {
            id: attr_id.clone(),
            name: col.name.clone(),
            ty,
            constraints: vec![],
            meta,
        });

        class_attrs.insert(
            attr_id.clone(),
            ClassAttribute {
                attribute: AttributeRef { id: attr_id },
                required: !col.is_nullable,
                ui_order: None,
                computed: None,
                constraints: vec![],
                meta: Meta::default(),
            },
        );
    }

    // Add the synthetic computed "id" attribute.
    let synthetic_id = build_synthetic_id_attr(&table.schema, &table.name, &table.pk_columns);
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
        strict_schema: false,
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

        if let [pk_col] = table.pk_columns.as_slice() {
            let field = format!("postgres:{}:{}:{}", table.schema, table.name, pk_col);
            let index_name = format!("__pg_pk_{}", pk_col);
            catalog.upsert_index(index_name, collection_lid, field, true)?;
        }

        // Core indexes are single-field. Composite and expression indexes stay
        // available to native planning metadata but are not misrepresented.
        for index in table.indexes.iter().filter(|index| {
            index.columns.len() == 1 && matches!(index.access_method.as_str(), "btree" | "hash")
        }) {
            let field = format!(
                "postgres:{}:{}:{}",
                table.schema, table.name, index.columns[0]
            );
            catalog.upsert_index_with_kind(
                format!("__pg_index_{}", index.name),
                collection_lid,
                field,
                index.unique,
                IndexKind::Equality,
            )?;
        }
    }

    Ok(())
}

fn register_foreign_keys(
    catalog: &mut Catalog,
    table: &DiscoveredTable,
    all_tables: &[DiscoveredTable],
) -> Result<(), DbError> {
    let source_collection = format!("postgres:{}:{}", table.schema, table.name);
    for foreign_key in &table.foreign_keys {
        if foreign_key.source_columns.len() != 1 || foreign_key.target_columns.len() != 1 {
            continue;
        }
        if !foreign_key_targets_identity(foreign_key, all_tables) {
            continue;
        }
        let attribute = format!(
            "postgres:{}:{}:{}",
            table.schema, table.name, foreign_key.source_columns[0]
        );
        catalog.upsert_relationship(RelationType {
            id: format!(
                "postgres:fk:{}:{}:{}",
                table.schema, table.name, foreign_key.name
            ),
            name: foreign_key.name.clone(),
            source_collection: source_collection.clone(),
            mode: RelationMode::Embedded { attribute },
            indexing_mode: RelationIndexingMode::Enabled,
            meta: Meta::default(),
        })?;
    }
    Ok(())
}

fn foreign_key_targets_identity(
    foreign_key: &DiscoveredForeignKey,
    tables: &[DiscoveredTable],
) -> bool {
    tables.iter().any(|target| {
        target.schema == foreign_key.target_schema
            && target.name == foreign_key.target_table
            && target.pk_columns == foreign_key.target_columns
    })
}

// ---------------------------------------------------------------------------
// Synthetic ID construction
// ---------------------------------------------------------------------------

/// Build a computed `ClassAttribute` for the synthetic `id` field.
///
/// Single PK:  `stringify(table_name + "-" + self.postgres:table:pk_col)`
/// Composite:  `stringify(table_name + "-" + pk_a + "::" + pk_b + ...)`
fn build_synthetic_id_attr(
    schema_name: &str,
    table_name: &str,
    pk_columns: &[String],
) -> ClassAttribute {
    let prefix = Expr::Literal(LiteralExpr {
        value: Value::String(format!("{}-", table_name)),
    });

    let concat = pk_columns
        .iter()
        .enumerate()
        .fold(prefix, |acc, (i, pk_col)| {
            let pk_field = Expr::FieldAccess(Box::new(FieldAccessExpr {
                target: Expr::Ref(RefExpr::Identifier("self".into())),
                field: format!("postgres:{}:{}:{}", schema_name, table_name, pk_col),
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
                        value: Value::String("::".into()),
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
        ui_order: None,
        computed: Some(stringify_call),
        constraints: vec![],
        meta: Meta::default(),
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Create a string value from a JSON string.
fn literal_value_from_json(json: &str) -> Value {
    Value::String(json.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_synthetic_id_attr_single_pk() {
        let attr = build_synthetic_id_attr("public", "users", &["id".to_string()]);
        assert_eq!(attr.attribute.id, PRIMARY_ID_FIELD);
        assert!(attr.required);
        assert!(attr.computed.is_some());
    }

    #[test]
    fn test_build_synthetic_id_attr_composite_pk() {
        let attr = build_synthetic_id_attr("public", "orders", &["a".to_string(), "b".to_string()]);
        assert!(attr.computed.is_some());
    }

    #[test]
    fn test_build_synthetic_id_attr_empty_pk() {
        let attr = build_synthetic_id_attr("public", "orphan", &[]);
        assert!(attr.computed.is_some());
    }

    #[test]
    fn test_pg_pk_constraint_desc() {
        assert_eq!(PG_PK_CONSTRAINT_DESC, "__pg_pk");
    }

    #[test]
    fn registered_attributes_are_schema_qualified_and_composite_pk_is_not_unique() {
        let table = DiscoveredTable {
            schema: "accounting".into(),
            name: "entries".into(),
            table_type: "BASE TABLE".into(),
            columns: vec![
                DiscoveredColumn {
                    name: "tenant".into(),
                    data_type: "text".into(),
                    is_nullable: false,
                    column_default: None,
                    character_maximum_length: None,
                    numeric_precision: None,
                    numeric_scale: None,
                    is_generated: false,
                    generation_expression: None,
                    is_identity: false,
                    identity_generation: None,
                    domain_schema: None,
                    domain_name: None,
                    udt_schema: "pg_catalog".into(),
                    udt_name: "text".into(),
                },
                DiscoveredColumn {
                    name: "number".into(),
                    data_type: "int4".into(),
                    is_nullable: false,
                    column_default: None,
                    character_maximum_length: None,
                    numeric_precision: None,
                    numeric_scale: None,
                    is_generated: false,
                    generation_expression: None,
                    is_identity: false,
                    identity_generation: None,
                    domain_schema: None,
                    domain_name: None,
                    udt_schema: "pg_catalog".into(),
                    udt_name: "int4".into(),
                },
            ],
            pk_columns: vec!["tenant".into(), "number".into()],
            unique_constraints: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
        };
        let mut catalog = Catalog::new();
        catalog.register_attribute(AttributeType {
            id: PRIMARY_ID_FIELD.into(),
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
        register_table(&mut catalog, &table, &[]).unwrap();

        assert!(
            catalog
                .attribute_by_id("postgres:accounting:entries:tenant")
                .is_some()
        );
        let collection = catalog
            .collection_by_name("postgres:accounting:entries")
            .unwrap();
        assert!(catalog.indexes_for_collection(collection.lid).all(|index| {
            index.canonical_field != "postgres:accounting:entries:tenant"
                && index.canonical_field != "postgres:accounting:entries:number"
        }));
    }
}
