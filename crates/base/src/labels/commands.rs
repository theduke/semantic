use super::{
    model::{optional_string, required_string},
    *,
};
use semantic_data::{
    builtin::DEFAULT_COLLECTION,
    schema::{FunctionParam, FunctionType, Type, TypeKind},
    value::{Object, Value},
};
use semantic_rpc_core::{CommandAdapter, DynCommand, RpcCommand, RpcCommandSpec, RpcError};
use std::{future::Future, pin::Pin};

/// Resolves the authorized database for each request, honoring the host's scope rules.
pub trait LabelContext: Sync + 'static {
    type Store: LabelStore;
    fn label_store(
        &self,
        scope_id: Option<String>,
    ) -> impl Future<Output = Result<Self::Store, RpcError>> + Send;
}

macro_rules! command {
    ($type:ident, $name:literal, $op:ident) => {
        pub struct $type;
        impl RpcCommandSpec for $type {
            type Payload = Value;
            type Output = Value;
            type Error = RpcError;
            const NAME: &'static str = $name;
            fn signature(&self) -> FunctionType {
                FunctionType {
                    params: vec![FunctionParam {
                        name: Some("payload".into()),
                        ty: Type::new(TypeKind::Any(semantic_data::schema::AnyType)),
                    }],
                    results: vec![Type::new(TypeKind::Any(semantic_data::schema::AnyType))],
                    throws: None,
                    async_fn: true,
                }
            }
        }
        impl<Ctx: LabelContext> RpcCommand<Ctx> for $type {
            fn call<'a>(
                &'a self,
                ctx: &'a Ctx,
                payload: Value,
            ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
                Box::pin(execute(ctx, payload, Operation::$op))
            }
        }
    };
}

#[derive(Clone, Copy)]
enum Operation {
    List,
    Load,
    Add,
    Remove,
    Replace,
    Save,
    Delete,
}
command!(ListLabels, "semantic.base.labels.list", List);
command!(LoadEntityLabels, "semantic.base.labels.load", Load);
command!(AddEntityLabels, "semantic.base.labels.add", Add);
command!(RemoveEntityLabels, "semantic.base.labels.remove", Remove);
command!(ReplaceEntityLabels, "semantic.base.labels.replace", Replace);
command!(SaveLabel, "semantic.base.labels.save", Save);
command!(DeleteLabel, "semantic.base.labels.delete", Delete);

pub fn commands<Ctx: LabelContext>() -> Vec<Box<dyn DynCommand<Ctx>>> {
    vec![
        Box::new(CommandAdapter::new(ListLabels)),
        Box::new(CommandAdapter::new(LoadEntityLabels)),
        Box::new(CommandAdapter::new(AddEntityLabels)),
        Box::new(CommandAdapter::new(RemoveEntityLabels)),
        Box::new(CommandAdapter::new(ReplaceEntityLabels)),
        Box::new(CommandAdapter::new(SaveLabel)),
        Box::new(CommandAdapter::new(DeleteLabel)),
    ]
}

async fn execute(
    ctx: &impl LabelContext,
    payload: Value,
    op: Operation,
) -> Result<Value, RpcError> {
    let Value::Object(object) = payload else {
        return Err(RpcError::invalid_payload("Expected an object"));
    };
    let store = ctx
        .label_store(optional_string(&object, "scope_id")?)
        .await?;
    match op {
        Operation::List => Ok(encode_labels(list_labels(&store).await?)),
        Operation::Save => {
            let Some(Value::Object(label)) = object.get("label") else {
                return Err(RpcError::invalid_payload("label is required"));
            };
            Ok(Value::Object(
                save_label(&store, Label::from_object(label)?)
                    .await?
                    .to_object(),
            ))
        }
        Operation::Delete => {
            delete_label(&store, &required_string(&object, "id")?).await?;
            Ok(Value::Null)
        }
        _ => {
            let collection = optional_string(&object, "collection")?
                .unwrap_or_else(|| DEFAULT_COLLECTION.into());
            let id = required_string(&object, "id")?;
            let result = match op {
                Operation::Load => labels_for_entity(&store, &collection, &id).await?,
                _ => {
                    let ids = label_ids(&object)?;
                    match op {
                        Operation::Add => add_labels(&store, &collection, &id, &ids).await?,
                        Operation::Remove => remove_labels(&store, &collection, &id, &ids).await?,
                        Operation::Replace => {
                            replace_labels(&store, &collection, &id, &ids).await?
                        }
                        _ => unreachable!(),
                    }
                }
            };
            Ok(encode_labels(result))
        }
    }
}

fn label_ids(object: &Object) -> Result<Vec<String>, RpcError> {
    let Some(Value::List(ids)) = object.get("label_ids") else {
        return Err(RpcError::invalid_payload("label_ids must be a list"));
    };
    ids.iter()
        .map(|id| match id {
            Value::String(id) if !id.trim().is_empty() => Ok(id.clone()),
            _ => Err(RpcError::invalid_payload(
                "label_ids must contain nonempty strings",
            )),
        })
        .collect()
}

pub fn encode_labels(labels: Vec<Label>) -> Value {
    Value::List(
        labels
            .into_iter()
            .map(|label| Value::Object(label.to_object()))
            .collect(),
    )
}

pub fn decode_labels(value: Value) -> Result<Vec<Label>, RpcError> {
    let Value::List(labels) = value else {
        return Err(RpcError::invalid_output("Expected a label list"));
    };
    labels
        .into_iter()
        .map(|label| match label {
            Value::Object(object) => Label::from_object(&object),
            _ => Err(RpcError::invalid_output("Expected a label object")),
        })
        .collect()
}
