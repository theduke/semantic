use super::*;
use crate::{BatchReturn, embedded::MemoryEntityStorage};
use semantic_data::schema::attribute::attribute_ref::AttributeRef;
use semantic_data::schema::{
    ArrayType, AttributeType, ClassAttribute, ClassConstraint, ClassRef, ClassType, Constraint,
    EnumRepr, EnumType, EnumVariant, Field, ForeignKeyRef, LengthSpec, MapType, Meta, RecordType,
    StringType, TupleType, TypeDef, TypeRef, UnionType, Visibility,
};

fn class(id: &str) -> ClassType {
    ClassType {
        id: id.into(),
        name: id.into(),
        inherits: None,
        extends: vec![],
        strict_schema: false,
        creatable_in_ui: None,
        attributes: BTreeMap::new(),
        constraints: vec![],
        meta: Meta::default(),
    }
}
fn attribute(id: &str, ty: Type) -> AttributeType {
    AttributeType {
        id: id.into(),
        name: id.into(),
        ty,
        constraints: vec![],
        meta: Meta::default(),
    }
}
fn field(ty: Type) -> Field {
    Field {
        ty,
        required: true,
        readonly: false,
        writeonly: false,
        default: None,
        meta: Meta::default(),
    }
}
fn class_attr(id: &str, required: bool) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef { id: id.into() },
        required,
        ui_order: None,
        computed: None,
        constraints: vec![],
        meta: Meta::default(),
    }
}
fn string() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}
fn reference(name: &str) -> Type {
    Type::new(TypeKind::Ref(TypeRef::new(name)))
}
fn fk() -> Constraint {
    Constraint::ForeignKey(ForeignKeyRef {
        to: TypeRef::new("v:PersonAlias"),
        fields: vec!["id".into()],
    })
}
fn schema(payload: Type) -> DdlBatch {
    let mut child = class("v:Child");
    child.inherits = Some(ClassRef {
        id: "v:Person".into(),
    });
    let mut holder = class("v:Holder");
    holder
        .attributes
        .insert("payload".into(), class_attr("v:payload", true));
    DdlBatch::new()
        .with_op(DdlOperation::UpsertAttribute {
            attribute: attribute("v:payload", payload),
        })
        .with_op(DdlOperation::UpsertTypeDef {
            type_def: TypeDef {
                name: "v:PersonAlias".into(),
                module: None,
                params: vec![],
                ty: reference("v:Person"),
                visibility: Visibility::Public,
                meta: Meta::default(),
            },
        })
        .with_op(DdlOperation::UpsertClass { class: holder })
        .with_op(DdlOperation::UpsertClass { class: child })
        .with_op(DdlOperation::UpsertClass {
            class: class("v:Person"),
        })
        .with_op(DdlOperation::UpsertClass {
            class: class("v:Other"),
        })
}
fn db(payload: Type) -> EmbeddedDb<MemoryEntityStorage> {
    let mut db = EmbeddedDb::new(MemoryEntityStorage::new());
    db.transact_ddl(schema(payload)).unwrap();
    db
}
fn row(id: &str, class: &str, payload: Option<Value>) -> Object {
    let mut object = Object::new();
    object.insert("id", id.to_string());
    object.insert("type", class.to_string());
    if let Some(value) = payload {
        object.insert("v:payload", value);
    }
    object
}
fn upsert(id: &str, class: &str, payload: Option<Value>) -> BatchOperation {
    BatchOperation::Upsert {
        collection: DEFAULT_COLLECTION.into(),
        id: id.into(),
        object: row(id, class, payload),
    }
}
fn delete(id: &str) -> BatchOperation {
    BatchOperation::DeleteById {
        collection: DEFAULT_COLLECTION.into(),
        id: id.into(),
    }
}
fn error(error: DbError) -> crate::ValidationError {
    let DbError::Validation(error) = error else {
        panic!("{error:?}");
    };
    error
}

