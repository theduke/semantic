use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use dioxus::prelude::*;
use dxform::{FieldSpec, FormOptions, FormRoot, SubformSpec, SubmitHandler};
use semantic_data::{
    schema::{
        AttributeRef, AttributeType, BoolType, ClassAttribute, ClassRef, ClassType, Constraint,
        LengthSpec, LiteralValue, Meta, StringType, Type, TypeKind,
    },
    value::{Object, Value},
};
use semantic_db_core::catalog::{
    CatalogStorageSnapshot, LocalAttrId, LocalClassId, OBJECT_TYPE_FIELD, StoredAttribute,
    StoredClass,
};
use semantic_ui_core::form::{
    attribute_field_spec, default_value_for_class, set_object_field_value,
    set_optional_object_field_value, validate_value_constraints,
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
        attributes,
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
fn literal_and_type_defaults_are_semantic_values() {
    assert_eq!(
        semantic_ui_core::literal_to_value(&LiteralValue::String("hello".to_string())),
        Value::String("hello".to_string())
    );
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
