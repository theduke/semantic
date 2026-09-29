use std::collections::BTreeMap;

use semantic_data::schema::{
    EnumRepr, NumberType, OptionalType, RecordType, Type, TypeKind, UIntWidth, VariantPayload,
    VariantTag,
};
use semantic_data::value::{FromValue, IntoValue, Object, SemanticType, Value};

#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq, Default)]
#[semantic(rename_all = "snake_case")]
enum Mode {
    /// Open only existing databases.
    OpenExisting,
    #[default]
    AutoCreate,
    #[semantic(rename = "other")]
    Renamed,
}

#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
struct Id(String);

#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
struct Payload {
    /// The target id.
    id: Id,
    scope_id: Option<String>,
    #[semantic(required)]
    note: Option<String>,
    #[semantic(default)]
    mode: Mode,
    #[semantic(default = "default_limit")]
    limit: u32,
    #[semantic(rename = "type")]
    kind: String,
    tags: Vec<String>,
    counts: BTreeMap<String, u64>,
}

fn default_limit() -> u32 {
    50
}

#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
struct Outer {
    #[semantic(flatten)]
    payload: Payload,
    extra: bool,
}

#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
#[semantic(tag = "kind", rename_all = "snake_case")]
enum Operation {
    Create {
        id: String,
        object: Object,
    },
    DeleteByIds {
        collection: Option<String>,
        ids: Vec<String>,
    },
    Ddl,
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn string(value: &str) -> Value {
    Value::String(value.to_owned())
}

fn record(ty: Type) -> RecordType {
    match ty.kind {
        TypeKind::Record(record) => record,
        other => panic!("expected record, got {other:?}"),
    }
}

fn payload() -> Payload {
    Payload {
        id: Id("a".into()),
        scope_id: None,
        note: None,
        mode: Mode::OpenExisting,
        limit: 3,
        kind: "k".into(),
        tags: vec!["x".into()],
        counts: BTreeMap::from([("c".to_owned(), 1)]),
    }
}

#[test]
fn struct_round_trips_with_wire_shape() {
    let value = payload().into_value();
    assert_eq!(
        value,
        object([
            ("id", string("a")),
            ("note", Value::Null),
            ("mode", string("open_existing")),
            ("limit", Value::U32(3)),
            ("type", string("k")),
            ("tags", Value::List(vec![string("x")])),
            ("counts", object([("c", Value::U64(1))])),
        ])
    );
    assert_eq!(Payload::from_value(value).unwrap(), payload());
}

#[test]
fn struct_decodes_defaults_nulls_and_ignores_unknown_fields() {
    let decoded = Payload::from_value(object([
        ("id", string("a")),
        ("scope_id", Value::Null),
        ("note", Value::Void),
        ("type", string("k")),
        ("tags", Value::List(vec![])),
        ("counts", object([])),
        ("unknown", Value::Bool(true)),
    ]))
    .unwrap();
    assert_eq!(decoded.scope_id, None);
    assert_eq!(decoded.note, None);
    assert_eq!(decoded.mode, Mode::AutoCreate);
    assert_eq!(decoded.limit, 50);
}

#[test]
fn errors_carry_the_field_path() {
    let err = Payload::from_value(object([("id", Value::I64(1))])).unwrap_err();
    assert_eq!(err.to_string(), "id: expected string, found integer");
    assert_eq!(
        err.describe("payload"),
        "payload.id: expected string, found integer"
    );

    let err = Payload::from_value(object([("id", string("a"))])).unwrap_err();
    assert_eq!(err.to_string(), "note: missing required field");

    let mut fields = payload().into_value();
    let Value::Object(map) = &mut fields else {
        unreachable!()
    };
    map.insert("tags", Value::List(vec![string("x"), Value::Bool(false)]));
    let err = Payload::from_value(fields).unwrap_err();
    assert_eq!(err.to_string(), "tags[1]: expected string, found boolean");

    let err = Mode::from_value(string("nope")).unwrap_err();
    assert_eq!(
        err.to_string(),
        "unknown variant 'nope', expected one of: open_existing, auto_create, other"
    );
    assert_eq!(
        Payload::from_value(Value::Null)
            .unwrap_err()
            .describe("payload"),
        "payload: expected object, found null"
    );
}

#[test]
fn record_type_matches_fields() {
    let record = record(Payload::semantic_type());
    assert!(!record.open);
    assert_eq!(
        record.fields.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "counts", "id", "limit", "mode", "note", "scope_id", "tags", "type"
        ]
    );
    let field = |name: &str| &record.fields[name];
    assert!(field("id").required);
    assert_eq!(field("id").ty, String::semantic_type());
    assert_eq!(
        field("id").meta.description.as_deref(),
        Some("The target id.")
    );
    assert!(!field("scope_id").required);
    assert_eq!(field("scope_id").ty, Option::<String>::semantic_type());
    assert!(field("note").required);
    assert!(matches!(
        field("note").ty.kind,
        TypeKind::Optional(OptionalType { .. })
    ));
    assert!(!field("mode").required);
    assert_eq!(field("mode").default, Some(string("auto_create")));
    assert!(!field("limit").required);
    assert_eq!(field("limit").default, Some(Value::U32(50)));
    assert_eq!(
        field("limit").ty.kind,
        TypeKind::Number(NumberType::UInt(UIntWidth::U32))
    );
    let TypeKind::Record(counts) = &field("counts").ty.kind else {
        panic!("counts must be a record")
    };
    assert_eq!(counts.additional.as_deref(), Some(&u64::semantic_type()));
}