#[test]
fn activation_preflight_is_non_mutating_and_persists_with_reverse_refs() {
    let mut scalar = string();
    scalar.constraints.push(fk());
    let mut db = db(scalar);
    db.transact(Batch::new().with_op(upsert("legacy", "v:Holder", None)))
        .unwrap();
    let revision = db.storage.current_revision().unwrap();
    let report = db.validation_preflight().unwrap();
    assert_eq!(report.len(), 1);
    assert_eq!(report[0].error.rule, "required");
    assert_eq!(db.storage.current_revision().unwrap(), revision);
    assert!(db.activate_validation().is_err());
    assert!(!db.validation_enabled().unwrap());
    db.transact(
        Batch::new()
            .with_op(upsert(
                "legacy",
                "v:Holder",
                Some(Value::String("target".into())),
            ))
            .with_op(upsert("target", "v:Child", None)),
    )
    .unwrap();
    db.activate_validation().unwrap();
    assert!(db.validation_enabled().unwrap());
    let mut db = EmbeddedDb::open(db.storage).unwrap();
    assert!(db.validation_enabled().unwrap());
    for mode in [BatchReturn::Stats, BatchReturn::Dataset] {
        let err = error(
            db.execute_batch_returning(Batch::new().with_op(delete("target")), mode.clone())
                .unwrap_err(),
        );
        assert_eq!(err.rule, "reference");
        assert_eq!(err.path, FieldPath::from_fields(["v:payload"]));
        let err = error(
            db.execute_batch_returning(
                Batch::new().with_op(upsert("target", "v:Other", None)),
                mode,
            )
            .unwrap_err(),
        );
        assert_eq!(err.rule, "reference_type");
    }
    db.execute_batch_returning(
        Batch::new()
            .with_op(delete("target"))
            .with_op(delete("legacy")),
        BatchReturn::Stats,
    )
    .unwrap();
}

#[test]
fn recursive_array_tuple_map_union_references_and_atomic_failure() {
    let refs = Type::new(TypeKind::Union(UnionType {
        variants: vec![reference("v:Other"), reference("v:PersonAlias")],
    }));
    let array = Type::new(TypeKind::Array(ArrayType {
        items: Box::new(refs),
        length: Some(LengthSpec::Exactly(1)),
    }));
    let tuple = Type::new(TypeKind::Tuple(TupleType {
        items: vec![array],
        rest: None,
    }));
    let payload = Type::new(TypeKind::Record(RecordType {
        fields: BTreeMap::from([("nested".into(), field(tuple))]),
        open: false,
        additional: None,
        required_order: None,
    }));
    let mut db = db(payload);
    db.activate_validation().unwrap();
    let nested = |id: &str| {
        Value::Object(Object::from_iter([(
            "nested".into(),
            Value::List(vec![Value::List(vec![Value::String(id.into())])]),
        )]))
    };
    db.execute_batch_returning(
        Batch::new()
            .with_op(upsert("owner", "v:Holder", Some(nested("target"))))
            .with_op(upsert("target", "v:Child", None)),
        BatchReturn::Stats,
    )
    .unwrap();
    for mode in [BatchReturn::Stats, BatchReturn::Dataset] {
        let err = error(
            db.execute_batch_returning(
                Batch::new().with_op(upsert("broken", "v:Holder", Some(nested("missing")))),
                mode.clone(),
            )
            .unwrap_err(),
        );
        assert_eq!(err.rule, "union");
        assert_eq!(
            err.path.0,
            vec![
                PathSegment::Field("v:payload".into()),
                PathSegment::Field("nested".into()),
                PathSegment::Index(0),
                PathSegment::Index(0)
            ]
        );
        assert!(db.get(DEFAULT_COLLECTION, "broken").unwrap().is_none());
        assert!(
            db.execute_batch_returning(Batch::new().with_op(delete("target")), mode)
                .is_err()
        );
    }
    let catalog = db.catalog();
    let refs = crate::validation::stored_references(
        &catalog,
        &(DEFAULT_COLLECTION.into(), "owner".into()),
        &db.get(DEFAULT_COLLECTION, "owner").unwrap().unwrap().object,
    );
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].target.1, "target");

    let map = Type::new(TypeKind::Map(MapType {
        keys: Box::new(reference("v:Person")),
        values: Box::new(reference("v:Person")),
        ordered: false,
    }));
    let mut map_db = self::db(map);
    map_db.activate_validation().unwrap();
    let mut entries = semantic_data::value::Map::new();
    entries.insert("key".to_string(), "value".to_string());
    let map = Value::Map(entries);
    map_db
        .execute_batch_returning(
            Batch::new()
                .with_op(upsert("owner", "v:Holder", Some(map)))
                .with_op(upsert("key", "v:Person", None))
                .with_op(upsert("value", "v:Child", None)),
            BatchReturn::Stats,
        )
        .unwrap();
    let err = error(
        map_db
            .execute_batch_returning(Batch::new().with_op(delete("key")), BatchReturn::Stats)
            .unwrap_err(),
    );
    assert_eq!(err.path.0.last(), Some(&PathSegment::Field("key".into())));
}

