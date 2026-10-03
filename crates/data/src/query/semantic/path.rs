use super::*;

named_type!(
    value::FieldPath,
    "semantic:value:FieldPath",
    Vec::<value::PathSegment>::semantic_type()
);
impl IntoValue for value::FieldPath {
    fn into_value(self) -> Value {
        self.0.into_value()
    }
}
impl FromValue for value::FieldPath {
    fn from_value(value: Value) -> Result<Self, FromValueError> {
        Vec::<value::PathSegment>::from_value(value).map(Self)
    }
}

ast_variant!(value::PathSegment, "semantic:value:PathSegment", {
    Field => "field" (inner: String),
    Index => "index" (inner: usize),
});
