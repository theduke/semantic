//! Unary discovery, submission and plugin administration. Content uses interface streams.
use crate::{AppError, AppRequestContext, DbScopeId};
use semantic_data::{
    Object, Value,
    import::{Operation, ProbeResult, SourceRequest},
    plugin::PluginActivation,
    schema::*,
};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{RpcCommand, RpcCommandSpec};
use std::{future::Future, pin::Pin};

pub(crate) fn register(registry: &mut RpcRegistry<AppRequestContext>) -> Result<(), AppError> {
    registry.register(Candidates)?;
    registry.register(StartSource)?;
    registry.register(Plugins)?;
    registry.register(Configure)?;
    registry.register(Uninstall)?;
    Ok(())
}
fn error(message: impl std::fmt::Display) -> AppError {
    crate::plugins::error(message)
}
fn scope(payload: &Object) -> Result<Option<DbScopeId>, AppError> {
    match payload.get("scope_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => Ok(Some(id.clone().into())),
        _ => Err(error("invalid scope_id")),
    }
}
fn optional_string<'a>(payload: &'a Object, key: &str) -> Result<Option<&'a str>, AppError> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        _ => Err(error(format!("invalid {key}"))),
    }
}
fn explicit(payload: &Object) -> Result<Option<(&str, &str)>, AppError> {
    match (
        optional_string(payload, "plugin_id")?,
        optional_string(payload, "export")?,
    ) {
        (None, None) => Ok(None),
        (Some(plugin), Some(export)) => Ok(Some((plugin, export))),
        _ => Err(error("plugin_id and export must be supplied together")),
    }
}

fn signature(name: &str) -> FunctionType {
    let method = match name {
        "semantic.import.candidates" => Some("list_candidates"),
        "semantic.import.start_source" => Some("start_import_source"),
        _ => None,
    };
    if let Some(method) = method {
        return semantic_data::import::package().root.interfaces["Application"]
            .methods
            .iter()
            .find(|m| m.name == method)
            .expect("canonical import method")
            .signature
            .clone();
    }
    let object = Type::new(TypeKind::Record(RecordType {
        fields: Default::default(),
        open: true,
        additional: None,
        required_order: None,
    }));
    let result = if name == "semantic.plugin.list" {
        Type::new(TypeKind::List(ListType {
            items: Box::new(object.clone()),
        }))
    } else {
        Type::new(TypeKind::Null(NullType))
    };
    FunctionType {
        params: vec![FunctionParam {
            name: Some("request".into()),
            ty: object.clone(),
        }],
        results: vec![result],
        throws: Some(Box::new(object)),
        async_fn: true,
    }
}
macro_rules! command {
    ($type:ident,$name:literal,$ctx:ident,$payload:ident,$body:block) => {
        struct $type;
        impl RpcCommandSpec for $type {
            type Payload = Value;
            type Output = Value;
            type Error = AppError;
            const NAME: &'static str = $name;
            fn signature(&self) -> FunctionType {
                signature($name)
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
                        Value::Object(o) => o,
                        Value::Null | Value::Void => Object::new(),
                        _ => return Err(error("expected request object")),
                    };
                    $body
                })
            }
        }
    };
}

pub(crate) fn candidate_value(candidate: semantic_import::SourceCandidate) -> Value {
    let mut object = Object::new();
    object.insert("plugin_id", candidate.operation.plugin_id().to_owned());
    object.insert("export", candidate.operation.descriptor().export.clone());
    object.insert(
        "source_export",
        candidate.source.descriptor().export.clone(),
    );
    object.insert("generation", candidate.operation.generation());
    object.insert("priority", Value::I32(candidate.priority));
    object.insert("descriptor", candidate.descriptor.to_value());
    let (status, reason) = match candidate.probe {
        ProbeResult::Supported => ("supported", None),
        ProbeResult::Unsupported(reason) => ("unsupported", Some(reason)),
        ProbeResult::Unavailable(reason) => ("unavailable", Some(reason)),
    };
    object.insert("status", status.to_owned());
    object.insert("reason", reason.map(Value::String).unwrap_or(Value::Null));
    Value::Object(object)
}
command!(Candidates, "semantic.import.candidates", ctx, payload, {
    let request = SourceRequest::from_value(Value::Object(payload.clone())).map_err(error)?;
    let operation =
        Operation::parse(optional_string(&payload, "operation")?.unwrap_or("import_source"))
            .map_err(error)?;
    let candidates = ctx
        .import_candidates(scope(&payload)?, &request, operation)
        .await?;
    Ok(Value::List(
        candidates.into_iter().map(candidate_value).collect(),
    ))
});
command!(StartSource, "semantic.import.start_source", ctx, payload, {
    let request = SourceRequest::from_value(Value::Object(payload.clone())).map_err(error)?;
    let choice = explicit(&payload)?;
    let generation = match payload.get("generation") {
        None | Some(Value::Null) => None,
        Some(Value::U64(value)) => Some(*value),
        _ => return Err(error("invalid generation")),
    };
    if generation.is_some() && choice.is_none() {
        return Err(error("generation requires explicit selection"));
    }
    let ticket = ctx
        .start_import_source_checked(scope(&payload)?, request, choice, generation)
        .await?;
    let mut output = Object::new();
    output.insert("id", ticket.id.0);
    Ok(Value::Object(output))
});
command!(Plugins, "semantic.plugin.list", ctx, payload, {
    let scope = ctx.resolve_scope_id(scope(&payload)?).await?;
    let plugins = ctx.app.plugins(&ctx.principal, scope).await?;
    let states = plugins.runtime.states().await;
    Ok(Value::List(
        plugins
            .list()
            .await?
            .iter()
            .map(|activation| {
                let Value::Object(mut object) = activation.to_value() else {
                    unreachable!("activation object")
                };
                if let Some((_, state, error)) =
                    states.iter().find(|(id, _, _)| id == &activation.id)
                {
                    object.insert("state", state.as_str().to_owned());
                    object.insert(
                        "error",
                        error
                            .as_ref()
                            .map(|e| {
                                let mut error = Object::new();
                                error.insert("code", e.code.clone());
                                error.insert("message", e.message.clone());
                                Value::Object(error)
                            })
                            .unwrap_or(Value::Null),
                    );
                }
                Value::Object(object)
            })
            .collect(),
    ))
});
command!(Configure, "semantic.plugin.configure", ctx, payload, {
    let activation = PluginActivation::from_value(
        payload
            .get("activation")
            .ok_or_else(|| error("activation required"))?,
    )
    .map_err(error)?;
    let scope = ctx.resolve_scope_id(scope(&payload)?).await?;
    ctx.app
        .plugins(&ctx.principal, scope)
        .await?
        .configure(activation)
        .await?;
    Ok(Value::Null)
});
command!(Uninstall, "semantic.plugin.uninstall", ctx, payload, {
    let id = optional_string(&payload, "id")?.ok_or_else(|| error("plugin id required"))?;
    let scope = ctx.resolve_scope_id(scope(&payload)?).await?;
    ctx.app
        .plugins(&ctx.principal, scope)
        .await?
        .uninstall(id)
        .await?;
    Ok(Value::Null)
});