#[test]
fn required_nested_defaults_enum_and_constraints_are_pathful() {
    let mut name = string();
    name.constraints = vec![
        Constraint::Length(LengthSpec::Range {
            min: Some(2),
            max: Some(4),
        }),
        Constraint::Pattern("^[a-z]+$".into()),
        Constraint::Prefix("a".into()),
        Constraint::Suffix("z".into()),
    ];
    let mut optional = field(string());
    optional.required = false;
    let mut defaulted = field(string());
    defaulted.default = Some(Value::String("yes".into()));
    let enumeration = Type::new(TypeKind::Enum(EnumType {
        repr: EnumRepr::String,
        variants: vec![EnumVariant {
            name: "good".into(),
            symbol: None,
            value: None,
            meta: Meta::default(),
        }],
    }));
    let payload = Type::new(TypeKind::Record(RecordType {
        fields: BTreeMap::from([
            ("name".into(), field(name)),
            ("kind".into(), field(enumeration)),
            ("default".into(), defaulted),
            ("optional".into(), optional),
        ]),
        open: false,
        additional: None,
        required_order: None,
    }));
    let mut db = db(payload);
    db.activate_validation().unwrap();
    let payload = |name: &str, kind: &str| {
        Value::Object(Object::from_iter([
            ("name".into(), Value::String(name.into())),
            ("kind".into(), Value::String(kind.into())),
            ("optional".into(), Value::Null),
        ]))
    };
    db.execute_batch_returning(
        Batch::new().with_op(upsert("ok", "v:Holder", Some(payload("az", "good")))),
        BatchReturn::Stats,
    )
    .unwrap();
    let object = db.get(DEFAULT_COLLECTION, "ok").unwrap().unwrap().object;
    let Value::Object(nested) = object.get("v:payload").unwrap() else {
        panic!()
    };
    assert_eq!(nested.get("default"), Some(&Value::String("yes".into())));
    assert!(!nested.contains_key("optional"));
    for (name, kind, rule) in [
        ("a", "good", "length"),
        ("AZ", "good", "pattern"),
        ("bz", "good", "prefix"),
        ("ab", "good", "suffix"),
        ("az", "bad", "enum"),
    ] {
        let err = error(
            db.execute_batch_returning(
                Batch::new().with_op(upsert("bad", "v:Holder", Some(payload(name, kind)))),
                BatchReturn::Stats,
            )
            .unwrap_err(),
        );
        assert_eq!(err.rule, rule);
        assert_eq!(err.path.0.len(), 2);
    }
}

