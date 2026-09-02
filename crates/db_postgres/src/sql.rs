use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Datelike, Timelike};
use semantic_data::schema::{
    AnyType, BoolType, BytesEncoding, BytesType, FloatWidth, IntWidth, IpAddrType, NumberType,
    OptionalType, StringType, TemporalType, TimeUnit, TimeZoneSpec, TimestampType, Type, TypeKind,
};
use semantic_data::value::{Object, OrderedF64, Value};
use semantic_db_core::DbError;
use tokio_postgres::Row;

use crate::codec::{decode_value, encode_value};

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
/// Each column is stored under the schema-qualified collection key.
/// If a column is named `id`, its value is also stored under `postgres:id`.
pub fn row_to_object(row: &Row, collection_name: &str) -> Result<Object, DbError> {
    let mut obj = BTreeMap::new();

    for (i, column) in row.columns().iter().enumerate() {
        let name = column.name();
        let type_name = column.type_().name();
        let value = column_to_value(row, i, type_name)?;
        let key = format!("{}:{}", collection_name, name);
        obj.insert(key.clone(), value.clone());

        // Additionally store the raw id column value at "postgres:id".
        if name == "id" {
            obj.insert("postgres:id".to_string(), value);
        }
    }
    Ok(Object::from(obj))
}

/// Extract a single column from a Row and convert to semantic Value.
fn column_to_value(row: &Row, i: usize, type_name: &str) -> Result<Value, DbError> {
    let decode_error = |error| {
        DbError::Deserialization(format!(
            "could not decode PostgreSQL column '{}' of type '{}': {error}",
            row.columns()[i].name(),
            type_name
        ))
    };
    match type_name {
        "text" | "varchar" | "bpchar" | "char" | "character varying" | "character" => Ok(row
            .try_get::<_, Option<&str>>(i)
            .map_err(decode_error)?
            .map(|s| Value::String(s.to_string()))
            .unwrap_or(Value::Null)),
        "int2" => Ok(row
            .try_get::<_, Option<i16>>(i)
            .map_err(decode_error)?
            .map(Value::I16)
            .unwrap_or(Value::Null)),
        "int4" => Ok(row
            .try_get::<_, Option<i32>>(i)
            .map_err(decode_error)?
            .map(Value::I32)
            .unwrap_or(Value::Null)),
        "int8" => Ok(row
            .try_get::<_, Option<i64>>(i)
            .map_err(decode_error)?
            .map(Value::I64)
            .unwrap_or(Value::Null)),
        "float4" => Ok(row
            .try_get::<_, Option<f32>>(i)
            .map_err(decode_error)?
            .map(|v| Value::F32(v.into()))
            .unwrap_or(Value::Null)),
        "float8" => Ok(row
            .try_get::<_, Option<f64>>(i)
            .map_err(decode_error)?
            .map(|v| Value::F64(OrderedF64::from(v)))
            .unwrap_or(Value::Null)),
        "numeric" => Ok(row
            .try_get::<_, Option<PgNumeric>>(i)
            .map_err(decode_error)?
            .map(|value| Value::F64(OrderedF64::from(value.0)))
            .unwrap_or(Value::Null)),
        "bool" | "boolean" => Ok(row
            .try_get::<_, Option<bool>>(i)
            .map_err(decode_error)?
            .map(Value::Bool)
            .unwrap_or(Value::Null)),
        "uuid" => Ok(row
            .try_get::<_, Option<uuid::Uuid>>(i)
            .map_err(decode_error)?
            .map(|u| Value::Uuid(semantic_data::value::Uuid::from(u)))
            .unwrap_or(Value::Null)),
        "jsonb" | "json" => Ok(row
            .try_get::<_, Option<serde_json::Value>>(i)
            .map_err(decode_error)?
            .map(json_value_to_semantic)
            .unwrap_or(Value::Null)),
        "bytea" => Ok(row
            .try_get::<_, Option<Vec<u8>>>(i)
            .map_err(decode_error)?
            .map(|b| Value::Bytes(bytes::Bytes::from(b)))
            .unwrap_or(Value::Null)),
        "date" => Ok(row
            .try_get::<_, Option<chrono::NaiveDate>>(i)
            .map_err(decode_error)?
            .map(chrono_date_to_value)
            .transpose()?
            .unwrap_or(Value::Null)),
        "time" => Ok(row
            .try_get::<_, Option<chrono::NaiveTime>>(i)
            .map_err(decode_error)?
            .map(chrono_time_to_value)
            .transpose()?
            .unwrap_or(Value::Null)),
        "timestamp" => Ok(row
            .try_get::<_, Option<chrono::NaiveDateTime>>(i)
            .map_err(decode_error)?
            .map(|value| chrono_datetime_to_value(value.and_utc()))
            .transpose()?
            .unwrap_or(Value::Null)),
        "timestamptz" => Ok(row
            .try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(i)
            .map_err(decode_error)?
            .map(chrono_datetime_to_value)
            .transpose()?
            .unwrap_or(Value::Null)),
        "inet" | "cidr" => Ok(row
            .try_get::<_, Option<cidr::IpInet>>(i)
            .map_err(decode_error)?
            .map(|value| Value::IpAddr(value.address()))
            .unwrap_or(Value::Null)),
        "_int2" => decode_array(row, i, |value: i16| Value::I16(value)),
        "_int4" => decode_array(row, i, |value: i32| Value::I32(value)),
        "_int8" => decode_array(row, i, |value: i64| Value::I64(value)),
        "_float4" => decode_array(row, i, |value: f32| Value::F32(value.into())),
        "_float8" => decode_array(row, i, |value: f64| Value::F64(value.into())),
        "_bool" => decode_array(row, i, Value::Bool),
        "_text" | "_varchar" | "_bpchar" => decode_array(row, i, Value::String),
        "_uuid" => decode_array(row, i, |value: uuid::Uuid| Value::Uuid(value.into())),
        _ => Err(DbError::Deserialization(format!(
            "unsupported PostgreSQL result type '{}' for column '{}'",
            type_name,
            row.columns()[i].name()
        ))),
    }
}

