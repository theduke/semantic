use crate::command::ScopeParams;
use crate::{AppError, AppRequestContext, DbScopeId};
use semantic_data::jobs::*;
use semantic_data::value::{FromValue, IntoValue, SemanticType};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{RpcCommand, RpcCommandSpec};
use std::{future::Future, pin::Pin};

pub(crate) fn register(
    registry: &mut RpcRegistry<AppRequestContext, AppError>,
) -> Result<(), AppError> {
    registry.register(List)?;
    registry.register(Get)?;
    registry.register(Cancel)?;
    registry.register(Clear)?;
    registry.register(Kinds)?;
    Ok(())
}

#[derive(SemanticType, IntoValue, FromValue, Default)]
struct ListPayload {
    scope_id: Option<String>,
    #[semantic(flatten)]
    query: JobListQuery,
}

#[derive(SemanticType, IntoValue, FromValue)]
struct IdPayload {
    scope_id: Option<String>,
    id: JobId,
}

/// Jobs commands accept null or void as an empty payload.
macro_rules! command {
    ($type:ident, $name:literal, $payload_ty:ty => $output:ty, |$ctx:ident, $payload:ident| $body:block) => {
        struct $type;
        impl RpcCommandSpec for $type {
            type Payload = Option<$payload_ty>;
            type Output = $output;
            type Error = AppError;
            const NAME: &'static str = $name;
        }
        impl RpcCommand<AppRequestContext> for $type {
            fn call<'a>(
                &'a self,
                $ctx: &'a AppRequestContext,
                payload: Option<$payload_ty>,
            ) -> Pin<Box<dyn Future<Output = Result<$output, AppError>> + Send + 'a>> {
                Box::pin(async move {
                    let $payload = payload;
                    $body
                })
            }
        }
    };
}

fn required(payload: Option<IdPayload>) -> Result<IdPayload, AppError> {
    payload.ok_or_else(|| super::error("job id required"))
}

command!(List, "semantic.jobs.list", ListPayload => JobListPage, |ctx, payload| {
    let ListPayload { scope_id, query } = payload.unwrap_or_default();
    if query.limit == 0 {
        return Err(super::error("limit must be positive"));
    }
    Ok(ctx
        .jobs(scope_id.map(DbScopeId::new))
        .await?
        .list(query)
        .await?)
});
command!(Get, "semantic.jobs.get", IdPayload => Option<JobRecord>, |ctx, payload| {
    let IdPayload { scope_id, id } = required(payload)?;
    Ok(ctx.jobs(scope_id.map(DbScopeId::new)).await?.get(id).await?)
});
command!(Cancel, "semantic.jobs.cancel", IdPayload => JobRecord, |ctx, payload| {
    let IdPayload { scope_id, id } = required(payload)?;
    Ok(ctx.jobs(scope_id.map(DbScopeId::new)).await?.cancel(id).await?)
});
command!(Clear, "semantic.jobs.clear_completed", ScopeParams => ClearCompletedResult, |ctx, payload| {
    let scope_id = payload.unwrap_or_default().scope_id();
    Ok(ctx.jobs(scope_id).await?.clear_completed().await?)
});
command!(Kinds, "semantic.jobs.kinds", ScopeParams => Vec<JobKindDescriptor>, |ctx, payload| {
    let scope_id = payload.unwrap_or_default().scope_id();
    Ok(ctx.jobs(scope_id).await?.kinds())
});
