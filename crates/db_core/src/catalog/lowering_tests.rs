//! Catalog enforcement of storable (lowered) data types.

use std::collections::BTreeMap;

use semantic_data::schema::{
    AttributeType, Field, FunctionType, InterfaceType, Meta, RecordType, StreamType, StringType,
    Type, TypeDef, TypeKind, TypeRef, Visibility,
    lowered::{DataKind, LowerErrorKind},
};

use crate::catalog::{
    Catalog, CatalogBatchOperation, CatalogError, CollectionKind, IntegrityMode, TypeDefData,
};

fn string() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn function() -> Type {
    Type::new(TypeKind::Function(FunctionType {
        params: Vec::new(),
        results: Vec::new(),
        throws: None,
        async_fn: false,
    }))
}

fn interface() -> Type {
    Type::new(TypeKind::Interface(InterfaceType {
        methods: Vec::new(),
    }))
}

fn named(name: &str) -> Type {
    Type::new(TypeKind::Named(TypeRef::new(name)))
}

fn attribute_type(id: &str, ty: Type) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: id.to_string(),
        ty,
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn attribute(id: &str, ty: Type) -> CatalogBatchOperation {
    CatalogBatchOperation::UpsertAttribute {
        attribute: attribute_type(id, ty),
        module: None,
    }
}

fn type_def(name: &str, ty: Type) -> CatalogBatchOperation {
    CatalogBatchOperation::UpsertTypeDef {
        type_def: TypeDef {
            name: name.to_string(),
            module: None,
            params: Vec::new(),
            ty,
            visibility: Visibility::Public,
            meta: Meta::default(),
        },
    }
}

fn record(field: &str, ty: Type) -> RecordType {
    RecordType {
        fields: BTreeMap::from([(
            field.to_string(),
            Field {
                ty,
                required: true,
                readonly: false,
                writeonly: false,
                default: None,
                meta: Meta::default(),
            },
        )]),
        open: false,
        additional: None,
        required_order: None,
    }
}

fn data(catalog: &Catalog, name: &str) -> TypeDefData {
    catalog.type_def_by_name(name).unwrap().data.clone()
}

fn unstorable(result: Result<(), CatalogError>) -> (String, LowerErrorKind) {
    match result.unwrap_err() {
        CatalogError::UnstorableType { definition, error } => (definition, error.kind),
        other => panic!("expected unstorable type error, got {other:?}"),
    }
}

#[test]
fn rejects_attributes_that_do_not_lower() {
    let mut catalog = Catalog::new();
    let err = catalog
        .apply_batch(&[attribute("x:callback", function())])
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid schema: attribute 'x:callback': function types cannot be stored"
    );

    let nested = Type::new(TypeKind::List(semantic_data::schema::ListType {
        items: Box::new(function()),
    }));
    let err = Catalog::new()
        .apply_batch(&[attribute("x:callbacks", nested)])
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "invalid schema: attribute 'x:callbacks' list item: function types cannot be stored"
    );
}

#[test]
fn rejects_record_types_that_do_not_lower() {
    let stream = Type::new(TypeKind::Stream(StreamType {
        element: Box::new(string()),
        end: None,
    }));
    let (definition, kind) =
        unstorable(
            Catalog::new().apply_batch(&[CatalogBatchOperation::UpsertRecordType {
                id: "x:Rec".into(),
                name: "Rec".into(),
                record: record("events", stream),
                module: None,
            }]),
        );
    assert_eq!(definition, "record type 'x:Rec'");
    assert_eq!(kind, LowerErrorKind::NotStorable { kind: "stream" });
}

#[test]
fn tags_type_defs_and_rejects_attributes_naming_interface_only_types() {
    let mut catalog = Catalog::new();
    catalog
        .apply_batch(&[type_def("x:Api", interface()), type_def("x:Name", string())])
        .unwrap();
    assert!(matches!(
        data(&catalog, "x:Api"),
        TypeDefData::Unstorable(error)
            if error.kind == LowerErrorKind::NotStorable { kind: "interface" }
    ));
    assert!(matches!(
        data(&catalog, "x:Name").data_type().map(|ty| &ty.kind),
        Some(DataKind::String(_))
    ));

    let (definition, kind) = unstorable(catalog.apply_batch(&[attribute("x:api", named("x:Api"))]));
    assert_eq!(definition, "attribute 'x:api'");
    assert_eq!(kind, LowerErrorKind::NotStorable { kind: "interface" });

    let (_, kind) = unstorable(catalog.apply_batch(&[attribute("x:missing", named("x:Nope"))]));
    assert_eq!(
        kind,
        LowerErrorKind::UnresolvedType {
            name: "x:Nope".into()
        }
    );
}

#[test]
fn lowering_sees_definitions_registered_later_in_the_same_batch() {
    let mut catalog = Catalog::new();
    catalog
        .apply_batch(&[
            attribute("x:address", named("x:Address")),
            CatalogBatchOperation::UpsertCollection {
                name: "x:things".into(),
                kind: CollectionKind::Schema,
                integrity_mode: IntegrityMode::Permissive,
            },
            CatalogBatchOperation::UpsertRecordType {
                id: "x:Address".into(),
                name: "Address".into(),
                record: record("street", string()),
                module: None,
            },
        ])
        .unwrap();
    let address = data(&catalog, "x:address");
    assert!(matches!(
        address.data_type().map(|ty| &ty.kind),
        Some(DataKind::Record(record)) if record.fields.contains_key("street")
    ));
}

#[test]
fn rejects_redefining_a_referenced_type_as_interface_only() {
    let mut catalog = Catalog::new();
    catalog
        .apply_batch(&[
            type_def("x:Name", string()),
            attribute("x:name", named("x:Name")),
        ])
        .unwrap();
    let (definition, _) = unstorable(catalog.apply_batch(&[type_def("x:Name", interface())]));
    assert_eq!(definition, "attribute 'x:name'");
}

#[test]
fn persisted_unstorable_definitions_keep_loading_and_are_tolerated() {
    // Direct registration is unvalidated; it stands in for a catalog written
    // before lowering was enforced.
    let mut legacy = Catalog::new();
    let _ = legacy.upsert_attribute(attribute_type("x:callback", function()));

    let mut catalog = Catalog::from_storage_snapshot(legacy.to_storage_snapshot()).unwrap();
    assert!(matches!(
        data(&catalog, "x:callback"),
        TypeDefData::Unstorable(_)
    ));

    // Unrelated DDL and re-applying the unchanged legacy definition succeed.
    catalog
        .apply_batch(&[
            attribute("x:title", string()),
            attribute("x:callback", function()),
        ])
        .unwrap();

    // Changing the legacy definition to another unstorable type is new DDL.
    let stream = Type::new(TypeKind::Stream(StreamType {
        element: Box::new(string()),
        end: None,
    }));
    let (definition, _) = unstorable(catalog.apply_batch(&[attribute("x:callback", stream)]));
    assert_eq!(definition, "attribute 'x:callback'");
}
