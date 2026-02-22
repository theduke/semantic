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

mod val;
pub use val::{OrderedF32, OrderedF64, Value};

mod valref;
pub use valref::ValueRef;

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
}
