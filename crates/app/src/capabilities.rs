//! Feature availability combines active scope schema with registered handlers.
use crate::{AppError, AppRequestContext, DbScopeId};
use semantic_data::value::{FromValue, IntoValue, SemanticType};
use semantic_rpc::RpcRegistry;
use semantic_rpc_core::{RpcCommand, RpcCommandSpec};

pub(crate) fn register(
    registry: &mut RpcRegistry<AppRequestContext, AppError>,
) -> Result<(), AppError> {
    registry.register(Capabilities)?;
    Ok(())
}

struct Capabilities;
#[derive(Default, SemanticType, IntoValue, FromValue)]
struct Payload {
    scope_id: Option<String>,
}
#[derive(SemanticType, IntoValue, FromValue)]
struct Output {
    tasks: bool,
    comments: bool,
}
impl RpcCommandSpec for Capabilities {
    type Payload = Option<Payload>;
    type Output = Output;
    type Error = AppError;
    const NAME: &'static str = "semantic.app.capabilities";
}
impl RpcCommand<AppRequestContext> for Capabilities {
    fn call<'a>(
        &'a self,
        ctx: &'a AppRequestContext,
        payload: Option<Payload>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Output, AppError>> + Send + 'a>>
    {
        Box::pin(async move {
            let db = ctx
                .resolve_db(payload.unwrap_or_default().scope_id.map(DbScopeId::new))
                .await?;
            let catalog = db.catalog().await?;
            let available = |command: &str, class: &str| {
                ctx.app
                    .registry()
                    .commands()
                    .any(|handler| handler.name() == command)
                    && catalog.class_id(class).is_some()
            };
            Ok(Output {
                tasks: available("semantic.tasks.list", "semantic:tasks:task"),
                comments: available("semantic.comments.list", "semantic:comments:comment"),
            })
        })
    }
}
