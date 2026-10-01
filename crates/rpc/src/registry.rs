use std::collections::BTreeMap;

use std::future::Future;
use std::pin::Pin;

use semantic_data::value::Value;
use semantic_rpc_core::command::{CallError, CommandAdapter, CommandDef, DynCommand, RpcCommand};
use semantic_rpc_core::error::{CommandDefError, RegisterError, RpcError};
use semantic_rpc_core::protocol::{RpcRequest, RpcResponse};

use crate::stream_command::{DynStreamCommand, RpcStreamCommand, StreamCommandAdapter};

pub struct RpcRegistry<Ctx, E> {
    commands: BTreeMap<String, Box<dyn DynCommand<Ctx, E>>>,
    /// Streaming handlers; every entry also has a placeholder in `commands`
    /// so introspection sees it.
    streams: BTreeMap<String, Box<dyn DynStreamCommand<Ctx, E>>>,
}

/// Introspection entry of a streaming command, which cannot be called as a
/// unary command.
struct StreamingPlaceholder {
    definition: CommandDef,
}

impl<Ctx, E> DynCommand<Ctx, E> for StreamingPlaceholder
where
    Ctx: Sync,
    E: Send,
{
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn definition(&self) -> &CommandDef {
        &self.definition
    }

    fn call_value<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError<E>>> + Send + 'a>> {
        Box::pin(async move { Err(CallError::StreamingRequired(self.definition.name.clone())) })
    }
}

impl<Ctx, E> RpcRegistry<Ctx, E> {
    pub fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
            streams: BTreeMap::new(),
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
        let definition = command.definition();
        definition.validate()?;
        if definition.is_streaming() {
            return Err(CommandDefError::UnaryStreaming(definition.name.clone()).into());
        }
        let name = command.name().to_owned();
        if self.commands.contains_key(&name) {
            return Err(RegisterError::DuplicateCommand(name));
        }

        self.commands.insert(name, command);

