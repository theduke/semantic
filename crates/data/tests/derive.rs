use std::collections::BTreeMap;

use semantic_data::attr::{
    AttrCreatedAt, AttrDescriptor, AttrDescriptorConst, AttrId, AttrType, created_at_attribute,
};
use semantic_data::schema::{
    EnumRepr, NumberType, OptionalType, RecordType, Type, TypeKind, UIntWidth, VariantPayload,
    VariantTag,
};
use semantic_data::value::{DateTime, FromValue, IntoValue, Object, SemanticType, Value};

semantic_data::attrs! {
    /// A score.
    Score, "test:scoring:score", u32;
    Label, "test:label:name", String, name = "label";
}

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
#[semantic(namespace = "test:payload")]
struct Payload {
    /// The target id.
    #[semantic(attr = AttrId)]
    id: Id,
    scope_id: Option<String>,
    #[semantic(required)]
    note: Option<String>,
    #[semantic(default)]
    mode: Mode,
    #[semantic(default = "default_limit")]
    limit: u32,
    #[semantic(rename = "kind_name")]
    kind: String,
    tags: Vec<String>,
    counts: BTreeMap<String, u64>,
}

fn default_limit() -> u32 {
    50
}

#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
#[semantic(namespace = "test:outer")]
struct Outer {
    #[semantic(flatten)]
    payload: Payload,
    extra: bool,
}

#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
#[semantic(tag = "kind", rename_all = "snake_case", namespace = "test:op")]
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

/// Keyed by markers only, so no namespace is needed.
#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
struct Scored {
    #[semantic(attr = AttrType)]
    ty: String,
    #[semantic(attr = Score)]
    score: u32,
    #[semantic(attr = Label)]
    label: Option<String>,
    #[semantic(attr = AttrCreatedAt)]
    created_at: Option<DateTime>,
}

/// Tagged by the built-in `type`, which needs no namespace.
#[derive(SemanticType, IntoValue, FromValue, Debug, Clone, PartialEq, Eq)]
#[semantic(tag = "type", rename_all = "snake_case")]
enum Kind {
    Plain,
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
            ("test:payload:note", Value::Null),
            ("test:payload:mode", string("open_existing")),
            ("test:payload:limit", Value::U32(3)),
            ("test:payload:kind_name", string("k")),
            ("test:payload:tags", Value::List(vec![string("x")])),
            ("test:payload:counts", object([("c", Value::U64(1))])),
        ])
    );
    assert_eq!(Payload::from_value(value).unwrap(), payload());
}

#[test]
fn struct_decodes_defaults_nulls_and_ignores_unknown_fields() {
    let decoded = Payload::from_value(object([
        ("id", string("a")),
        ("test:payload:scope_id", Value::Null),
        ("test:payload:note", Value::Void),
        ("test:payload:kind_name", string("k")),
        ("test:payload:tags", Value::List(vec![])),
        ("test:payload:counts", object([])),
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
    assert_eq!(err.to_string(), "test:payload:note: missing required field");

    let mut fields = payload().into_value();
    let Value::Object(map) = &mut fields else {
        unreachable!()
    };
    map.insert(
        "test:payload:tags",
        Value::List(vec![string("x"), Value::Bool(false)]),
    );
    let err = Payload::from_value(fields).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test:payload:tags[1]: expected string, found boolean"
    );

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
            "id",
            "test:payload:counts",
            "test:payload:kind_name",
            "test:payload:limit",
            "test:payload:mode",
            "test:payload:note",
            "test:payload:scope_id",
            "test:payload:tags",
        ]
    );
    let field = |name: &str| {
        let key = if name == "id" {
            name.to_owned()
        } else {
            format!("test:payload:{name}")
        };
        &record.fields[&key]
    };
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
    assert_eq!(fields.get("test:outer:extra"), Some(&Value::Bool(true)));
    assert_eq!(fields.get("id"), Some(&string("a")));
    assert_eq!(fields.get("test:payload:kind_name"), Some(&string("k")));
    assert_eq!(Outer::from_value(value).unwrap(), outer);

    let record = record(Outer::semantic_type());
    assert!(record.fields.contains_key("test:outer:extra"));
    assert!(record.fields.contains_key("test:payload:kind_name"));
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
            ("test:op:kind", string("create")),
            ("test:op:id", string("a")),
            ("test:op:object", object([])),
        ])
    );
    assert_eq!(Operation::from_value(value).unwrap(), create);
    assert_eq!(
        Operation::from_value(object([("test:op:kind", string("ddl"))])).unwrap(),
        Operation::Ddl
    );
    // Plain names decode as aliases, and errors name the key that was given.
    let err = Operation::from_value(object([
        ("kind", string("delete_by_ids")),
        ("ids", Value::Null),
    ]))
    .unwrap_err();
    assert_eq!(err.to_string(), "ids: expected list, found null");
    let err = Operation::from_value(object([("test:op:kind", string("x"))])).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test:op:kind: unknown variant 'x', expected one of: create, delete_by_ids, ddl"
    );

    let TypeKind::Variant(ty) = Operation::semantic_type().kind else {
        panic!("expected variant")
    };
    assert_eq!(
        ty.tag,
        VariantTag::InternallyTagged {
            field: "test:op:kind".into()
        }
    );
    assert!(matches!(ty.variants[2].payload, VariantPayload::Unit));
    let VariantPayload::Record(fields) = &ty.variants[1].payload else {
        panic!("expected record payload")
    };
    assert!(fields.fields["test:op:ids"].required);
    assert!(!fields.fields["test:op:collection"].required);

    assert_eq!(
        Kind::Plain.into_value(),
        object([("type", string("plain"))])
    );
    assert_eq!(
        Kind::from_value(object([("type", string("plain"))])).unwrap(),
        Kind::Plain
    );
}