#[test]
fn unit_enum_is_a_string_enum() {
    let TypeKind::Enum(ty) = Mode::semantic_type().kind else {
        panic!("expected enum")
    };
    assert_eq!(ty.repr, EnumRepr::String);
    let names: Vec<_> = ty.variants.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["open_existing", "auto_create", "other"]);
    assert_eq!(
        ty.variants[0].meta.description.as_deref(),
        Some("Open only existing databases.")
    );
    assert_eq!(Mode::Renamed.into_value(), string("other"));
    assert_eq!(Mode::from_value(string("other")).unwrap(), Mode::Renamed);
}

#[test]
fn newtype_is_transparent() {
    assert_eq!(Id::semantic_type(), String::semantic_type());
    assert_eq!(Id("x".into()).into_value(), string("x"));
    assert_eq!(Id::from_value(string("x")).unwrap(), Id("x".into()));
}

#[test]
fn flattened_fields_are_inlined() {
    let outer = Outer {
        payload: payload(),
        extra: true,
    };
    let value = outer.clone().into_value();
    let Value::Object(fields) = &value else {
        panic!("expected object")
    };
    assert_eq!(fields.get("extra"), Some(&Value::Bool(true)));
    assert_eq!(fields.get("id"), Some(&string("a")));
    assert_eq!(Outer::from_value(value).unwrap(), outer);

    let record = record(Outer::semantic_type());
    assert!(record.fields.contains_key("extra"));
    assert!(record.fields.contains_key("type"));
}

#[test]
fn tagged_enum_round_trips() {
    let create = Operation::Create {
        id: "a".into(),
        object: Object::new(),
    };
    let value = create.clone().into_value();
    assert_eq!(
        value,
        object([
            ("kind", string("create")),
            ("id", string("a")),
            ("object", object([])),
        ])
    );
    assert_eq!(Operation::from_value(value).unwrap(), create);
    assert_eq!(
        Operation::from_value(object([("kind", string("ddl"))])).unwrap(),
        Operation::Ddl
    );
    let err = Operation::from_value(object([
        ("kind", string("delete_by_ids")),
        ("ids", Value::Null),
    ]))
    .unwrap_err();
    assert_eq!(err.to_string(), "ids: expected list, found null");
    let err = Operation::from_value(object([("kind", string("x"))])).unwrap_err();
    assert_eq!(
        err.to_string(),
        "kind: unknown variant 'x', expected one of: create, delete_by_ids, ddl"
    );

    let TypeKind::Variant(ty) = Operation::semantic_type().kind else {
        panic!("expected variant")
    };
    assert_eq!(
        ty.tag,
        VariantTag::InternallyTagged {
            field: "kind".into()
        }
    );
    assert!(matches!(ty.variants[2].payload, VariantPayload::Unit));
    let VariantPayload::Record(fields) = &ty.variants[1].payload else {
        panic!("expected record payload")
    };
    assert!(fields.fields["ids"].required);
    assert!(!fields.fields["collection"].required);
}

#[test]
fn scalars_and_collections() {
    assert_eq!(u32::from_value(Value::I64(7)).unwrap(), 7);
    assert_eq!(u64::from_value(Value::U8(7)).unwrap(), 7);
    assert_eq!(
        u8::from_value(Value::I64(-1)).unwrap_err().to_string(),
        "integer -1 is out of range for u8"
    );
    assert_eq!(usize::semantic_type(), u64::semantic_type());
    assert_eq!(7usize.into_value(), Value::U64(7));
    assert_eq!(<()>::from_value(Value::Null), Ok(()));
    assert!(<()>::from_value(object([])).is_err());
    assert_eq!(().into_value(), Value::Void);
    assert_eq!(Option::<bool>::from_value(Value::Void), Ok(None));
    assert_eq!(None::<bool>.into_value(), Value::Null);
    assert_eq!(
        Value::semantic_type().kind,
        TypeKind::Any(semantic_data::schema::AnyType)
    );
}
