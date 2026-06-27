use std::rc::Rc;

use dxform::{FormOptions, SubmitContext, SubmitError, SubmitHandler, ValidationStrategy};
use futures::future::{FutureExt, LocalBoxFuture};
use semantic_data::{
    builtin::ID_ATTRIBUTE_ID,
    schema::{ClassType, Type},
    value::{Object, Value},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticFormMode {
    Create,
    Edit,
}

#[derive(Clone)]
pub struct SemanticSubmitContext {
    pub mode: SemanticFormMode,
    pub value: Value,
    pub class: Option<ClassType>,
    pub collection: Option<String>,
    pub id: Option<String>,
    pub scope_id: Option<String>,
}

#[derive(Clone)]
pub struct SemanticFormSubmit(
    Rc<
        dyn Fn(
            SemanticSubmitContext,
        ) -> LocalBoxFuture<'static, std::result::Result<(), SubmitError>>,
    >,
);

impl PartialEq for SemanticFormSubmit {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl SemanticFormSubmit {
    pub fn sync(
        f: impl Fn(SemanticSubmitContext) -> std::result::Result<(), SubmitError> + 'static,
    ) -> Self {
        Self(Rc::new(move |ctx| {
            futures::future::ready(f(ctx)).boxed_local()
        }))
    }

    pub fn async_(
        f: impl Fn(
            SemanticSubmitContext,
        ) -> LocalBoxFuture<'static, std::result::Result<(), SubmitError>>
        + 'static,
    ) -> Self {
        Self(Rc::new(f))
    }

    pub fn to_dx_submit_handler(
        &self,
        mode: SemanticFormMode,
        class: Option<ClassType>,
        collection: Option<String>,
        id: Option<String>,
        scope_id: Option<String>,
    ) -> SubmitHandler<Value> {
        let submit = self.clone();
        SubmitHandler::async_(move |ctx: SubmitContext<Value>| {
            let submit = submit.clone();
            let class = class.clone();
            let collection = collection.clone();
            let id = id.clone();
            let scope_id = scope_id.clone();
            async move {
                (submit.0)(SemanticSubmitContext {
                    mode,
                    value: ctx.values,
                    class,
                    collection,
                    id,
                    scope_id,
                })
                .await
            }
            .boxed_local()
        })
    }
}

#[derive(Clone, PartialEq)]
pub struct SemanticFormOptions {
    pub mode: SemanticFormMode,
    pub initial_value: Value,
    pub type_hint: Option<Type>,
    pub class: Option<ClassType>,
    pub collection: Option<String>,
    pub id: Option<String>,
    pub scope_id: Option<String>,
    pub submit: Option<SemanticFormSubmit>,
    pub validation: ValidationStrategy,
    pub show_actions: bool,
}

impl SemanticFormOptions {
    pub fn new(initial_value: Value) -> Self {
        Self {
            mode: SemanticFormMode::Edit,
            initial_value,
            type_hint: None,
            class: None,
            collection: None,
            id: None,
            scope_id: None,
            submit: None,
            validation: ValidationStrategy::submit(),
            show_actions: true,
        }
    }
}

pub fn build_value_form_options(options: &SemanticFormOptions) -> FormOptions<Value> {
    let mut out = FormOptions::new(options.initial_value.clone()).validation(options.validation);
    if let Some(submit) = &options.submit {
        out = out.on_submit(submit.to_dx_submit_handler(
            options.mode,
            options.class.clone(),
            options.collection.clone(),
            options.id.clone(),
            options.scope_id.clone(),
        ));
    }
    out
}

pub fn build_class_form_options(
    class: ClassType,
    object: Object,
    mode: SemanticFormMode,
    collection: Option<String>,
    id: Option<String>,
    scope_id: Option<String>,
    submit: Option<SemanticFormSubmit>,
) -> SemanticFormOptions {
    let mut value = Value::Object(object);
    if let Value::Object(object) = &mut value {
        object.insert(
            semantic_db_core::catalog::OBJECT_TYPE_FIELD,
            Value::String(class.id.clone()),
        );
    }
    SemanticFormOptions {
        mode,
        initial_value: value,
        type_hint: None,
        class: Some(class),
        collection,
        id,
        scope_id,
        submit,
        validation: ValidationStrategy::submit(),
        show_actions: true,
    }
}

pub fn rpc_insert_submit_handler(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: String,
    id: String,
) -> SemanticFormSubmit {
    rpc_insert_submit_handler_with_primary_id(
        client,
        scope_id,
        collection,
        id,
        ID_ATTRIBUTE_ID.to_string(),
    )
}

pub fn rpc_insert_submit_handler_with_primary_id(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: String,
    id: String,
    primary_id_field: String,
) -> SemanticFormSubmit {
    rpc_batch_upsert_submit_handler_with_primary_id(
        client,
        scope_id,
        collection,
        id,
        primary_id_field,
    )
}

pub fn rpc_batch_upsert_submit_handler(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: String,
    id: String,
) -> SemanticFormSubmit {
    rpc_batch_upsert_submit_handler_with_primary_id(
        client,
        scope_id,
        collection,
        id,
        ID_ATTRIBUTE_ID.to_string(),
    )
}

pub fn rpc_batch_upsert_submit_handler_with_primary_id(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    collection: String,
    id: String,
    primary_id_field: String,
) -> SemanticFormSubmit {
    SemanticFormSubmit::async_(move |ctx| {
        let client = client.clone();
        let scope_id = scope_id.clone();
        let collection = collection.clone();
        let id = id.clone();
        let primary_id_field = primary_id_field.clone();
        async move {
            let Value::Object(mut object) = ctx.value else {
                return Err(SubmitError::message("submitted value must be an object"));
            };
            inject_primary_id(&mut object, &primary_id_field, &id)?;
            let mut payload = Object::new();
            if let Some(scope_id) = scope_id {
                payload.insert("scope_id", Value::String(scope_id));
            }
            payload.insert(
                "operations",
                Value::List(vec![Value::Object(batch_upsert_operation(
                    collection, id, object,
                ))]),
            );
            client
                .invoke_value("semantic.db.batch", Value::Object(payload))
                .await
                .map(|_| ())
                .map_err(|err| SubmitError::message(err.to_string()))
        }
        .boxed_local()
    })
}

fn batch_upsert_operation(collection: String, id: String, object: Object) -> Object {
    let mut operation = Object::new();
    operation.insert("kind", Value::String("upsert".to_string()));
    operation.insert("collection", Value::String(collection));
    operation.insert(ID_ATTRIBUTE_ID, Value::String(id));
    operation.insert("object", Value::Object(object));
    operation
}

fn inject_primary_id(
    object: &mut Object,
    primary_id_field: &str,
    id: &str,
) -> std::result::Result<(), SubmitError> {
    match object.get(primary_id_field) {
        Some(Value::String(existing)) if existing == id => Ok(()),
        Some(Value::String(existing)) => Err(SubmitError::message(format!(
            "primary key field '{primary_id_field}' value '{existing}' does not match entity id '{id}'"
        ))),
        Some(_) => Err(SubmitError::message(format!(
            "primary key field '{primary_id_field}' must be a string"
        ))),
        None => {
            object.insert(primary_id_field.to_string(), Value::String(id.to_string()));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::builtin::ID_ATTRIBUTE_ID;
    use semantic_data::value::{Object, Value};

    use super::inject_primary_id;

    #[test]
    fn inject_primary_id_adds_canonical_primary_key_field() {
        let mut object = Object::new();

        inject_primary_id(&mut object, ID_ATTRIBUTE_ID, "entity-1").unwrap();

        assert_eq!(
            object.get(ID_ATTRIBUTE_ID),
            Some(&Value::String("entity-1".to_string()))
        );
    }

    #[test]
    fn inject_primary_id_rejects_conflicting_primary_key_field() {
        let mut object = Object::new();
        object.insert(ID_ATTRIBUTE_ID, Value::String("other-entity".to_string()));

        let err = inject_primary_id(&mut object, ID_ATTRIBUTE_ID, "entity-1").unwrap_err();

        assert!(
            err.message
                .as_deref()
                .unwrap_or_default()
                .contains("does not match entity id 'entity-1'")
        );
    }
}