#[test]
fn builtin_endpoints_use_inherited_field_constraints_and_keep_unconstrained_behavior() {
    let mut db = db(string());
    let mut relation = class("v:Link");
    relation.inherits = Some(ClassRef {
        id: crate::catalog::RELATION_CLASS_ID.into(),
    });
    relation.constraints = vec![
        ClassConstraint::Field {
            attribute: AttributeRef {
                id: ATTR_RELATION_FROM.into(),
            },
            constraint: fk(),
        },
        ClassConstraint::Field {
            attribute: AttributeRef {
                id: ATTR_RELATION_TO.into(),
            },
            constraint: fk(),
        },
    ];
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertClass { class: relation }))
        .unwrap();
    db.activate_validation().unwrap();
    let relation = |id: &str, class: &str, from: &str, to: &str| {
        let mut object = row(id, class, None);
        object.insert(ATTR_RELATION_FROM, from.to_string());
        object.insert(ATTR_RELATION_TO, to.to_string());
        object.insert(ATTR_RELATION_RELATION, "unregistered".to_string());
        BatchOperation::Upsert {
            collection: DEFAULT_COLLECTION.into(),
            id: id.into(),
            object,
        }
    };
    db.execute_batch_returning(
        Batch::new().with_op(relation(
            "unconstrained",
            crate::catalog::RELATION_CLASS_ID,
            "missing",
            "also_missing",
        )),
        BatchReturn::Stats,
    )
    .unwrap();
    db.execute_batch_returning(
        Batch::new()
            .with_op(relation("link", "v:Link", "one", "two"))
            .with_op(upsert("one", "v:Person", None))
            .with_op(upsert("two", "v:Child", None)),
        BatchReturn::Stats,
    )
    .unwrap();
    let err = error(
        db.execute_batch_returning(Batch::new().with_op(delete("two")), BatchReturn::Stats)
            .unwrap_err(),
    );
    assert_eq!(err.attribute, ATTR_RELATION_TO);
    let err = error(
        db.execute_batch_returning(
            Batch::new().with_op(relation("bad", "v:Link", "missing", "two")),
            BatchReturn::Dataset,
        )
        .unwrap_err(),
    );
    assert_eq!(err.attribute, ATTR_RELATION_FROM);
}

#[test]
fn registration_rejects_unknown_foreign_targets_and_non_primary_tuples() {
    for foreign in [
        ForeignKeyRef {
            to: TypeRef::new("v:Missing"),
            fields: vec!["id".into()],
        },
        ForeignKeyRef {
            to: TypeRef::new("v:Person"),
            fields: vec!["name".into()],
        },
        ForeignKeyRef {
            to: TypeRef::new("v:Person"),
            fields: vec!["id".into(), "name".into()],
        },
    ] {
        let mut db = db(string());
        let mut attribute = attribute("v:bad", string());
        attribute.constraints.push(Constraint::ForeignKey(foreign));
        assert!(
            db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute { attribute }))
                .is_err()
        );
        assert!(db.catalog().attribute_by_id("v:bad").is_none());
    }
}

#[test]
fn required_stored_defaults_exclude_computed_and_reject_unsupported_constraints() {
    let mut db = db(string());
    let mut default = attribute("v:default", string());
    default.constraints.push(Constraint::DefaultValue {
        value: Value::String("filled".into()),
    });
    let mut holder = class("v:Defaults");
    holder
        .attributes
        .insert("default".into(), class_attr("v:default", true));
    let mut computed = class_attr("v:payload", true);
    computed.computed = Some(semantic_data::expr::Expr::Literal(
        semantic_data::expr::LiteralExpr {
            value: Value::String("computed".into()),
        },
    ));
    holder.attributes.insert("computed".into(), computed);
    db.transact_ddl(
        DdlBatch::new()
            .with_op(DdlOperation::UpsertAttribute { attribute: default })
            .with_op(DdlOperation::UpsertClass { class: holder }),
    )
    .unwrap();
    db.activate_validation().unwrap();
    db.execute_batch_returning(
        Batch::new().with_op(upsert("defaulted", "v:Defaults", None)),
        BatchReturn::Stats,
    )
    .unwrap();
    assert_eq!(
        db.get(DEFAULT_COLLECTION, "defaulted")
            .unwrap()
            .unwrap()
            .object
            .get("v:default"),
        Some(&Value::String("filled".into()))
    );

    let mut unsupported = attribute("v:unsupported", string());
    unsupported
        .constraints
        .push(Constraint::Contains(Value::String("x".into())));
    assert!(
        matches!(db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute { attribute: unsupported.clone() })).unwrap_err(), DbError::UnsupportedConstraint { kind } if kind == "contains")
    );
    assert!(db.catalog().attribute_by_id("v:unsupported").is_none());
    let mut legacy = self::db(string());
    legacy
        .transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute {
            attribute: unsupported,
        }))
        .unwrap();
    assert!(
        matches!(legacy.validation_preflight().unwrap_err(), DbError::UnsupportedConstraint { kind } if kind == "contains")
    );
    assert!(!legacy.validation_enabled().unwrap());
}

