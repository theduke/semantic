use std::rc::Rc;

use dxform::{FieldSpec, FieldValidator, ValidationStrategy};
use semantic_data::{
    schema::{AttributeType, ClassAttribute},
    value::Value,
};

use crate::{
    UiCatalog,
    form::{
        is_empty_value, object_field_value, set_object_field_value,
        set_optional_object_field_value, validators_for_attribute,
    },
};

pub fn value_field_spec(
    field_name: String,
    fallback: Value,
    is_empty: Rc<dyn Fn(&Value) -> bool>,
    validators: Vec<FieldValidator<Value, Value>>,
    validation: ValidationStrategy,
) -> FieldSpec<Value, Value> {
    let get_field = field_name.clone();
    let set_field = field_name.clone();
    let mut spec = FieldSpec {
        name: field_name,
        get: Rc::new(move |parent: &Value| {
            object_field_value(parent, &get_field, fallback.clone())
        }),
        set: Rc::new(move |parent: &mut Value, value: Value| {
            set_object_field_value(parent, &set_field, value);
        }),
        is_empty,
        validators,
        validation,
    };
    spec.validation = validation;
    spec
}

pub fn attribute_field_spec(
    field_name: String,
    attribute: AttributeType,
    class_attribute: ClassAttribute,
    catalog: UiCatalog,
) -> FieldSpec<Value, Value> {
    attribute_field_spec_with_storage_name(
        field_name.clone(),
        field_name,
        attribute,
        class_attribute,
        catalog,
    )
}

pub fn attribute_field_spec_with_storage_name(
    field_name: String,
    storage_field_name: String,
    attribute: AttributeType,
    class_attribute: ClassAttribute,
    _catalog: UiCatalog,
) -> FieldSpec<Value, Value> {
    let fallback = crate::form::default_value_for_type(&attribute.ty);
    let get_field = storage_field_name.clone();
    let set_field = storage_field_name;
    FieldSpec {
        name: field_name,
        get: Rc::new(move |parent: &Value| {
            object_field_value(parent, &get_field, fallback.clone())
        }),
        set: Rc::new(move |parent: &mut Value, value: Value| {
            if class_attribute.required {
                set_object_field_value(parent, &set_field, value);
            } else {
                set_optional_object_field_value(parent, &set_field, value);
            }
        }),
        is_empty: Rc::new(is_empty_value),
        validators: validators_for_attribute(&attribute, &class_attribute),
        validation: ValidationStrategy::submit(),
    }
}
