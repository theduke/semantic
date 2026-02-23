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

mod path;
pub use path::{FieldPath, PathSegment};

mod access;
pub use access::{ObjectAccess, ObjectAccessMut};

pub mod serde;

pub mod convert;

#[cfg(test)]
mod tests {
    use super::*;

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
