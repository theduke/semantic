use std::collections::BTreeMap;

use crate::command::{CommandAdapter, DynCommand, RpcCommand};
use crate::error::{RegisterError, RpcError};
use crate::protocol::{RpcRequest, RpcResponse};

pub struct RpcRegistry<Ctx> {
    commands: BTreeMap<String, Box<dyn DynCommand<Ctx>>>,
}

impl<Ctx> RpcRegistry<Ctx> {
    pub fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
        }
    }

    pub fn register<C>(&mut self, command: C) -> Result<(), RegisterError>
    where
        C: RpcCommand<Ctx>,
        Ctx: Sync,
    {
        let name = C::NAME.to_owned();
        if self.commands.contains_key(&name) {
            return Err(RegisterError::DuplicateCommand(name));
        }

        self.commands
            .insert(name, Box::new(CommandAdapter::new(command)));

        Ok(())
    }

    pub fn get(&self, command: &str) -> Option<&dyn DynCommand<Ctx>> {
        self.commands.get(command).map(|command| command.as_ref())
    }

    pub async fn invoke(&self, ctx: &Ctx, request: RpcRequest) -> RpcResponse {
        let Some(command) = self.commands.get(&request.command) else {
            return RpcResponse::err(request.id, RpcError::unknown_command(request.command));
        };

        RpcResponse {
            id: request.id,
            result: command.call_value(ctx, request.payload).await,
        }
    }
}

impl<Ctx> Default for RpcRegistry<Ctx> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use semantic_data::schema::FunctionType;
    use semantic_data::value::Value;

    use crate::{RpcCommand, RpcCommandSpec, RpcResult};

    use super::RpcRegistry;

    struct EchoCommand;

    impl RpcCommandSpec for EchoCommand {
        type Payload = Value;
        type Output = Value;
        type Error = crate::RpcError;

        const NAME: &'static str = "test.echo";

        fn signature(&self) -> FunctionType {
            FunctionType {
                params: Vec::new(),
                results: Vec::new(),
                throws: None,
                async_fn: true,
            }
        }
    }

    impl RpcCommand<()> for EchoCommand {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            payload: Self::Payload,
        ) -> Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send + 'a>> {
            Box::pin(async move { Ok(payload) })
        }
    }

    #[tokio::test]
    async fn registry_invokes_registered_command() {
        let mut registry = RpcRegistry::new();
        registry.register(EchoCommand).expect("register command");

        let response = registry
            .invoke(
                &(),
                crate::RpcRequest {
                    id: 7,
                    command: "test.echo".to_string(),
                    payload: Value::U8(42),
                },
            )
            .await;

        assert_eq!(response.id, 7);
        assert_eq!(response.result, RpcResult::Ok(Value::U8(42)));
    }

    #[tokio::test]
    async fn registry_reports_unknown_command() {
        let registry = RpcRegistry::new();

        let response = registry
            .invoke(
                &(),
                crate::RpcRequest {
                    id: 9,
                    command: "missing".to_string(),
                    payload: Value::Void,
                },
            )
            .await;

        assert_eq!(response.id, 9);
        match response.result {
            RpcResult::Err(err) => assert_eq!(err.code, "unknown_command"),
            RpcResult::Ok(value) => panic!("unexpected ok response: {value:?}"),
        }
    }
}