        Ok(())
    }

    /// Register a streaming command, served over interface sessions.
    ///
    /// Its definition must stream in at least one direction. Unary calls to it
    /// fail with [`CallError::StreamingRequired`].
    pub fn register_stream<C>(&mut self, command: C) -> Result<(), RegisterError>
    where
        C: RpcStreamCommand<Ctx>,
        C::Error: Into<E>,
        Ctx: Sync,
        E: Send + 'static,
    {
        let adapter = StreamCommandAdapter::new(command);
        let definition = DynStreamCommand::<Ctx, E>::definition(&adapter).clone();
        definition.validate()?;
        if !definition.is_streaming() {
            return Err(CommandDefError::NotStreaming(definition.name).into());
        }
        let name = definition.name.clone();
        if self.commands.contains_key(&name) {
            return Err(RegisterError::DuplicateCommand(name));
        }

        self.commands
            .insert(name.clone(), Box::new(StreamingPlaceholder { definition }));
        self.streams.insert(name, Box::new(adapter));
        Ok(())
    }

    /// The streaming handler registered under `command`, if any.
    pub fn stream(&self, command: &str) -> Option<&dyn DynStreamCommand<Ctx, E>> {
        self.streams.get(command).map(|command| command.as_ref())
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

    use futures::future::BoxFuture;
    use semantic_data::value::{SemanticType, StreamOf, Value};

    use semantic_rpc_core::{
        CallError, CommandDef, CommandDefError, RegisterError, RpcCommand, RpcCommandSpec,
        RpcError, RpcResult,
    };

    use super::RpcRegistry;
    use crate::interface::CancellationToken;
    use crate::stream_command::{RpcStreamCommand, RpcStreamCommandSpec, Single, TypedStream};

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

    struct StreamEcho;

    impl RpcStreamCommandSpec for StreamEcho {
        type Payload = ();
        type Input = StreamOf<String>;
        type Output = StreamOf<String>;
        type Error = RpcError;

        const NAME: &'static str = "test.stream_echo";
    }

    impl RpcStreamCommand<()> for StreamEcho {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
            input: TypedStream<String>,
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<TypedStream<String>, RpcError>> {
            Box::pin(async move { Ok(TypedStream::from_events(input)) })
        }
    }

    struct UnaryShaped;

    impl RpcStreamCommandSpec for UnaryShaped {
        type Payload = ();
        type Input = ();
        type Output = Single<()>;
        type Error = RpcError;

        const NAME: &'static str = "test.unary_shaped";
    }

    impl RpcStreamCommand<()> for UnaryShaped {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
            _input: (),
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<(), RpcError>> {
            Box::pin(async move { Ok(()) })
        }
    }

    #[test]
    fn registry_rejects_streaming_definitions_registered_as_unary() {
        struct StreamingUnary;
        impl RpcCommandSpec for StreamingUnary {
            type Payload = Value;
            type Output = Value;
            type Error = RpcError;

            const NAME: &'static str = "test.streaming_unary";

            fn definition(&self) -> CommandDef {
                CommandDef::new(
                    Self::NAME,
                    Value::semantic_type(),
                    StreamOf::<Value>::semantic_type(),
                )
            }
        }
        impl RpcCommand<()> for StreamingUnary {
            fn call<'a>(
                &'a self,
                _ctx: &'a (),
                payload: Value,
            ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
                Box::pin(async move { Ok(payload) })
            }
        }

        let mut registry = RpcRegistry::<(), RpcError>::new();

        let result = registry.register(StreamingUnary);

        assert!(matches!(
            result,
            Err(RegisterError::InvalidDefinition(CommandDefError::UnaryStreaming(name)))
                if name == "test.streaming_unary"
        ));
        assert!(registry.get("test.streaming_unary").is_none());
    }

    #[tokio::test]
    async fn registry_serves_streaming_commands_separately() {
        let mut registry = RpcRegistry::<(), RpcError>::new();
        registry.register_stream(StreamEcho).expect("register");

        let names: Vec<_> = registry.commands().map(|command| command.name()).collect();
        assert_eq!(names, ["test.stream_echo"]);
        let definition = registry.get("test.stream_echo").unwrap().definition();
        assert_eq!(
            definition.input_stream,
            Some(StreamOf::<String>::semantic_type())
        );
        assert!(registry.stream("test.stream_echo").is_some());
        assert!(registry.stream("missing").is_none());

        match registry.call(&(), "test.stream_echo", Value::Void).await {
            Err(CallError::StreamingRequired(name)) => assert_eq!(name, "test.stream_echo"),
            other => panic!("unexpected result: {other:?}"),
        }
        let response = registry
            .invoke(
                &(),
                semantic_rpc_core::RpcRequest {
                    id: 1,
                    command: "test.stream_echo".to_string(),
                    payload: Value::Void,
                },
            )
            .await;
        match response.result {
            RpcResult::Err(err) => assert_eq!(err.code, "streaming_required"),
            RpcResult::Ok(value) => panic!("unexpected ok response: {value:?}"),
        }
    }

    #[test]
    fn registry_rejects_non_streaming_stream_registration() {
        let mut registry = RpcRegistry::<(), RpcError>::new();

        let result = registry.register_stream(UnaryShaped);

        assert!(matches!(
            result,
            Err(RegisterError::InvalidDefinition(CommandDefError::NotStreaming(name)))
                if name == "test.unary_shaped"
        ));
        assert!(registry.get("test.unary_shaped").is_none());
    }

    #[test]
    fn registry_rejects_duplicates_across_unary_and_streaming() {
        struct UnaryEcho;
        impl RpcCommandSpec for UnaryEcho {
            type Payload = Value;
            type Output = Value;
            type Error = RpcError;

            const NAME: &'static str = "test.stream_echo";
        }
        impl RpcCommand<()> for UnaryEcho {
            fn call<'a>(
                &'a self,
                _ctx: &'a (),
                payload: Value,
            ) -> Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send + 'a>> {
                Box::pin(async move { Ok(payload) })
            }
        }

        let mut registry = RpcRegistry::<(), RpcError>::new();
        registry.register_stream(StreamEcho).expect("register");
        assert!(matches!(
            registry.register(UnaryEcho),
            Err(RegisterError::DuplicateCommand(name)) if name == "test.stream_echo"
        ));
        assert!(matches!(
            registry.register_stream(StreamEcho),
            Err(RegisterError::DuplicateCommand(_))
        ));

        let mut registry = RpcRegistry::<(), RpcError>::new();
        registry.register(UnaryEcho).expect("register");
        assert!(matches!(
            registry.register_stream(StreamEcho),
            Err(RegisterError::DuplicateCommand(name)) if name == "test.stream_echo"
        ));
        assert!(registry.stream("test.stream_echo").is_none());
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
