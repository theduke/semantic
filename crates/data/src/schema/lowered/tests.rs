use std::collections::BTreeMap;

use super::*;
use crate::schema::{
    AnyType, ArrayType, AttributeRef, AttributeType, BigIntType, ClassAttribute, ClassConstraint,
    ClassRef, ClassType, DecimalEncoding, DecimalType, ExtensionType, Field, FunctionType,
    HandleMode, HandleType, InterfaceType, IntersectionType, ListType, Meta, NeverType,
    NumberBound, NumberType, OpaqueType, OptionalType, RecordType, ResultType, SetType, StreamType,
    TemporalType, TimeUnit, TimeZoneSpec, TimestampType, Type, TypeDef, TypeKind, TypeParam,
    TypeRef, UnknownType, Visibility,
};

#[derive(Default)]
struct Defs(BTreeMap<String, TypeDef>);

impl Defs {
    fn with(mut self, name: &str, ty: Type) -> Self {
        self.0.insert(name.to_string(), type_def(name, ty));
        self
    }

    fn with_attribute(self, id: &str, ty: Type) -> Self {
        self.with(
            id,
            Type::new(TypeKind::Attribute(Box::new(attribute(id, ty)))),
        )
    }

    fn with_class(self, class: ClassType) -> Self {
        let id = class.id.clone();
        self.with(&id, Type::new(TypeKind::Class(class)))
    }
}

impl TypeResolver for Defs {
    fn resolve_type_def(&self, name: &str) -> Option<&TypeDef> {
        self.0.get(name)
    }
}

fn type_def(name: &str, ty: Type) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        module: None,
        params: Vec::new(),
        ty,
        visibility: Visibility::Public,
        meta: Meta::default(),
    }
}