#[test]
fn plain_names_decode_as_aliases() {
    let decoded = Payload::from_value(object([
        ("id", string("a")),
        ("note", Value::Null),
        ("test:payload:mode", string("open_existing")),
        ("limit", Value::U32(3)),
        ("kind_name", string("k")),
        ("tags", Value::List(vec![string("x")])),
        ("counts", object([("c", Value::U64(1))])),
    ]))
    .unwrap();
    assert_eq!(decoded, payload());

    let scored = Scored::from_value(object([
        ("type", string("t")),
        ("score", Value::U32(2)),
        ("label", string("l")),
        ("created_at", Value::Null),
    ]))
    .unwrap();
    assert_eq!(scored.score, 2);
    assert_eq!(scored.label.as_deref(), Some("l"));
}

#[test]
fn qualified_id_and_alias_together_are_an_error() {
    let mut value = payload().into_value();
    let Value::Object(map) = &mut value else {
        unreachable!()
    };
    map.insert("limit", Value::U32(4));
    let err = Payload::from_value(value).unwrap_err();
    assert_eq!(
        err.to_string(),
        "test:payload:limit: field is also given by its alias 'limit'"
    );
}

#[test]
fn attr_markers_key_fields() {
    let scored = Scored {
        ty: "t".into(),
        score: 2,
        label: Some("l".into()),
        created_at: None,
    };
    let value = scored.clone().into_value();
    assert_eq!(
        value,
        object([
            ("type", string("t")),
            ("test:scoring:score", Value::U32(2)),
            ("test:label:name", string("l")),
        ])
    );
    assert_eq!(Scored::from_value(value).unwrap(), scored);

    let record = record(Scored::semantic_type());
    assert_eq!(
        record.fields.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "semantic:created_at",
            "test:label:name",
            "test:scoring:score",
            "type"
        ]
    );
    assert_eq!(record.fields["test:scoring:score"].ty, u32::semantic_type());
}

#[test]
fn attr_marker_describes_the_attribute() {
    assert_eq!(Score::ID, "test:scoring:score");
    assert_eq!(Score::PLAIN_NAME, "score");
    assert_eq!(Label::PLAIN_NAME, "label");
    let schema = Score::attr_schema();
    assert_eq!(schema.id, "test:scoring:score");
    assert_eq!(schema.name, "score");
    assert_eq!(schema.ty, u32::semantic_type());

    assert_eq!(AttrId::ID, "id");
    assert_eq!(AttrId::PLAIN_NAME, "id");
    assert_eq!(AttrType::ID, "type");
    assert_eq!(AttrCreatedAt::PLAIN_NAME, "created_at");
    assert_eq!(AttrCreatedAt::attr_schema(), created_at_attribute());
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
