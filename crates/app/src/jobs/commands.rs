use crate::{AppError, AppRequestContext, DbScopeId};
use semantic_data::{Object, Value, jobs::*, schema::FunctionType};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{RpcCommand, RpcCommandSpec};
use std::{future::Future, pin::Pin};

pub(crate) fn register(registry: &mut RpcRegistry<AppRequestContext>) -> Result<(), AppError> {
    registry.register(List)?;
    registry.register(Get)?;
    registry.register(Cancel)?;
    registry.register(Clear)?;
    registry.register(Kinds)?;
    Ok(())
}
macro_rules! command {
    ($type:ident, $name:literal, $ctx:ident, $payload:ident, $body:block) => {
        struct $type;
        impl RpcCommandSpec for $type {
            type Payload = Value;
            type Output = Value;
            type Error = AppError;
            const NAME: &'static str = $name;
            fn signature(&self) -> FunctionType {
                command_signature($name).expect("registered jobs command")
            }
        }
        impl RpcCommand<AppRequestContext> for $type {
            fn call<'a>(
                &'a self,
                $ctx: &'a AppRequestContext,
                value: Value,
            ) -> Pin<Box<dyn Future<Output = Result<Value, AppError>> + Send + 'a>> {
                Box::pin(async move {
                    let $payload = match value {
                        Value::Object(v) => v,
                        Value::Void | Value::Null => Object::new(),
                        _ => return Err(super::error("expected object")),
                    };
                    $body
                })
            }
        }
    };
}
fn scope(object: &Object) -> Result<Option<DbScopeId>, AppError> {
    match object.get("scope_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.clone().into())),
        _ => Err(super::error("invalid scope_id")),
    }
}
fn id(object: &Object) -> Result<JobId, AppError> {
    object
        .get("id")
        .and_then(Value::as_str)
        .map(|v| JobId(v.into()))
        .ok_or_else(|| super::error("job id required"))
}
command!(List, "semantic.jobs.list", ctx, payload, {
    let query = JobListQuery::from_object(&payload).map_err(super::error)?;
    Ok(ctx
        .jobs(scope(&payload)?)
        .await?
        .list(query)
        .await?
        .to_value())
});
command!(Get, "semantic.jobs.get", ctx, payload, {
    Ok(ctx
        .jobs(scope(&payload)?)
        .await?
        .get(id(&payload)?)
        .await?
        .map(|r| Value::Object(r.to_rpc_object()))
        .unwrap_or(Value::Null))
});
command!(Cancel, "semantic.jobs.cancel", ctx, payload, {
    Ok(Value::Object(
        ctx.jobs(scope(&payload)?)
            .await?
            .cancel(id(&payload)?)
            .await?
            .to_rpc_object(),
    ))
});
command!(Clear, "semantic.jobs.clear_completed", ctx, payload, {
    let result = ctx.jobs(scope(&payload)?).await?.clear_completed().await?;
    let mut object = Object::new();
    object.insert("deleted", result.deleted);
    Ok(Value::Object(object))
});
command!(Kinds, "semantic.jobs.kinds", ctx, payload, {
    let kinds = ctx.jobs(scope(&payload)?).await?.kinds();
    Ok(Value::List(
        kinds
            .into_iter()
            .map(|kind| {
                let mut object = Object::new();
                object.insert("id", kind.id.0);
                object.insert("title", kind.title);
                object.insert(
                    "description",
                    kind.description.map(Value::String).unwrap_or(Value::Null),
                );
                Value::Object(object)
            })
            .collect(),
    ))
});
