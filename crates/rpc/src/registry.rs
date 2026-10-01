use std::collections::BTreeMap;

use semantic_data::value::Value;
use semantic_rpc_core::command::{CallError, CommandAdapter, DynCommand, RpcCommand};
use semantic_rpc_core::error::{RegisterError, RpcError};
use semantic_rpc_core::protocol::{RpcRequest, RpcResponse};

pub struct RpcRegistry<Ctx, E> {
    commands: BTreeMap<String, Box<dyn DynCommand<Ctx, E>>>,
}

impl<Ctx, E> RpcRegistry<Ctx, E> {
    pub fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
        }
    }

    pub fn register<C>(&mut self, command: C) -> Result<(), RegisterError>
    where
        C: RpcCommand<Ctx>,
        C::Error: Into<E>,
        Ctx: Sync,
    {
        self.register_dyn(Box::new(CommandAdapter::new(command)))
    }

    /// Register an erased handler, rejecting duplicate command names and
    /// definitions with misplaced streams.
    pub fn register_dyn(
        &mut self,
        command: Box<dyn DynCommand<Ctx, E>>,
    ) -> Result<(), RegisterError> {
        command.definition().validate()?;
        let name = command.name().to_owned();
        if self.commands.contains_key(&name) {
            return Err(RegisterError::DuplicateCommand(name));
        }

        self.commands.insert(name, command);

        Ok(())
    }

    /// Registered commands, ordered by name.
    pub fn commands(&self) -> impl Iterator<Item = &dyn DynCommand<Ctx, E>> + '_ {
        self.commands.values().map(|command| command.as_ref())
    }

    pub fn get(&self, command: &str) -> Option<&dyn DynCommand<Ctx, E>> {
        self.commands.get(command).map(|command| command.as_ref())
    }

    /// Call a command by name, preserving its typed error.
    pub async fn call(
        &self,
        ctx: &Ctx,
        command: &str,
        payload: Value,
    ) -> Result<Value, CallError<E>> {
        let Some(handler) = self.commands.get(command) else {
            return Err(CallError::UnknownCommand(command.to_owned()));
        };

        handler.call_value(ctx, payload).await
    }

    pub async fn invoke(&self, ctx: &Ctx, request: RpcRequest) -> RpcResponse
    where
        E: Into<RpcError>,
    {
        let result = self
            .call(ctx, &request.command, request.payload)
            .await
            .map_err(RpcError::from);

        RpcResponse {
            id: request.id,
            result: result.into(),
        }
    }
}

impl<Ctx, E> Default for RpcRegistry<Ctx, E> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use semantic_data::value::{SemanticType, StreamOf, Value};

    use semantic_rpc_core::{
        CallError, CommandDef, CommandDefError, RegisterError, RpcCommand, RpcCommandSpec,
        RpcError, RpcResult,
    };

    use super::RpcRegistry;

    struct EchoCommand;

    impl RpcCommandSpec for EchoCommand {
        type Payload = Value;
        type Output = Value;
        type Error = RpcError;

        const NAME: &'static str = "test.echo";
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

    struct FailCommand;

    impl RpcCommandSpec for FailCommand {
        type Payload = ();
        type Output = ();
        type Error = &'static str;

        const NAME: &'static str = "test.fail";
    }

    impl RpcCommand<()> for FailCommand {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: Self::Payload,
        ) -> Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send + 'a>> {
            Box::pin(async move { Err("boom") })
        }
    }

    #[tokio::test]
    async fn registry_invokes_registered_command() {
        let mut registry = RpcRegistry::<(), RpcError>::new();
        registry.register(EchoCommand).expect("register command");

        let response = registry
            .invoke(
                &(),
                semantic_rpc_core::RpcRequest {
                    id: 7,
                    command: "test.echo".to_string(),
                    payload: Value::U8(42),
                },
            )
            .await;

        assert_eq!(response.id, 7);
        assert_eq!(response.result, RpcResult::Ok(Value::U8(42)));
    }

    #[test]
    fn registry_enumerates_commands_by_name() {
        let mut registry = RpcRegistry::<(), RpcError>::new();
        registry.register(FailCommand).expect("register command");
        registry.register(EchoCommand).expect("register command");

        let names: Vec<_> = registry.commands().map(|command| command.name()).collect();

        assert_eq!(names, ["test.echo", "test.fail"]);
    }

    struct StreamInputCommand;

    impl RpcCommandSpec for StreamInputCommand {
        type Payload = Value;
        type Output = Value;
        type Error = RpcError;

        const NAME: &'static str = "test.stream_input";

        fn definition(&self) -> CommandDef {
            CommandDef::new(
                Self::NAME,
                StreamOf::<Value>::semantic_type(),
                Value::semantic_type(),
            )
        }
    }

    impl RpcCommand<()> for StreamInputCommand {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            payload: Self::Payload,
        ) -> Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send + 'a>> {
            Box::pin(async move { Ok(payload) })
        }
    }

    #[test]
    fn registry_rejects_misplaced_streams() {
        let mut registry = RpcRegistry::<(), RpcError>::new();

        let result = registry.register(StreamInputCommand);

        assert!(matches!(
            result,
            Err(RegisterError::InvalidDefinition(CommandDefError::StreamInput(name)))
                if name == "test.stream_input"
        ));
        assert!(registry.get("test.stream_input").is_none());
    }

    #[tokio::test]
    async fn registry_reports_unknown_command() {
        let registry = RpcRegistry::<(), RpcError>::new();

        let response = registry
            .invoke(
                &(),
                semantic_rpc_core::RpcRequest {
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

    #[tokio::test]
    async fn registry_invoke_folds_typed_command_errors() {
        let mut registry = RpcRegistry::<(), String>::new();
        registry.register(FailCommand).expect("register command");

        let response = registry
            .invoke(
                &(),
                semantic_rpc_core::RpcRequest {
                    id: 3,
                    command: "test.fail".to_string(),
                    payload: Value::Void,
                },
            )
            .await;

        assert_eq!(response.id, 3);
        assert_eq!(response.result, RpcResult::Err(RpcError::from("boom")));
    }

    #[tokio::test]
    async fn registry_call_preserves_typed_errors() {
        let mut registry = RpcRegistry::<(), String>::new();
        registry.register(FailCommand).expect("register command");

        match registry.call(&(), "test.fail", Value::Void).await {
            Err(CallError::Command(err)) => assert_eq!(err, "boom"),
            other => panic!("unexpected result: {other:?}"),
        }
        match registry.call(&(), "missing", Value::Void).await {
            Err(CallError::UnknownCommand(command)) => assert_eq!(command, "missing"),
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
