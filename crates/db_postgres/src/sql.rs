use std::collections::BTreeMap;

use semantic_data::schema::{
    AnyType, BoolType, BytesEncoding, BytesType, FloatWidth, IntWidth, IpAddrType, NumberType,
    OptionalType, StringType, TemporalType, TimeUnit, TimeZoneSpec, TimestampType, Type, TypeKind,
};
use semantic_data::value::{Object, OrderedF64, Value};
use semantic_db_core::DbError;
use tokio_postgres::Row;

/// Quote a Postgres identifier, escaping embedded double quotes by doubling them.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('\"', "\"\""))
}

/// Map a Postgres data type to a semantic Type.
pub fn pg_type_to_semantic(data_type: &str, is_nullable: bool, numeric_scale: Option<i32>) -> Type {
    let base_kind = match data_type {
        "integer" | "int" | "int4" => TypeKind::Number(NumberType::Int(IntWidth::I32)),
        "bigint" | "int8" => TypeKind::Number(NumberType::Int(IntWidth::I64)),
        "smallint" | "int2" => TypeKind::Number(NumberType::Int(IntWidth::I16)),
        "serial" | "serial4" => TypeKind::Number(NumberType::Int(IntWidth::I32)),
        "bigserial" | "serial8" => TypeKind::Number(NumberType::Int(IntWidth::I64)),
        "real" | "float4" => TypeKind::Number(NumberType::Float(FloatWidth::F32)),
        "double precision" | "float8" => TypeKind::Number(NumberType::Float(FloatWidth::F64)),
        "numeric" | "decimal" if numeric_scale == Some(0) => {
            TypeKind::Number(NumberType::Int(IntWidth::I64))
        }
        "numeric" | "decimal" => TypeKind::Number(NumberType::Float(FloatWidth::F64)),
        "boolean" | "bool" => TypeKind::Bool(BoolType),
        "text" | "varchar" | "character varying" | "char" | "character" => {
            TypeKind::String(StringType {
                format: None,
                normalization: None,
            })
        }
        "uuid" => TypeKind::Uuid,
        "json" | "jsonb" => TypeKind::Json,
        "bytea" => TypeKind::Bytes(BytesType {
            encoding: Some(BytesEncoding::Base64),
        }),
        "date" => TypeKind::Temporal(TemporalType::Date),
        "time" | "time without time zone" => TypeKind::Temporal(TemporalType::Time),
        "timestamp" | "timestamp without time zone" => {
            TypeKind::Temporal(TemporalType::Timestamp(TimestampType {
                unit: TimeUnit::Micros,
                timezone: TimeZoneSpec::Forbidden,
            }))
        }
        "timestamptz" | "timestamp with time zone" => {
            TypeKind::Temporal(TemporalType::Timestamp(TimestampType {
                unit: TimeUnit::Micros,
                timezone: TimeZoneSpec::Specific("UTC".into()),
            }))
        }
        "inet" => TypeKind::IpAddr(IpAddrType::Any),
        _ => TypeKind::Any(AnyType),
    };

    if is_nullable {
        Type {
            kind: TypeKind::Optional(OptionalType {
                inner: Box::new(Type {
                    kind: base_kind,
                    constraints: vec![],
                    annotations: vec![],
                }),
            }),
            constraints: vec![],
            annotations: vec![],
        }
    } else {
        Type {
            kind: base_kind,
            constraints: vec![],
            annotations: vec![],
        }
    }
}

/// Convert a tokio_postgres Row into a semantic Object.
///
/// Each column is stored under the key `postgres:{table_name}:{column_name}`.
/// If a column is named `id`, its value is also stored under `postgres:id`.
pub fn row_to_object(row: &Row, table_name: &str) -> Result<Object, DbError> {
    let mut obj = BTreeMap::new();

    for (i, column) in row.columns().iter().enumerate() {
        let name = column.name();
        let type_name = column.type_().name();
        let value = column_to_value(row, i, type_name);
        let key = format!("postgres:{}:{}", table_name, name);
        obj.insert(key.clone(), value.clone());

        // Additionally store the raw id column value at "postgres:id".
        if name == "id" {
            obj.insert("postgres:id".to_string(), value);
        }
    }
    Ok(Object::from(obj))
}

/// Extract a single column from a Row and convert to semantic Value.
fn column_to_value(row: &Row, i: usize, type_name: &str) -> Value {
    match type_name {
        "text" | "varchar" | "char" | "character varying" | "character" => row
            .get::<_, Option<&str>>(i)
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null),
        "int4" => row
            .get::<_, Option<i32>>(i)
            .map(Value::I32)
            .unwrap_or(Value::Null),
        "int8" => row
            .get::<_, Option<i64>>(i)
            .map(Value::I64)
            .unwrap_or(Value::Null),
        "float8" => row
            .get::<_, Option<f64>>(i)
            .map(|v| Value::F64(OrderedF64::from(v)))
            .unwrap_or(Value::Null),
        "bool" | "boolean" => row
            .get::<_, Option<bool>>(i)
            .map(Value::Bool)
            .unwrap_or(Value::Null),
        "uuid" => row
            .get::<_, Option<uuid::Uuid>>(i)
            .map(|u| Value::Uuid(semantic_data::value::Uuid::from(u)))
            .unwrap_or(Value::Null),
        "jsonb" | "json" => row
            .get::<_, Option<serde_json::Value>>(i)
            .map(json_value_to_semantic)
            .unwrap_or(Value::Null),
        "bytea" => row
            .get::<_, Option<Vec<u8>>>(i)
            .map(|b| Value::Bytes(bytes::Bytes::from(b)))
            .unwrap_or(Value::Null),
        _ => {
            // Fallback: try text representation.
            row.get::<_, Option<&str>>(i)
                .map(|s| Value::String(s.to_string()))
                .unwrap_or(Value::Null)
        }
    }
}