fn attribute(id: &str, ty: Type) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: id.to_string(),
        ty,
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn class(id: &str, attributes: &[(&str, bool)]) -> ClassType {
    ClassType {
        id: id.to_string(),
        name: id.to_string(),
        inherits: None,
        extends: Vec::new(),
        strict_schema: false,
        creatable_in_ui: None,
        include_in_ui_listings: None,
        attributes: attributes
            .iter()
            .map(|(id, required)| {
                (
                    id.to_string(),
                    ClassAttribute {
                        attribute: AttributeRef { id: id.to_string() },
                        required: *required,
                        ui_order: None,
                        computed: None,
                        default: None,
                        constraints: Vec::new(),
                        meta: Meta::default(),
                    },
                )
            })
            .collect(),
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn string() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn int() -> Type {
    Type::new(TypeKind::Number(NumberType::Int(IntWidth::I64)))
}

fn list(items: Type) -> Type {
    Type::new(TypeKind::List(ListType {
        items: Box::new(items),
    }))
}

fn named(name: &str) -> Type {
    Type::new(TypeKind::Named(TypeRef::new(name)))
}

fn function() -> Type {
    Type::new(TypeKind::Function(FunctionType {
        params: Vec::new(),
        results: Vec::new(),
        throws: None,
        async_fn: false,
    }))
}

fn record(fields: &[(&str, Type)], open: bool) -> Type {
    Type::new(TypeKind::Record(RecordType {
        fields: fields
            .iter()
            .map(|(name, ty)| {
                (
                    name.to_string(),
                    Field {
                        ty: ty.clone(),
                        required: true,
                        readonly: false,
                        writeonly: false,
                        default: None,
                        meta: Meta::default(),
                    },
                )
            })
            .collect(),
        open,
        additional: None,
        required_order: None,
    }))
}

fn min(value: &str) -> Constraint {
    Constraint::Min(NumberBound::Inclusive(value.to_string()))
}

fn lower(ty: Type) -> Result<DataType, LowerError> {
    lower_type(&ty, &Defs::default())
}

fn lowered_kind(ty: Type) -> DataKind {
    lower(ty).unwrap().kind
}

fn error_kind(ty: Type) -> LowerErrorKind {
    lower(ty).unwrap_err().kind
}

fn not_storable(kind: &'static str) -> LowerErrorKind {
    LowerErrorKind::NotStorable { kind }
}

#[test]
fn primitives_lower_directly() {
    assert_eq!(lowered_kind(Type::new_bool()), DataKind::Bool);
    assert_eq!(
        lowered_kind(Type::new(TypeKind::Any(AnyType))),
        DataKind::Any
    );
    assert_eq!(lowered_kind(Type::new(TypeKind::Uuid)), DataKind::Uuid);
    assert_eq!(lowered_kind(Type::new(TypeKind::Json)), DataKind::Json);
    assert!(matches!(lowered_kind(string()), DataKind::String(_)));

    let mut constrained = int();
    constrained.constraints.push(min("1"));
    let lowered = lower(constrained).unwrap();
    assert_eq!(
        lowered.kind,
        DataKind::Number(NumberRepr::Int(IntWidth::I64))
    );
    assert_eq!(lowered.constraints, vec![min("1")]);
}

#[test]
fn numbers_without_value_representation_fail() {
    for (number, repr) in [
        (NumberType::Int(IntWidth::I256), "i256"),
        (NumberType::UInt(UIntWidth::U256), "u256"),
        (NumberType::Float(FloatWidth::F128), "f128"),
        (NumberType::Float(FloatWidth::Decimal64), "decimal64"),
        (
            NumberType::BigInt(BigIntType {
                min_bits: None,
                max_bits: None,
            }),
            "big_int",
        ),
        (
            NumberType::Decimal(DecimalType {
                precision: None,
                scale: None,
                encoding: DecimalEncoding::String,
            }),
            "decimal",
        ),
    ] {
        assert_eq!(
            error_kind(Type::new(TypeKind::Number(number))),
            LowerErrorKind::UnrepresentableNumber { repr }
        );
    }
    for number in [
        NumberType::Int(IntWidth::I24),
        NumberType::UInt(UIntWidth::U128),
        NumberType::Float(FloatWidth::F16),
        NumberType::Unspecified,
    ] {
        assert!(lower(Type::new(TypeKind::Number(number))).is_ok());
    }
}

#[test]
fn temporals_without_value_representation_fail() {
    assert_eq!(
        error_kind(Type::new(TypeKind::Temporal(TemporalType::Period))),
        LowerErrorKind::UnrepresentableTemporal { repr: "period" }
    );
    assert_eq!(
        error_kind(Type::new(TypeKind::Temporal(TemporalType::Instant))),
        LowerErrorKind::UnrepresentableTemporal { repr: "instant" }
    );
    assert_eq!(
        lowered_kind(Type::new(TypeKind::Temporal(TemporalType::Date))),
        DataKind::Temporal(TemporalRepr::Date)
    );
    assert_eq!(
        lowered_kind(Type::new(TypeKind::Temporal(TemporalType::Timestamp(
            TimestampType {
                unit: TimeUnit::Micros,
                timezone: TimeZoneSpec::Forbidden,
            }
        )))),
        DataKind::Temporal(TemporalRepr::DateTime)
    );
}

#[test]
fn behavioural_and_uninhabited_kinds_fail() {
    assert_eq!(error_kind(function()), not_storable("function"));
    assert_eq!(
        error_kind(Type::new(TypeKind::Interface(InterfaceType {
            methods: Vec::new()
        }))),
        not_storable("interface")
    );
    assert_eq!(
        error_kind(Type::new(TypeKind::Handle(HandleType {
            interface: TypeRef::new("x:Api"),
            mode: HandleMode::Own,
        }))),
        not_storable("handle")
    );
    assert_eq!(
        error_kind(Type::new(TypeKind::Stream(StreamType {
            element: Box::new(int()),
            end: None,
        }))),
        not_storable("stream")
    );
    assert_eq!(
        error_kind(Type::new(TypeKind::Never(NeverType))),
        not_storable("never")
    );
    assert_eq!(
        error_kind(Type::new(TypeKind::Unknown(UnknownType))),
        not_storable("unknown")
    );
    assert_eq!(
        error_kind(Type::new(TypeKind::Opaque(OpaqueType {
            id: "x".into(),
            domain: None,
            repr: None,
        }))),
        not_storable("opaque")
    );
}

#[test]
fn extensions_are_kept_opaque() {
    let extension = ExtensionType {
        namespace: "x".into(),
        name: "y".into(),
        payload: BTreeMap::new(),
    };
    assert_eq!(
        lowered_kind(Type::new(TypeKind::Extension(extension.clone()))),
        DataKind::Extension(extension)
    );
}

#[test]
fn arrays_and_sets_lower_to_lists() {
    let array = Type::new(TypeKind::Array(ArrayType {
        items: Box::new(int()),
        length: Some(LengthSpec::Exactly(3)),
    }));
    let DataKind::List(array) = lowered_kind(array) else {
        panic!("array should lower to a list");
    };
    assert_eq!(array.length, Some(LengthSpec::Exactly(3)));
    assert!(!array.distinct);

    let set = Type::new(TypeKind::Set(SetType {
        items: Box::new(string()),
    }));
    let DataKind::List(set) = lowered_kind(set) else {
        panic!("set should lower to a list");
    };
    assert!(set.distinct);
    assert!(matches!(set.items.kind, DataKind::String(_)));
}

#[test]
fn results_lower_to_two_case_variants() {
    let result = Type::new(TypeKind::Result(ResultType {
        ok: Box::new(int()),
        err: Box::new(string()),
    }));
    let DataKind::Variant(variant) = lowered_kind(result) else {
        panic!("result should lower to a variant");
    };
    assert_eq!(variant.tag, VariantTag::ExternallyTagged);
    let names = variant
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["ok", "err"]);
}

#[test]
fn nested_errors_report_their_path() {
    let err = lower(record(&[("items", list(function()))], false)).unwrap_err();
    assert_eq!(
        err.path,
        vec![
            LowerPathSegment::Field("items".into()),
            LowerPathSegment::ListItem
        ]
    );
    assert_eq!(
        err.in_context("attribute 'x'"),
        "attribute 'x' field 'items' list item: function types cannot be stored"
    );
    assert_eq!(
        lower(function()).unwrap_err().in_context("attribute 'x'"),
        "attribute 'x': function types cannot be stored"
    );
}

#[test]
fn named_types_resolve_and_merge_reference_constraints() {
    let mut positive = int();
    positive.constraints.push(min("0"));
    let defs = Defs::default().with("x:Positive", positive);
    let mut reference = named("x:Positive");
    reference.constraints.push(min("10"));
    let lowered = lower_type(&reference, &defs).unwrap();
    assert_eq!(
        lowered.kind,
        DataKind::Number(NumberRepr::Int(IntWidth::I64))
    );
    assert_eq!(lowered.constraints, vec![min("0"), min("10")]);
}

#[test]
fn named_types_must_resolve_to_data() {
    let defs = Defs::default().with(
        "x:Api",
        Type::new(TypeKind::Interface(InterfaceType {
            methods: Vec::new(),
        })),
    );
    let err = lower_type(&list(named("x:Api")), &defs).unwrap_err();
    assert_eq!(err.kind, not_storable("interface"));
    assert_eq!(
        err.path,
        vec![
            LowerPathSegment::ListItem,
            LowerPathSegment::Named("x:Api".into())
        ]
    );
    assert_eq!(
        lower(named("x:Missing")).unwrap_err().kind,
        LowerErrorKind::UnresolvedType {
            name: "x:Missing".into()
        }
    );
}

#[test]
fn generics_are_rejected() {
    let defs = Defs::default().with("x:Box", int());
    let mut applied = TypeRef::new("x:Box");
    applied.args.push(int());
    assert_eq!(
        lower_type(&Type::new(TypeKind::Named(applied)), &defs)
            .unwrap_err()
            .kind,
        LowerErrorKind::GenericArguments {
            name: "x:Box".into()
        }
    );

    let mut generic = type_def("x:Generic", named("T"));
    generic.params.push(TypeParam {
        name: "T".into(),
        bounds: Vec::new(),
        default: None,
    });
    let mut defs = Defs::default();
    defs.0.insert(generic.name.clone(), generic.clone());
    assert_eq!(
        lower_type(&named("x:Generic"), &defs).unwrap_err().kind,
        LowerErrorKind::GenericDefinition {
            name: "x:Generic".into()
        }
    );
    assert_eq!(
        lower_type_def(&generic, &defs).unwrap_err().kind,
        LowerErrorKind::GenericDefinition {
            name: "x:Generic".into()
        }
    );
}

#[test]
fn recursive_definitions_use_back_references() {
    let defs = Defs::default().with(
        "x:Tree",
        record(&[("children", list(named("x:Tree")))], false),
    );
    let lowered = lower_type_def(&defs.0["x:Tree"], &defs).unwrap();
    let DataKind::Record(record) = lowered.kind else {
        panic!("tree should lower to a record");
    };
    let DataKind::List(children) = &record.fields["children"].ty.kind else {
        panic!("children should lower to a list");
    };
    assert_eq!(children.items.kind, DataKind::Recursive("x:Tree".into()));

    // Referenced from elsewhere, the definition is inlined once.
    let lowered = lower_type(&named("x:Tree"), &defs).unwrap();
    assert!(matches!(lowered.kind, DataKind::Record(_)));
}

#[test]
fn alias_cycles_fail() {
    let defs = Defs::default().with("x:A", named("x:B")).with(
        "x:B",
        Type::new(TypeKind::Optional(OptionalType {
            inner: Box::new(named("x:A")),
        })),
    );
    assert_eq!(
        lower_type(&named("x:A"), &defs).unwrap_err().kind,
        LowerErrorKind::AliasCycle { name: "x:A".into() }
    );
}

#[test]
fn record_intersections_merge() {
    let intersection = Type::new(TypeKind::Intersection(IntersectionType {
        variants: vec![
            record(&[("a", int())], true),
            record(&[("b", string())], false),
        ],
    }));
    let DataKind::Record(merged) = lowered_kind(intersection) else {
        panic!("intersection should lower to a record");
    };
    assert_eq!(merged.fields.keys().collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(merged.rest, RecordRest::Closed);

    let conflicting = Type::new(TypeKind::Intersection(IntersectionType {
        variants: vec![
            record(&[("a", int())], true),
            record(&[("a", string())], true),
        ],
    }));
    assert_eq!(
        error_kind(conflicting),
        LowerErrorKind::IntersectionConflict {
            field: Some("a".into())
        }
    );

    let mixed = Type::new(TypeKind::Intersection(IntersectionType {
        variants: vec![record(&[("a", int())], true), int()],
    }));
    assert_eq!(error_kind(mixed), LowerErrorKind::IntersectionNotRecord);
}

#[test]
fn inline_attributes_lower_to_their_value_type() {
    let mut attr = attribute("x:count", int());
    attr.constraints.push(min("1"));
    let lowered = lower(Type::new(TypeKind::Attribute(Box::new(attr)))).unwrap();
    assert_eq!(
        lowered.kind,
        DataKind::Number(NumberRepr::Int(IntWidth::I64))
    );
    assert_eq!(lowered.constraints, vec![min("1")]);
}

#[test]
fn embedded_classes_lower_to_class_records() {
    let mut base = class("x:Base", &[("x:name", true)]);
    base.constraints.push(ClassConstraint::Field {
        attribute: AttributeRef {
            id: "x:name".into(),
        },
        constraint: Constraint::Length(LengthSpec::Exactly(2)),
    });
    let defs = Defs::default()
        .with_attribute("x:name", string())
        .with_attribute("x:size", int())
        .with_class(base);

    let mut child = class("x:Inline", &[("x:size", false), ("x:missing", true)]);
    child.inherits = Some(ClassRef {
        id: "x:Base".into(),
    });
    child.strict_schema = true;
    let lowered = lower_type(&Type::new(TypeKind::Class(child)), &defs).unwrap();
    let DataKind::Record(record) = lowered.kind else {
        panic!("class should lower to a record");
    };
    assert_eq!(record.class.as_deref(), Some("x:Inline"));
    assert_eq!(record.rest, RecordRest::Closed);
    // Unresolved attributes are skipped, matching stored-value validation.
    assert_eq!(
        record.fields.keys().collect::<Vec<_>>(),
        ["x:name", "x:size"]
    );
    assert!(record.fields["x:name"].required);
    assert_eq!(
        record.fields["x:name"].ty.constraints,
        vec![Constraint::Length(LengthSpec::Exactly(2))]
    );
    assert!(!record.fields["x:size"].required);
}

#[test]
fn embedded_classes_reject_unstorable_attributes() {
    let defs = Defs::default().with_attribute("x:callback", function());
    let err = lower_type(
        &Type::new(TypeKind::Class(class("x:Inline", &[("x:callback", true)]))),
        &defs,
    )
    .unwrap_err();
    assert_eq!(err.kind, not_storable("function"));
    assert_eq!(
        err.path,
        vec![LowerPathSegment::Attribute("x:callback".into())]
    );
}

#[test]
fn self_embedding_class_attributes_use_back_references() {
    let defs = Defs::default().with_attribute(
        "x:children",
        list(Type::new(TypeKind::Class(class(
            "x:Node",
            &[("x:children", false)],
        )))),
    );
    let lowered = lower_type_def(&defs.0["x:children"], &defs).unwrap();
    let DataKind::List(list) = lowered.kind else {
        panic!("attribute should lower to a list");
    };
    let DataKind::Record(node) = list.items.kind else {
        panic!("list items should lower to a record");
    };
    assert_eq!(
        node.fields["x:children"].ty.kind,
        DataKind::Recursive("x:children".into())
    );
}