#[test]
fn active_class_upsert_validates_existing_rows_and_backfills_new_foreign_keys() {
    let mut db = db(string());
    db.activate_validation().unwrap();
    db.execute_batch_returning(
        Batch::new()
            .with_op(upsert(
                "owner",
                "v:Holder",
                Some(Value::String("target".into())),
            ))
            .with_op(upsert("target", "v:Person", None)),
        BatchReturn::Stats,
    )
    .unwrap();
    let mut holder = class("v:Holder");
    holder
        .attributes
        .insert("payload".into(), class_attr("v:payload", true));
    holder.constraints.push(ClassConstraint::Field {
        attribute: AttributeRef {
            id: "payload".into(),
        },
        constraint: fk(),
    });
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertClass { class: holder }))
        .unwrap();
    assert!(matches!(
        db.execute_batch_returning(Batch::new().with_op(delete("target")), BatchReturn::Stats),
        Err(DbError::Validation(_))
    ));
    let mut invalid = attribute("v:payload", string());
    invalid.constraints.push(Constraint::Prefix("z".into()));
    assert!(matches!(
        db.transact_ddl(
            DdlBatch::new().with_op(DdlOperation::UpsertAttribute { attribute: invalid })
        ),
        Err(DbError::Validation(_))
    ));
    assert!(
        db.catalog()
            .attribute_by_id("v:payload")
            .unwrap()
            .attribute
            .constraints
            .is_empty()
    );
}

#[test]
fn numeric_bounds_preserve_full_integer_precision_and_collection_constraints() {
    use semantic_data::schema::{ListType, NumberBound, NumberType, UIntWidth};
    let mut number = Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U128)));
    number.constraints = vec![
        Constraint::Min(NumberBound::Exclusive((u128::MAX - 2).to_string())),
        Constraint::Max(NumberBound::Inclusive(u128::MAX.to_string())),
    ];
    let mut payload = Type::new(TypeKind::List(ListType {
        items: Box::new(number),
    }));
    payload.constraints = vec![Constraint::MinItems(1), Constraint::MaxItems(2)];
    let mut db = db(payload);
    db.activate_validation().unwrap();
    db.execute_batch_returning(
        Batch::new().with_op(upsert(
            "ok",
            "v:Holder",
            Some(Value::List(vec![Value::U128(u128::MAX)])),
        )),
        BatchReturn::Stats,
    )
    .unwrap();
    for (values, rule) in [
        (vec![Value::U128(u128::MAX - 2)], "min"),
        (vec![], "min_items"),
        (vec![Value::U128(u128::MAX); 3], "max_items"),
    ] {
        assert_eq!(
            error(
                db.execute_batch_returning(
                    Batch::new().with_op(upsert("bad", "v:Holder", Some(Value::List(values)))),
                    BatchReturn::Stats
                )
                .unwrap_err()
            )
            .rule,
            rule
        );
    }
    let mut record = Type::new(TypeKind::Record(RecordType {
        fields: BTreeMap::from([("needed".into(), {
            let mut f = field(string());
            f.required = false;
            f
        })]),
        open: true,
        additional: None,
        required_order: None,
    }));
    record.constraints = vec![
        Constraint::MinProperties(1),
        Constraint::MaxProperties(2),
        Constraint::RequiredFields(vec!["needed".into()]),
    ];
    let mut db = self::db(record);
    db.activate_validation().unwrap();
    for (object, rule) in [
        (Object::new(), "min_properties"),
        (
            Object::from_iter([("other".into(), Value::String("x".into()))]),
            "required_fields",
        ),
        (
            Object::from_iter([
                ("needed".into(), Value::String("x".into())),
                ("b".into(), Value::Null),
                ("c".into(), Value::Null),
            ]),
            "max_properties",
        ),
    ] {
        assert_eq!(
            error(
                db.execute_batch_returning(
                    Batch::new().with_op(upsert("bad", "v:Holder", Some(Value::Object(object)))),
                    BatchReturn::Stats
                )
                .unwrap_err()
            )
            .rule,
            rule
        );
    }
}