/// Recursively convert a serde_json::Value to a semantic Value.
pub fn json_value_to_semantic(v: serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::I64(i)
            } else if let Some(f) = n.as_f64() {
                Value::F64(OrderedF64::from(f))
            } else {
                Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(arr) => {
            Value::List(arr.into_iter().map(json_value_to_semantic).collect())
        }
        serde_json::Value::Object(map) => {
            let mut btree = BTreeMap::new();
            for (k, val) in map {
                btree.insert(k, json_value_to_semantic(val));
            }
            Value::Object(Object::from(btree))
        }
    }
}

/// Parse a collection name like `postgres:schema:table` into `(schema, table)`.
pub fn parse_collection_name(name: &str) -> Result<(&str, &str), DbError> {
    let parts: Vec<&str> = name.splitn(3, ':').collect();
    if parts.len() < 3 || parts[0] != "postgres" {
        return Err(DbError::InvalidQuery(format!(
            "invalid postgres collection name: '{}'",
            name
        )));
    }
    Ok((parts[1], parts[2]))
}

/// Parse a synthetic entity ID back into PK column values.
///
/// Format for single PK: `{table_name}-{pk_value}`
/// Format for composite PK: `{table_name}-{pk1}::{pk2}::...`
///
/// The `::` separator is chosen for its extreme rarity in column values.
/// **Known limitation:** PK values containing `::` will fail to parse.
pub fn parse_entity_id(
    id: &str,
    table_name: &str,
    pk_column_count: usize,
) -> Result<Vec<String>, DbError> {
    let prefix = format!("{}-", table_name);
    let remainder = id
        .strip_prefix(&prefix)
        .ok_or_else(|| DbError::EntityNotFound {
            collection: format!("postgres:?:{}", table_name),
            id: id.to_string(),
        })?;

    if pk_column_count == 1 {
        return Ok(vec![remainder.to_string()]);
    }

    let parts: Vec<&str> = remainder.split("::").collect();
    if parts.len() != pk_column_count {
        return Err(DbError::InvalidQuery(format!(
            "expected {pk_column_count} PK values in id '{id}', got {}",
            parts.len()
        )));
    }
    Ok(parts.into_iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_ident() {
        assert_eq!(quote_ident("hello"), "\"hello\"");
        assert_eq!(quote_ident("foo\"bar"), "\"foo\"\"bar\"");
    }

    #[test]
    fn test_parse_collection_name() {
        let (schema, table) = parse_collection_name("postgres:public:users").unwrap();
        assert_eq!(schema, "public");
        assert_eq!(table, "users");

        assert!(parse_collection_name("invalid").is_err());
        assert!(parse_collection_name("postgres:only_one").is_err());
        assert!(parse_collection_name("mysql:public:users").is_err());
    }

    #[test]
    fn test_parse_entity_id_single_pk() {
        let result = parse_entity_id("users-42", "users", 1).unwrap();
        assert_eq!(result, vec!["42"]);
    }

    #[test]
    fn test_parse_entity_id_composite_pk() {
        let result = parse_entity_id("composite_test-1::2", "composite_test", 2).unwrap();
        assert_eq!(result, vec!["1", "2"]);
    }

    #[test]
    fn test_parse_entity_id_wrong_prefix() {
        let err = parse_entity_id("other-42", "users", 1).unwrap_err();
        assert!(matches!(err, DbError::EntityNotFound { .. }));
    }

    #[test]
    fn test_parse_entity_id_wrong_count() {
        let err = parse_entity_id("t-1::2::3", "t", 2).unwrap_err();
        assert!(matches!(err, DbError::InvalidQuery(_)));
    }

    #[test]
    fn test_pg_type_to_semantic() {
        let t = pg_type_to_semantic("int4", false, None);
        assert_eq!(t.kind, TypeKind::Number(NumberType::Int(IntWidth::I32)));

        let t = pg_type_to_semantic("text", true, None);
        assert_eq!(
            t.kind,
            TypeKind::Optional(OptionalType {
                inner: Box::new(Type {
                    kind: TypeKind::String(StringType {
                        format: None,
                        normalization: None,
                    }),
                    constraints: vec![],
                    annotations: vec![],
                })
            })
        );

        let t = pg_type_to_semantic("uuid", false, None);
        assert_eq!(t.kind, TypeKind::Uuid);
    }

    #[test]
    fn test_json_value_to_semantic() {
        let json = serde_json::json!({
            "name": "hello",
            "count": 42,
            "active": true,
            "tags": ["a", "b"],
            "nested": {"x": 1}
        });

        let val = json_value_to_semantic(json);
        match &val {
            Value::Object(obj) => {
                assert_eq!(obj.get("name").and_then(|v| v.as_str()), Some("hello"));
                assert_eq!(obj.get("count").and_then(|v| v.as_i64()), Some(42));
                assert_eq!(obj.get("active").and_then(|v| v.as_bool()), Some(true));
                match obj.get("tags") {
                    Some(Value::List(tags)) => assert_eq!(tags.len(), 2),
                    other => panic!("expected List, got {:?}", other),
                }
                assert!(matches!(obj.get("nested"), Some(Value::Object(_))));
            }
            _ => panic!("expected Object"),
        }
    }
}
