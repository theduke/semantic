use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use dioxus::prelude::*;
use dxform::{FieldSpec, FormOptions, FormRoot, SubformSpec, SubmitHandler};
use semantic_data::{
    schema::{
        AttributeRef, AttributeType, BoolType, ClassAttribute, ClassRef, ClassType, Constraint,
        LengthSpec, Meta, NumberType, OptionalType, StringType, Type, TypeKind, TypeRef, UIntWidth,
        UnionType,
    },
    value::{Object, Value},
};
use semantic_db_core::catalog::{
    CatalogStorageSnapshot, LocalAttrId, LocalClassId, OBJECT_TYPE_FIELD, StoredAttribute,
    StoredClass,
};
use semantic_ui_core::form::{
    attribute_field_spec, class_form_field_label, default_value_for_class, set_object_field_value,
    set_optional_object_field_value, validate_value_against_type, validate_value_constraints,
};
use semantic_ui_core::{
    AttributeFormRenderContext, UiCatalog, ValueFormRenderContext, default_value_for_type,
};

fn run_in_runtime(f: impl FnOnce() + 'static) {
    #[derive(Clone)]
    struct TestProps {
        f: Rc<RefCell<Option<Box<dyn FnOnce()>>>>,
    }

    fn app(props: TestProps) -> Element {
        if let Some(f) = props.f.borrow_mut().take() {
            f();
        }
        rsx! {}
    }

    let f = Rc::new(RefCell::new(Some(Box::new(f) as Box<dyn FnOnce()>)));
    let mut dom = VirtualDom::new_with_props(app, TestProps { f });
    dom.rebuild_to_vec();
}

fn empty_snapshot() -> CatalogStorageSnapshot {
    CatalogStorageSnapshot {
        attributes: Vec::new(),
        type_defs: Vec::new(),
        record_types: Vec::new(),
        classes: Vec::new(),
        collections: Vec::new(),
        indexes: Vec::new(),
        relationships: Vec::new(),
        packages: Vec::new(),
        applied_migrations: Vec::new(),
        next_field_id: 0,
        auto_index_enabled: false,
    }
}