#[test]
fn inline_classes_and_record_aliases_validate_nested_stored_values() {
    let mut nested = class("v:Inline");
    nested.strict_schema = true;
    nested
        .attributes
        .insert("target".into(), class_attr("v:target", true));
    let mut db = db(Type::new(TypeKind::Class(nested)));
    let mut target = attribute("v:target", string());
    target.constraints.push(fk());
    db.transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertAttribute { attribute: target }))
        .unwrap();
    db.activate_validation().unwrap();
    let nested = Value::Object(Object::from_iter([(
        "target".into(),
        Value::String("target".into()),
    )]));
    db.execute_batch_returning(
        Batch::new()
            .with_op(upsert("owner", "v:Holder", Some(nested)))
            .with_op(upsert("target", "v:Person", None)),
        BatchReturn::Stats,
    )
    .unwrap();
    let err = error(
        db.execute_batch_returning(Batch::new().with_op(delete("target")), BatchReturn::Stats)
            .unwrap_err(),
    );
    assert_eq!(err.path, FieldPath::from_fields(["v:payload", "v:target"]));

    let mut alias = self::db(reference("v:Record"));
    alias
        .transact_ddl(DdlBatch::new().with_op(DdlOperation::UpsertRecordType {
            id: "v:Record".into(),
            name: "Record".into(),
            record: RecordType {
                fields: BTreeMap::from([("target".into(), field(reference("v:Person")))]),
                open: false,
                additional: None,
                required_order: None,
            },
        }))
        .unwrap();
    alias.activate_validation().unwrap();
    let nested = Value::Object(Object::from_iter([(
        "target".into(),
        Value::String("target".into()),
    )]));
    alias
        .execute_batch_returning(
            Batch::new()
                .with_op(upsert("owner", "v:Holder", Some(nested)))
                .with_op(upsert("target", "v:Person", None)),
            BatchReturn::Stats,
        )
        .unwrap();
    let err = error(
        alias
            .execute_batch_returning(Batch::new().with_op(delete("target")), BatchReturn::Stats)
            .unwrap_err(),
    );
    assert_eq!(err.path, FieldPath::from_fields(["v:payload", "target"]));
}

#[test]
fn union_normalization_selects_structural_alternative_and_nullable_foreign_keys() {
    let mut default = field(string());
    default.default = Some(Value::String("filled".into()));
    let record = Type::new(TypeKind::Record(RecordType {
        fields: BTreeMap::from([("default".into(), default)]),
        open: false,
        additional: None,
        required_order: None,
    }));
    let union = Type::new(TypeKind::Union(UnionType {
        variants: vec![reference("v:Person"), record],
    }));
    let mut db = db(union);
    db.activate_validation().unwrap();
    db.execute_batch_returning(
        Batch::new().with_op(upsert(
            "owner",
            "v:Holder",
            Some(Value::Object(Object::new())),
        )),
        BatchReturn::Stats,
    )
    .unwrap();
    let row = db.get(DEFAULT_COLLECTION, "owner").unwrap().unwrap();
    assert!(
        matches!(row.object.get("v:payload"), Some(Value::Object(object)) if object.get("default") == Some(&Value::String("filled".into())))
    );
    let mut optional = Type::new(TypeKind::Optional(semantic_data::schema::OptionalType {
        inner: Box::new(string()),
    }));
    optional.constraints.push(fk());
    let mut db = self::db(optional);
    db.activate_validation().unwrap();
    db.execute_batch_returning(
        Batch::new().with_op(upsert("nullable", "v:Holder", Some(Value::Null))),
        BatchReturn::Stats,
    )
    .unwrap();
}
