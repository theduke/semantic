use std::collections::BTreeMap;

use super::{FieldPath, Object, PathSegment, Value};

pub trait ObjectAccess {
    fn field_value(&self, field: &str) -> Option<&Value>;

    fn value_at_path<'a>(&'a self, path: &'a FieldPath) -> Option<&'a Value> {
        let mut current = path.segments().first().and_then(|first| match first {
            PathSegment::Field(field) => self.field_value(field),
            PathSegment::Index(_) => None,
        })?;

        for segment in path.segments().iter().skip(1) {
            current = match segment {
                PathSegment::Field(field) => current.get_field(field)?,
                PathSegment::Index(index) => current.get_index(*index)?,
            };
        }

        Some(current)
    }
}

pub trait ObjectAccessMut: ObjectAccess {
    fn field_value_mut(&mut self, field: &str) -> Option<&mut Value>;
}

impl ObjectAccess for Object {
    fn field_value(&self, field: &str) -> Option<&Value> {
        self.get(field)
    }
}

impl ObjectAccessMut for Object {
    fn field_value_mut(&mut self, field: &str) -> Option<&mut Value> {
        self.get_mut(field)
    }
}

impl ObjectAccess for BTreeMap<String, Value> {
    fn field_value(&self, field: &str) -> Option<&Value> {
        self.get(field)
    }
}

impl ObjectAccessMut for BTreeMap<String, Value> {
    fn field_value_mut(&mut self, field: &str) -> Option<&mut Value> {
        self.get_mut(field)
    }
}