fn decode_array<T>(row: &Row, index: usize, convert: impl Fn(T) -> Value) -> Result<Value, DbError>
where
    T: for<'a> tokio_postgres::types::FromSql<'a>,
{
    let values = row
        .try_get::<_, Option<Vec<Option<T>>>>(index)
        .map_err(|error| DbError::Deserialization(error.to_string()))?;
    Ok(values
        .map(|values| {
            Value::List(
                values
                    .into_iter()
                    .map(|value| value.map(&convert).unwrap_or(Value::Null))
                    .collect(),
            )
        })
        .unwrap_or(Value::Null))
}

fn chrono_date_to_value(value: chrono::NaiveDate) -> Result<Value, DbError> {
    let month = time::Month::try_from(value.month() as u8)
        .map_err(|error| DbError::Deserialization(error.to_string()))?;
    let date = time::Date::from_calendar_date(value.year(), month, value.day() as u8)
        .map_err(|error| DbError::Deserialization(error.to_string()))?;
    Ok(Value::Date(date.into()))
}

fn chrono_time_to_value(value: chrono::NaiveTime) -> Result<Value, DbError> {
    let time = time::Time::from_hms_nano(
        value.hour() as u8,
        value.minute() as u8,
        value.second() as u8,
        value.nanosecond(),
    )
    .map_err(|error| DbError::Deserialization(error.to_string()))?;
    Ok(Value::Time(time.into()))
}

fn chrono_datetime_to_value(value: chrono::DateTime<chrono::Utc>) -> Result<Value, DbError> {
    let nanos = value
        .timestamp_nanos_opt()
        .ok_or_else(|| DbError::Deserialization("timestamp is outside nanosecond range".into()))?;
    let datetime = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(nanos))
        .map_err(|error| DbError::Deserialization(error.to_string()))?;
    Ok(Value::DateTime(datetime.into()))
}

#[derive(Debug)]
struct PgNumeric(f64);

