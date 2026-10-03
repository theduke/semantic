trait ObjectRef {
    fn object_ref(&self) -> &Object;
}
impl ObjectRef for Value {
    fn object_ref(&self) -> &Object {
        match self {
            Value::Object(object) => object,
            _ => panic!("expected object"),
        }
    }
}

use super::*;

fn round_trip<T: IntoValue + FromValue + Clone + std::fmt::Debug + PartialEq>(value: T) {
    let encoded = value.clone().into_value();
    assert_eq!(T::from_value(encoded).unwrap(), value);
}

fn dynamic_values() -> Value {
    Value::List(vec![
        Value::Void,
        Value::Null,
        Value::Bool(true),
        Value::I8(-1),
        Value::I16(-2),
        Value::I32(-3),
        Value::I64(-4),
        Value::I128(i128::MIN),
        Value::U8(1),
        Value::U16(2),
        Value::U32(3),
        Value::U64(u64::MAX),
        Value::U128(u128::MAX),
        Value::F32(value::OrderedF32::from(1.5)),
        Value::F64(value::OrderedF64::from(f64::INFINITY)),
        Value::Bytes(bytes::Bytes::from_static(b"\0\xff")),
        Value::Uuid(value::Uuid::NIL),
        Value::IpAddr("::1".parse().unwrap()),
        Value::Date(time::OffsetDateTime::UNIX_EPOCH.date().into()),
        Value::Time(time::Time::MIDNIGHT.into()),
        Value::DateTime(time::OffsetDateTime::UNIX_EPOCH.into()),
        Value::Duration(time::Duration::nanoseconds(123).into()),
        Value::String("literal".into()),
        Value::Map({
            let mut map = value::Map::new();
            map.insert(Value::U8(1), Value::Null);
            map
        }),
        Value::Object(
            [("nested".into(), Value::U128(u128::MAX))]
                .into_iter()
                .collect(),
        ),
        Value::Variant(Box::new(value::VariantValue {
            r#type: Some("example:Variant".into()),
            variant: "case".into(),
            value: Value::U16(2),
        })),
    ])
}

#[test]
fn dynamic_literals_and_nested_parameter_queries_are_lossless() {
    let query = query::Query::Select(
        query::SelectQuery::new()
            .with_collection("items")
            .with_predicate(query::Expr::Binary {
                op: query::BinaryOp::Eq,
                left: Box::new(query::Expr::Subquery(Box::new(
                    query::SelectQuery::new().with_projection(vec![query::QueryField {
                        expr: Box::new(query::Expr::Operand(query::Operand::Parameter(
                            "id".into(),
                        ))),
                        alias: Some("value".into()),
                        wildcard: None,
                    }]),
                ))),
                right: Box::new(query::Expr::Operand(query::Operand::Literal(
                    dynamic_values(),
                ))),
            }),
    );
    let encoded = query.clone().into_value();
    let select = encoded.object_ref().get("select").unwrap().object_ref();
    let predicate = select
        .get("predicate")
        .unwrap()
        .object_ref()
        .get("binary")
        .unwrap()
        .object_ref();
    let right = predicate
        .get("right")
        .unwrap()
        .object_ref()
        .get("operand")
        .unwrap()
        .object_ref();
    assert_eq!(right.get("literal"), Some(&dynamic_values()));
    round_trip(query);
    round_trip(query::InsertSource::Objects(vec![
        Object::from_value(Value::Object(
            [("payload".into(), dynamic_values())].into_iter().collect(),
        ))
        .unwrap(),
    ]));
}

#[test]
fn presence_preserves_explicit_null_schema_defaults_and_discriminants() {
    let mut field = schema::Field {
        ty: bool::semantic_type(),
        required: false,
        readonly: false,
        writeonly: false,
        default: None,
        meta: schema::Meta::default(),
    };
    assert!(
        !field
            .clone()
            .into_value()
            .object_ref()
            .contains_key("default")
    );
    round_trip(field.clone());
    field.default = Some(Value::Null);
    assert_eq!(
        field.clone().into_value().object_ref().get("default"),
        Some(&Value::Null)
    );
    round_trip(field.clone());
    field.default = Some(dynamic_values());
    round_trip(field);
    for discriminant in [None, Some(Value::Null), Some(dynamic_values())] {
        round_trip(schema::VariantCase {
            name: "case".into(),
            payload: schema::VariantPayload::Unit,
            discriminant,
            meta: schema::Meta::default(),
        });
    }
}

#[test]
fn closed_records_and_default_fields_are_consistent() {
    let empty = Value::Object(Object::new());
    assert_eq!(
        query::SelectQuery::from_value(empty.clone()).unwrap(),
        query::SelectQuery::new()
    );
    let mut invalid = Object::new();
    invalid.insert("unknown", Value::Bool(true));
    assert_eq!(
        query::SelectQuery::from_value(Value::Object(invalid))
            .unwrap_err()
            .path(),
        "unknown"
    );
    let definition = definition::<query::SelectQuery>();
    let TypeKind::Record(record) = definition.ty.kind else {
        panic!("expected record");
    };
    for (name, field) in record.fields {
        assert!(!field.required, "{name} must have an omission rule");
        assert!(
            field.default.is_some() || field.meta.description.is_some(),
            "{name} must describe its default"
        );
    }
}

