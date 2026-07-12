mod duration;
pub use duration::Duration;

mod time;
pub use time::Time;

mod date;
pub use date::Date;

mod datetime;
pub use datetime::DateTime;

mod uuid;
pub use uuid::Uuid;

mod map;
pub use map::Map;

mod obj;
pub use obj::Object;

mod variant;
pub use variant::VariantValue;

mod val;
pub use val::{OrderedF32, OrderedF64, Value};

mod valref;
pub use valref::ValueRef;

pub mod canonical;
pub use canonical::{
    canonical_value_bytes, canonical_value_ref_bytes, write_canonical_value,
    write_canonical_value_ref,
};

mod path;
pub use path::{FieldPath, PathSegment};

mod access;
pub use access::{ObjectAccess, ObjectAccessMut};

pub mod serde;

pub mod convert;

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    fn value_samples() -> Vec<Value> {
        let mut map = Map::new();
        map.insert(Value::String("key".to_owned()), Value::I64(1));

        let mut object = Object::new();
        object.insert("key", Value::I64(1));

        vec![
            Value::Void,
            Value::Null,
            Value::Bool(true),
            Value::I8(1),
            Value::I16(1),
            Value::I32(1),
            Value::I64(1),
            Value::I128(1),
            Value::U8(1),
            Value::U16(1),
            Value::U32(1),
            Value::U64(1),
            Value::U128(1),
            Value::F32(OrderedF32::from(1.0)),
            Value::F64(OrderedF64::from(1.0)),
            Value::Uuid(Uuid::NIL),
            Value::IpAddr("127.0.0.1".parse().expect("valid IP address")),
            Value::Duration(Duration::from(::time::Duration::seconds(1))),
            Value::Time(Time::now_utc()),
            Value::Date(Date::now_utc()),
            Value::DateTime(DateTime::now_utc()),
            Value::Bytes(bytes::Bytes::from_static(b"bytes")),
            Value::String("string".to_owned()),
            Value::List(vec![Value::I64(1)]),
            Value::Map(map),
            Value::Object(object),
            Value::Variant(Box::new(VariantValue {
                r#type: Some("Example".to_owned()),
                variant: "value".to_owned(),
                value: Value::Null,
            })),
        ]
    }

    #[test]
    fn trait_contracts_hold_for_all_value_variants() {
        let values = value_samples();

        for value in &values {
            assert_eq!(value, value);
        }

        for left in &values {
            for right in &values {
                assert_eq!(left.cmp(right), right.cmp(left).reverse());
                assert_eq!(left == right, left.cmp(right) == Ordering::Equal);
            }
        }

        let mut sorted = values;
        sorted.sort();
        assert!(sorted.is_sorted());
    }

    #[test]
    fn map_supports_heterogeneous_value_keys() {
        let keys = [
            Value::Bool(true),
            Value::I64(42),
            Value::String("key".to_owned()),
            Value::Uuid(Uuid::NIL),
        ];
        let mut map = Map::new();

        for (index, key) in keys.iter().enumerate() {
            map.insert(key.clone(), Value::U64(index as u64));
        }
        for (index, key) in keys.iter().enumerate() {
            assert_eq!(map.get(key), Some(&Value::U64(index as u64)));
        }
    }

    #[test]
    fn void_and_ip_addr_are_reflexively_equal() {
        assert_eq!(Value::Void, Value::Void);
        let ip = Value::IpAddr("127.0.0.1".parse().expect("valid IP address"));
        assert_eq!(ip, ip);
    }

    #[test]
    fn test_facet_json() {
        let v = Value::String("hello".to_string());

        let x = facet_json::to_string(&v).unwrap();
        eprintln!("Serialized:\n{}", x);
    }

    #[test]
    fn test_object_access_path() {
        let mut nested = Object::new();
        nested.insert("answer", Value::I64(42));

        let mut root = Object::new();
        root.insert("nested", Value::Object(nested));

        let path = FieldPath::from(vec![
            PathSegment::Field("nested".to_string()),
            PathSegment::Field("answer".to_string()),
        ]);

        assert_eq!(root.value_at_path(&path), Some(&Value::I64(42)));
    }
}