impl<'a> tokio_postgres::types::FromSql<'a> for PgNumeric {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        if raw.len() < 8 {
            return Err("numeric payload is shorter than its header".into());
        }
        let read_i16 = |offset: usize| i16::from_be_bytes([raw[offset], raw[offset + 1]]);
        let digits = usize::try_from(read_i16(0)).map_err(|_| "negative numeric digit count")?;
        let weight = i32::from(read_i16(2));
        let sign = u16::from_be_bytes([raw[4], raw[5]]);
        if sign == 0xC000 {
            return Ok(Self(f64::NAN));
        }
        if raw.len() != 8 + digits * 2 {
            return Err("numeric payload length does not match digit count".into());
        }
        let mut value = 0.0;
        for index in 0..digits {
            let offset = 8 + index * 2;
            let digit = u16::from_be_bytes([raw[offset], raw[offset + 1]]);
            if digit >= 10_000 {
                return Err("numeric base-10000 digit is invalid".into());
            }
            value += f64::from(digit) * 10_000_f64.powi(weight - index as i32);
        }
        match sign {
            0x0000 => Ok(Self(value)),
            0x4000 => Ok(Self(-value)),
            _ => Err("unsupported numeric sign code".into()),
        }
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::NUMERIC
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
    if let Some(payload) = id.strip_prefix("pg1.") {
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|err| DbError::InvalidQuery(format!("invalid pg1 entity id: {err}")))?;
        let tuple: Vec<serde_json::Value> = serde_json::from_slice(&bytes)
            .map_err(|err| DbError::InvalidQuery(format!("invalid pg1 entity id: {err}")))?;
        if tuple.len() != pk_column_count {
            return Err(DbError::InvalidQuery(format!(
                "expected {pk_column_count} PK values in id '{id}', got {}",
                tuple.len()
            )));
        }
        return tuple
            .iter()
            .map(decode_value)
            .map(|value| value.and_then(pk_value_to_text))
            .collect();
    }

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

/// Encode a typed primary-key tuple without delimiter ambiguity.
pub fn encode_entity_id(values: &[Value]) -> Result<String, DbError> {
    let tuple = values.iter().map(encode_value).collect::<Vec<_>>();
    let bytes =
        serde_json::to_vec(&tuple).map_err(|err| DbError::Serialization(err.to_string()))?;
    Ok(format!("pg1.{}", URL_SAFE_NO_PAD.encode(bytes)))
}

fn pk_value_to_text(value: Value) -> Result<String, DbError> {
    match value {
        Value::String(value) => Ok(value),
        Value::Bool(value) => Ok(value.to_string()),
        Value::I8(value) => Ok(value.to_string()),
        Value::I16(value) => Ok(value.to_string()),
        Value::I32(value) => Ok(value.to_string()),
        Value::I64(value) => Ok(value.to_string()),
        Value::I128(value) => Ok(value.to_string()),
        Value::U8(value) => Ok(value.to_string()),
        Value::U16(value) => Ok(value.to_string()),
        Value::U32(value) => Ok(value.to_string()),
        Value::U64(value) => Ok(value.to_string()),
        Value::U128(value) => Ok(value.to_string()),
        Value::F32(value) => Ok(value.into_inner().to_string()),
        Value::F64(value) => Ok(value.into_inner().to_string()),
        Value::Uuid(value) => {
            let value: uuid::Uuid = value.into();
            Ok(value.to_string())
        }
        Value::IpAddr(value) => Ok(value.to_string()),
        Value::Bytes(value) => Ok(format!("\\x{}", hex_bytes(&value))),
        other => Err(DbError::InvalidQuery(format!(
            "PostgreSQL primary-key value cannot be bound from semantic value {other:?}"
        ))),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
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
    fn versioned_entity_id_round_trips_delimiters_and_types() {
        let id = encode_entity_id(&[
            Value::String("a::b".into()),
            Value::I32(42),
            Value::Uuid(uuid::Uuid::nil().into()),
        ])
        .unwrap();
        assert!(id.starts_with("pg1."));
        assert_eq!(
            parse_entity_id(&id, "ignored", 3).unwrap(),
            vec!["a::b", "42", "00000000-0000-0000-0000-000000000000"]
        );
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