fn string_type() -> Type {
    Type::from(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn bool_type() -> Type {
    Type::from(TypeKind::Bool(BoolType))
}

fn uint64_type() -> Type {
    Type::from(TypeKind::Number(NumberType::UInt(UIntWidth::U64)))
}

fn attr(id: &str, name: &str, ty: Type) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty,
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn class_attr(id: &str, required: bool) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef { id: id.to_string() },
        required,
        ui_order: None,
        computed: None,
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn class(id: &str, name: &str, attributes: BTreeMap<String, ClassAttribute>) -> ClassType {
    ClassType {
        id: id.to_string(),
        name: name.to_string(),
        inherits: None,
        extends: Vec::new(),
        strict_schema: false,
        attributes,
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn subclass(id: &str, name: &str, parent: &str) -> ClassType {
    ClassType {
        id: id.to_string(),
        name: name.to_string(),
        inherits: Some(ClassRef {
            id: parent.to_string(),
        }),
        extends: Vec::new(),
        strict_schema: false,
        attributes: BTreeMap::new(),
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn catalog_with(attributes: Vec<AttributeType>, classes: Vec<ClassType>) -> UiCatalog {
    let mut snapshot = empty_snapshot();
    snapshot.attributes = attributes
        .into_iter()
        .enumerate()
        .map(|(index, attribute)| StoredAttribute {
            lid: LocalAttrId(index),
            attribute,
        })
        .collect();
    snapshot.classes = classes
        .into_iter()
        .enumerate()
        .map(|(index, class)| StoredClass {
            lid: LocalClassId(index),
            class,
        })
        .collect();
    UiCatalog::from_snapshot(snapshot)
}

#[test]
fn form_registry_lookup_precedence_is_explicit() {
    let catalog = catalog_with(Vec::new(), Vec::new());
    let mut registry = catalog.form_registry().clone();
    registry.register_type_form_renderer("string", Rc::new(|_: ValueFormRenderContext| rsx! {}));
    registry.register_attribute_form_renderer(
        "attr.name",
        Rc::new(|_: AttributeFormRenderContext| rsx! {}),
    );
    registry.register_class_field_form_renderer(
        "person",
        "name",
        Rc::new(|_: AttributeFormRenderContext| rsx! {}),
    );

    assert!(registry.type_form_renderer(&string_type().kind).is_some());
    assert!(registry.attribute_form_renderer("attr.name").is_some());
    assert!(
        registry
            .class_field_form_renderer("person", "name")
            .is_some()
    );
}

#[test]
fn ref_autocomplete_query_searches_entities_fields_and_limits() {
    let sql = semantic_ui_core::form::ref_autocomplete_query(
        "Ada",
        &["person".to_string(), "employee".to_string()],
        Some("person-1"),
    );

    assert!(sql.starts_with("SELECT * FROM entities WHERE"));
    assert!(sql.contains("\"id\" ILIKE '%Ada%'"));
    assert!(sql.contains("\"semantic:title\" ILIKE '%Ada%'"));
    assert!(sql.contains("\"semantic:base:person:display_name\" ILIKE '%Ada%'"));
    assert!(sql.contains("\"type\" IN ('person', 'employee')"));
    assert!(sql.contains("\"id\" <> 'person-1'"));
    assert!(sql.ends_with(" LIMIT 25"));
}

#[test]
fn ref_autocomplete_query_escapes_search_and_class_literals() {
    let sql = semantic_ui_core::form::ref_autocomplete_query(
        "O'Hara",
        &["local:person's".to_string()],
        Some("person'1"),
    );

    assert!(sql.contains("%O''Hara%"));
    assert!(sql.contains("'local:person''s'"));
    assert!(sql.contains("\"id\" <> 'person''1'"));
}

#[test]
fn ref_autocomplete_class_filter_includes_subclasses_from_ref_type() {
    let person = class("person", "Person", BTreeMap::new());
    let employee = subclass("employee", "Employee", "person");
    let catalog = catalog_with(Vec::new(), vec![person, employee]);
    let ty = Type::from(TypeKind::Ref(TypeRef::new("Person")));

    let class_ids = semantic_ui_core::form::ref_autocomplete_class_ids(&catalog, &ty);

    assert_eq!(
        class_ids,
        vec!["employee".to_string(), "person".to_string()]
    );
}

#[test]
fn ref_autocomplete_class_filter_collects_optional_union_refs() {
    let person = class("person", "Person", BTreeMap::new());
    let org = class("organization", "Organization", BTreeMap::new());
    let catalog = catalog_with(Vec::new(), vec![person, org]);
    let ty = Type::from(TypeKind::Optional(OptionalType {
        inner: Box::new(Type::from(TypeKind::Union(UnionType {
            variants: vec![
                Type::from(TypeKind::Ref(TypeRef::new("person"))),
                Type::from(TypeKind::Ref(TypeRef::new("Organization"))),
            ],
        }))),
    }));

    let class_ids = semantic_ui_core::form::ref_autocomplete_class_ids(&catalog, &ty);

    assert_eq!(
        class_ids,
        vec!["organization".to_string(), "person".to_string()]
    );
}

#[test]
fn default_class_value_sets_type_and_required_fields() {
    let name = attr("attr.name", "name", string_type());
    let active = attr("attr.active", "active", bool_type());
    let mut fields = BTreeMap::new();
    fields.insert("name".to_string(), class_attr("attr.name", true));
    fields.insert("active".to_string(), class_attr("attr.active", false));
    let class = class("person", "Person", fields);
    let catalog = catalog_with(vec![name, active], vec![class.clone()]);

    let Value::Object(object) = default_value_for_class(&class, &catalog) else {
        panic!("expected object default");
    };
    assert_eq!(
        object.get(OBJECT_TYPE_FIELD),
        Some(&Value::String("person".to_string()))
    );
    assert_eq!(object.get("attr.name"), Some(&Value::Null));
    assert_eq!(object.get("name"), None);
    assert_eq!(object.get("active"), None);
}

#[test]
fn type_defaults_are_semantic_values() {
    assert_eq!(
        default_value_for_type(&string_type()),
        Value::String(String::new())
    );
    assert_eq!(default_value_for_type(&bool_type()), Value::Bool(false));
}

#[test]
fn object_field_helpers_create_and_remove_fields() {
    let mut value = Value::Null;
    set_object_field_value(&mut value, "name", Value::String("Ada".to_string()));
    assert!(matches!(&value, Value::Object(object) if object.get("name").is_some()));

    set_optional_object_field_value(&mut value, "name", Value::Null);
    assert!(matches!(&value, Value::Object(object) if object.get("name").is_none()));
}

#[test]
fn validation_reports_required_string_shape_constraints() {
    let errors = validate_value_constraints(
        dxform::FieldPath::new("name"),
        &Value::String("ab".to_string()),
        &[
            Constraint::Length(LengthSpec::Range {
                min: Some(3),
                max: Some(5),
            }),
            Constraint::Prefix("A".to_string()),
        ],
    );
    assert_eq!(errors.len(), 2);
    assert!(
        errors
            .iter()
            .any(|err| err.code.as_deref() == Some("length"))
    );
    assert!(
        errors
            .iter()
            .any(|err| err.code.as_deref() == Some("prefix"))
    );
}

#[test]
fn null_value_passes_number_type_validation() {
    assert!(
        validate_value_against_type(
            dxform::FieldPath::new("byte_size"),
            &Value::Null,
            &uint64_type()
        )
        .is_empty()
    );
}

#[test]
fn attribute_field_spec_updates_root_object_and_meta() {
    run_in_runtime(|| {
        let attribute = attr("attr.name", "name", string_type());
        let class_attribute = class_attr("attr.name", true);
        let catalog = catalog_with(vec![attribute.clone()], Vec::new());
        let form = FormRoot::new(Value::Object(Object::new()));
        let scope = form.scope();
        let field = scope.field(attribute_field_spec(
            "name".to_string(),
            attribute,
            class_attribute,
            catalog,
        ));

        field.set_value(Value::String("Ada".to_string()));

        let Value::Object(object) = form.values() else {
            panic!("expected object value");
        };
        assert_eq!(object.get("name"), Some(&Value::String("Ada".to_string())));
        assert!(form.meta().dirty);
    });
}

#[test]
fn optional_number_attribute_uses_null_fallback_and_removes_null_values() {
    run_in_runtime(|| {
        let attribute = attr("attr.byte_size", "byte_size", uint64_type());
        let class_attribute = class_attr("attr.byte_size", false);
        let catalog = catalog_with(vec![attribute.clone()], Vec::new());
        let form = FormRoot::new(Value::Object(Object::new()));
        let scope = form.scope();
        let field = scope.field(attribute_field_spec(
            "byte_size".to_string(),
            attribute,
            class_attribute,
            catalog,
        ));

        assert_eq!(field.value(), Value::Null);

        field.set_value(Value::U64(42));
        let Value::Object(object) = form.values() else {
            panic!("expected object value");
        };
        assert_eq!(object.get("byte_size"), Some(&Value::U64(42)));

        field.set_value(Value::Null);
        let Value::Object(object) = form.values() else {
            panic!("expected object value");
        };
        assert_eq!(object.get("byte_size"), None);
    });
}

#[test]
fn class_attribute_field_spec_reads_and_writes_canonical_storage_field() {
    run_in_runtime(|| {
        let attribute = attr("semantic:description", "description", string_type());
        let class_attribute = class_attr("semantic:description", false);
        let catalog = catalog_with(vec![attribute.clone()], Vec::new());
        let mut initial = Object::new();
        initial.insert("semantic:description", Value::String("old".to_string()));
        let form = FormRoot::new(Value::Object(initial));
        let scope = form.scope();
        let field = scope.field(
            semantic_ui_core::form::attribute_field_spec_with_storage_name(
                "description".to_string(),
                "semantic:description".to_string(),
                attribute,
                class_attribute,
                catalog,
            ),
        );

        assert_eq!(field.value(), Value::String("old".to_string()));
        field.set_value(Value::String("new".to_string()));

        let Value::Object(object) = form.values() else {
            panic!("expected object value");
        };
        assert_eq!(
            object.get("semantic:description"),
            Some(&Value::String("new".to_string()))
        );
        assert_eq!(object.get("description"), None);
    });
}

#[test]
fn semantic_leaf_field_submit_receives_edited_root_object() {
    run_in_runtime(|| {
        let submitted = Rc::new(RefCell::new(None));
        let submitted_for_handler = submitted.clone();
        let form =
            FormRoot::with_options(FormOptions::new(Value::Object(Object::new())).on_submit(
                SubmitHandler::sync(move |ctx| {
                    *submitted_for_handler.borrow_mut() = Some(ctx.values);
                    Ok(())
                }),
            ));
        let object_scope = form.scope();
        let name_scope = object_scope.subform(SubformSpec {
            name: "name".to_string(),
            get: Rc::new(|parent: &Value| match parent {
                Value::Object(object) => object.get("name").cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            }),
            set: Rc::new(|parent: &mut Value, value| {
                set_object_field_value(parent, "name", value);
            }),
            is_empty: Rc::new(semantic_ui_core::form::is_empty_value),
            validators: Vec::new(),
            validation: dxform::ValidationStrategy::submit(),
        });
        let leaf = name_scope.field(FieldSpec {
            name: "value".to_string(),
            get: Rc::new(|value: &Value| value.clone()),
            set: Rc::new(|parent: &mut Value, value| *parent = value),
            format: Rc::new(|value: &Value| value.clone()),
            parse: Rc::new(|value: &Value| Ok(value.clone())),
            is_empty: Rc::new(semantic_ui_core::form::is_empty_value),
            validators: Vec::new(),
            validation: dxform::ValidationStrategy::submit(),
        });

        leaf.set_value(Value::String("Ada".to_string()));
        futures::executor::block_on(form.submit()).expect("submit should succeed");

        let Some(Value::Object(object)) = submitted.borrow().clone() else {
            panic!("expected submitted object");
        };
        assert_eq!(object.get("name"), Some(&Value::String("Ada".to_string())));
    });
}

#[test]
fn semantic_leaf_field_replaces_explicit_null_on_submit() {
    run_in_runtime(|| {
        let submitted = Rc::new(RefCell::new(None));
        let submitted_for_handler = submitted.clone();
        let mut initial = Object::new();
        initial.insert("description", Value::Null);
        let form = FormRoot::with_options(FormOptions::new(Value::Object(initial)).on_submit(
            SubmitHandler::sync(move |ctx| {
                *submitted_for_handler.borrow_mut() = Some(ctx.values);
                Ok(())
            }),
        ));
        let object_scope = form.scope();
        let description_scope = object_scope.subform(SubformSpec {
            name: "description".to_string(),
            get: Rc::new(|parent: &Value| match parent {
                Value::Object(object) => object.get("description").cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            }),
            set: Rc::new(|parent: &mut Value, value| {
                set_object_field_value(parent, "description", value);
            }),
            is_empty: Rc::new(semantic_ui_core::form::is_empty_value),
            validators: Vec::new(),
            validation: dxform::ValidationStrategy::submit(),
        });
        let leaf = description_scope.field(FieldSpec {
            name: "value".to_string(),
            get: Rc::new(|value: &Value| value.clone()),
            set: Rc::new(|parent: &mut Value, value| *parent = value),
            format: Rc::new(|value: &Value| value.clone()),
            parse: Rc::new(|value: &Value| Ok(value.clone())),
            is_empty: Rc::new(semantic_ui_core::form::is_empty_value),
            validators: Vec::new(),
            validation: dxform::ValidationStrategy::submit(),
        });

        leaf.set_value(Value::String("updated".to_string()));
        futures::executor::block_on(form.submit()).expect("submit should succeed");

        let Some(Value::Object(object)) = submitted.borrow().clone() else {
            panic!("expected submitted object");
        };
        assert_eq!(
            object.get("description"),
            Some(&Value::String("updated".to_string()))
        );
    });
}

#[test]
fn semantic_submit_preserves_extra_object_fields_while_editing_known_field() {
    run_in_runtime(|| {
        let submitted = Rc::new(RefCell::new(None));
        let submitted_for_handler = submitted.clone();
        let attribute = attr("semantic:description", "description", string_type());
        let class_attribute = class_attr("semantic:description", false);
        let catalog = catalog_with(vec![attribute.clone()], Vec::new());
        let mut initial = Object::new();
        initial.insert("semantic:description", Value::String("old".to_string()));
        initial.insert("external:note", Value::String("keep me".to_string()));
        let form = FormRoot::with_options(FormOptions::new(Value::Object(initial)).on_submit(
            SubmitHandler::sync(move |ctx| {
                *submitted_for_handler.borrow_mut() = Some(ctx.values);
                Ok(())
            }),
        ));
        let field = form.scope().field(
            semantic_ui_core::form::attribute_field_spec_with_storage_name(
                "description".to_string(),
                "semantic:description".to_string(),
                attribute,
                class_attribute,
                catalog,
            ),
        );

        field.set_value(Value::String("new".to_string()));
        futures::executor::block_on(form.submit()).expect("submit should succeed");

        let Some(Value::Object(object)) = submitted.borrow().clone() else {
            panic!("expected submitted object");
        };
        assert_eq!(
            object.get("semantic:description"),
            Some(&Value::String("new".to_string()))
        );
        assert_eq!(
            object.get("external:note"),
            Some(&Value::String("keep me".to_string()))
        );
    });
}

#[test]
fn inherited_class_form_fields_are_collected_before_child_overrides() {
    let title = attr("attr.title", "title", string_type());
    let name = attr("attr.name", "name", string_type());
    let mut base_fields = BTreeMap::new();
    base_fields.insert("title".to_string(), class_attr("attr.title", true));
    let base = class("base", "Base", base_fields);
    let mut child_fields = BTreeMap::new();
    child_fields.insert("name".to_string(), class_attr("attr.name", true));
    let mut child = class("child", "Child", child_fields);
    child.inherits = Some(ClassRef {
        id: "base".to_string(),
    });
    let catalog = catalog_with(vec![title, name], vec![base, child.clone()]);

    let fields = catalog.class_form_fields(&child);
    let names = fields
        .iter()
        .map(|field| field.field_name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["title".to_string(), "name".to_string()]);
    let storage_names = fields
        .into_iter()
        .map(|field| field.storage_field_name)
        .collect::<Vec<_>>();
    assert_eq!(
        storage_names,
        vec!["attr.title".to_string(), "attr.name".to_string()]
    );
}

#[test]
fn class_form_fields_respect_class_attribute_ui_order() {
    let first = attr("attr.first", "first", string_type());
    let second = attr("attr.second", "second", string_type());
    let third = attr("attr.third", "third", string_type());
    let mut fields = BTreeMap::new();
    let mut second_attr = class_attr("attr.second", false);
    second_attr.ui_order = Some(1);
    let mut first_attr = class_attr("attr.first", false);
    first_attr.ui_order = Some(2);
    fields.insert("first".to_string(), first_attr);
    fields.insert("second".to_string(), second_attr);
    fields.insert("third".to_string(), class_attr("attr.third", false));
    let class = class("ordered", "Ordered", fields);
    let catalog = catalog_with(vec![first, second, third], vec![class.clone()]);

    let names = catalog
        .class_form_fields(&class)
        .into_iter()
        .map(|field| field.field_name)
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "second".to_string(),
            "first".to_string(),
            "third".to_string()
        ]
    );
}

#[test]
fn class_form_field_label_uses_attribute_id_as_title_for_custom_titles() {
    let mut attribute = attr("semantic:description", "description", string_type());
    attribute.meta.title = Some("Description".to_string());
    let mut fields = BTreeMap::new();
    fields.insert(
        "description".to_string(),
        class_attr("semantic:description", false),
    );
    let class = class("document", "Document", fields);
    let catalog = catalog_with(vec![attribute], vec![class.clone()]);

    let field = catalog
        .class_form_fields(&class)
        .into_iter()
        .next()
        .expect("field should exist");
    let label = class_form_field_label(&field);

    assert_eq!(label.text, "Description");
    assert_eq!(label.title.as_deref(), Some("semantic:description"));
}

#[test]
fn class_form_field_label_omits_title_without_custom_title() {
    let attribute = attr("semantic:description", "description", string_type());
    let mut fields = BTreeMap::new();
    fields.insert(
        "description".to_string(),
        class_attr("semantic:description", false),
    );
    let class = class("document", "Document", fields);
    let catalog = catalog_with(vec![attribute], vec![class.clone()]);

    let field = catalog
        .class_form_fields(&class)
        .into_iter()
        .next()
        .expect("field should exist");
    let label = class_form_field_label(&field);

    assert_eq!(label.text, "description");
    assert_eq!(label.title, None);
}