struct Resolver(BTreeMap<String, schema::TypeDef>);
impl schema::lowered::TypeResolver for Resolver {
    fn resolve_type_def(&self, name: &str) -> Option<&schema::TypeDef> {
        self.0.get(name)
    }
}

#[test]
fn complete_definition_graph_matches_frozen_migration_and_round_trips() {
    let current = definitions();
    let frozen = crate::bundles::query::migration_v1()
        .operations
        .into_iter()
        .map(|op| {
            let schema::MigrationOperation::Ddl(schema::MigrationDdlOperation::UpsertTypeDef {
                type_def,
            }) = op
            else {
                panic!("expected type definition");
            };
            (type_def.name.clone(), type_def)
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(current.len(), frozen.len());
    for (name, definition) in &current {
        assert_eq!(Some(definition), frozen.get(name), "{name}");
    }
    for definition in current.values() {
        round_trip(definition.clone());
    }
    let resolver = Resolver(current);
    for definition in resolver.0.values() {
        schema::lowered::lower_type_def(definition, &resolver)
            .unwrap_or_else(|error| panic!("{} failed to resolve: {error}", definition.name));
    }
}

mod fixtures;

#[test]
fn legacy_optional_index_and_text_match_settings_keep_their_defaults() {
    let operation = Value::Object(
        [(
            "upsert_index".into(),
            Value::Object(
                [
                    ("name".into(), Value::String("by_title".into())),
                    ("collection".into(), Value::String("items".into())),
                    ("field".into(), Value::String("title".into())),
                    ("unique".into(), Value::Bool(false)),
                ]
                .into_iter()
                .collect(),
            ),
        )]
        .into_iter()
        .collect(),
    );
    let query::DdlOperation::UpsertIndex {
        kind,
        extra_fields,
        predicate,
        analyzer,
        ..
    } = query::DdlOperation::from_value(operation).unwrap()
    else {
        panic!("expected index");
    };
    assert_eq!(kind, schema::IndexKind::Equality);
    assert!(extra_fields.is_empty());
    assert!(predicate.is_none());
    assert_eq!(analyzer, query::TextAnalyzer::default());

    let expression = Value::Object(
        [(
            "text_match".into(),
            Value::Object(
                [
                    ("exprs".into(), Value::List(Vec::new())),
                    (
                        "query".into(),
                        query::Expr::Operand(query::Operand::Parameter("text".into())).into_value(),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        )]
        .into_iter()
        .collect(),
    );
    let query::Expr::TextMatch { mode, analyzer, .. } =
        query::Expr::from_value(expression).unwrap()
    else {
        panic!("expected text match");
    };
    assert_eq!(mode, query::TextMatchMode::All);
    assert_eq!(analyzer, query::TextAnalyzer::default());
}

#[test]
fn ddl_nested_schema_constraints_and_general_expressions_are_lossless() {
    let mut ty = Type::new(TypeKind::Record(schema::RecordType {
        fields: [(
            "payload".into(),
            schema::Field {
                ty: Type::new(TypeKind::Optional(schema::OptionalType {
                    inner: Box::new(Type::new(TypeKind::List(schema::ListType {
                        items: Box::new(Type::new(TypeKind::Named(schema::TypeRef::new(
                            "example:Node",
                        )))),
                    }))),
                })),
                required: false,
                readonly: false,
                writeonly: false,
                default: Some(Value::Null),
                meta: schema::Meta {
                    examples: vec![dynamic_values()],
                    ..Default::default()
                },
            },
        )]
        .into_iter()
        .collect(),
        open: true,
        additional: Some(Box::new(Value::semantic_type())),
        required_order: None,
    }));
    ty.constraints.push(schema::Constraint::DefaultExpr {
        expr: expr::Expr::Cast(Box::new(expr::CastExpr {
            expr: expr::Expr::If(Box::new(expr::IfExpr {
                condition: expr::Expr::Ref(expr::RefExpr::Parameter(expr::ParameterRef {
                    name: "flag".into(),
                })),
                then_expr: expr::Expr::Literal(expr::LiteralExpr {
                    value: dynamic_values(),
                }),
                else_expr: expr::Expr::Literal(expr::LiteralExpr { value: Value::Null }),
            })),
            to: Value::semantic_type(),
            safe: true,
        })),
    });
    ty.annotations.push(schema::Annotation {
        key: "example:annotation".into(),
        value: schema::AnnotationValue::Map(
            [(
                "nested".into(),
                schema::AnnotationValue::List(vec![
                    schema::AnnotationValue::Bool(true),
                    schema::AnnotationValue::Number("1.234567890123456789".into()),
                ]),
            )]
            .into_iter()
            .collect(),
        ),
    });
    round_trip(query::Query::Ddl(query::DdlQuery {
        batch: query::DdlBatch::new().with_op(query::DdlOperation::UpsertTypeDef {
            type_def: schema::TypeDef {
                name: "example:Node".into(),
                module: Some("example".into()),
                params: Vec::new(),
                ty,
                visibility: schema::Visibility::Public,
                meta: schema::Meta {
                    examples: vec![dynamic_values()],
                    ..Default::default()
                },
            },
        }),
    }));
}
