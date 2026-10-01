use std::future::Future;
use std::pin::Pin;

use semantic_data::schema::{StreamType, Type, TypeKind};
use semantic_data::value::{FromValue, IntoValue, SemanticType, Value};

use crate::error::{CommandDefError, RpcError};

/// Typed definition of a command: its name plus payload and output types.
///
/// Unary commands have no `input_stream` and a non-stream `output`. Streams
/// only appear at the top level: as `input_stream` (client to server) and as
/// a [`TypeKind::Stream`] `output` (server to client).
#[derive(Clone, Debug, PartialEq)]
pub struct CommandDef {
    pub name: String,
    /// The request payload. Never a stream.
    pub input: Type,
    /// A [`TypeKind::Stream`] sent by the client after the payload, if any.
    pub input_stream: Option<Type>,
    /// The response; a [`TypeKind::Stream`] for server-streaming commands.
    pub output: Type,
}

impl CommandDef {
    /// A definition without an input stream.
    pub fn new(name: impl Into<String>, input: Type, output: Type) -> Self {
        Self {
            name: name.into(),
            input,
            input_stream: None,
            output,
        }
    }

    /// Declare a client-to-server stream; `stream` must be a stream type.
    pub fn with_input_stream(mut self, stream: Type) -> Self {
        self.input_stream = Some(stream);
        self
    }

    /// The client-to-server stream, if declared as a stream type.
    pub fn input_stream_type(&self) -> Option<&StreamType> {
        self.input_stream.as_ref().and_then(stream_type)
    }

    /// The server-to-client stream, if the output is a stream type.
    pub fn output_stream_type(&self) -> Option<&StreamType> {
        stream_type(&self.output)
    }

    /// Whether the command streams in either direction.
    pub fn is_streaming(&self) -> bool {
        self.input_stream.is_some() || self.output_stream_type().is_some()
    }

    /// Check the top-level stream placement.
    ///
    /// Nested streams and referenced types are checked by the interface
    /// validation profile, which has the type definitions at hand.
    pub fn validate(&self) -> Result<(), CommandDefError> {
        if stream_type(&self.input).is_some() {
            return Err(CommandDefError::StreamInput(self.name.clone()));
        }
        if self.input_stream.is_some() && self.input_stream_type().is_none() {
            return Err(CommandDefError::InvalidInputStream(self.name.clone()));
        }
        Ok(())
    }
}

fn stream_type(ty: &Type) -> Option<&StreamType> {
    match &ty.kind {
        TypeKind::Stream(stream) => Some(stream),
        _ => None,
    }
}

pub trait RpcCommandSpec {
    type Payload: SemanticType + IntoValue + FromValue + Send + 'static;
    type Output: SemanticType + IntoValue + FromValue + Send + 'static;
    type Error: Send + 'static;

    const NAME: &'static str;

    /// Derived from the payload and output types by default.
    fn definition(&self) -> CommandDef {
        CommandDef::new(
            Self::NAME,
            Self::Payload::semantic_type(),
            Self::Output::semantic_type(),
        )
    }
}

pub trait RpcCommand<Ctx>: RpcCommandSpec + Send + Sync + 'static {
    fn call<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Self::Payload,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send + 'a>>;
}

/// Failure of a type-erased command call.
///
/// Keeps the command's typed error so transports can map it as they see fit.
#[derive(Debug)]
pub enum CallError<E> {
    UnknownCommand(String),
    InvalidPayload(RpcError),
    InvalidOutput(RpcError),
    /// The command streams and cannot be called as a unary command.
    StreamingRequired(String),
    Command(E),
}

impl<E: Into<RpcError>> From<CallError<E>> for RpcError {
    fn from(err: CallError<E>) -> Self {
        match err {
            CallError::UnknownCommand(command) => RpcError::unknown_command(command),
            CallError::InvalidPayload(err) | CallError::InvalidOutput(err) => err,
            CallError::StreamingRequired(command) => RpcError::new(
                "streaming_required",
                format!("RPC command '{command}' streams and requires a streaming session"),
            ),
            CallError::Command(err) => err.into(),
        }
    }
}

pub trait DynCommand<Ctx, E>: Send + Sync {
    fn name(&self) -> &str;

    fn definition(&self) -> &CommandDef;

    fn call_value<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError<E>>> + Send + 'a>>;
}

pub struct CommandAdapter<C> {
    command: C,
    definition: CommandDef,
}

impl<C> CommandAdapter<C>
where
    C: RpcCommandSpec,
{
    pub fn new(command: C) -> Self {
        let definition = command.definition();
        Self {
            command,
            definition,
        }
    }
}

impl<Ctx, E, C> DynCommand<Ctx, E> for CommandAdapter<C>
where
    C: RpcCommand<Ctx>,
    C::Error: Into<E>,
    Ctx: Sync,
{
    fn name(&self) -> &str {
        C::NAME
    }

    fn definition(&self) -> &CommandDef {
        &self.definition
    }

    fn call_value<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError<E>>> + Send + 'a>> {
        Box::pin(async move {
            let payload = C::Payload::from_value(payload).map_err(|err| {
                CallError::InvalidPayload(RpcError::invalid_payload(err.describe("payload")))
            })?;
            let output = self
                .command
                .call(ctx, payload)
                .await
                .map_err(|err| CallError::Command(err.into()))?;
            Ok(output.into_value())
        })
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::value::StreamOf;

    use super::*;

    struct Unary;

    impl RpcCommandSpec for Unary {
        type Payload = String;
        type Output = Vec<u32>;
        type Error = RpcError;

        const NAME: &'static str = "test.unary";
    }

    #[test]
    fn derived_definition_is_unary() {
        let definition = Unary.definition();
        assert_eq!(definition.name, "test.unary");
        assert_eq!(definition.input, String::semantic_type());
        assert_eq!(definition.input_stream, None);
        assert_eq!(definition.output, Vec::<u32>::semantic_type());
        assert!(!definition.is_streaming());
        assert_eq!(definition.validate(), Ok(()));
    }

    #[test]
    fn streaming_definition_exposes_streams() {
        let definition = CommandDef::new(
            "test.stream",
            String::semantic_type(),
            StreamOf::<u32, bool>::semantic_type(),
        )
        .with_input_stream(StreamOf::<String>::semantic_type());

        assert!(definition.is_streaming());
        assert_eq!(definition.validate(), Ok(()));
        let input = definition.input_stream_type().unwrap();
        assert_eq!(*input.element, String::semantic_type());
        assert_eq!(input.end, None);
        let output = definition.output_stream_type().unwrap();
        assert_eq!(*output.element, u32::semantic_type());
        assert_eq!(output.end, Some(Box::new(bool::semantic_type())));
    }

    #[test]
    fn validate_rejects_misplaced_streams() {
        let stream_input = CommandDef::new(
            "test.input",
            StreamOf::<String>::semantic_type(),
            String::semantic_type(),
        );
        assert_eq!(
            stream_input.validate(),
            Err(CommandDefError::StreamInput("test.input".into()))
        );

        let value_input_stream = CommandDef::new(
            "test.input_stream",
            String::semantic_type(),
            String::semantic_type(),
        )
        .with_input_stream(String::semantic_type());
        assert!(value_input_stream.is_streaming());
        assert_eq!(
            value_input_stream.validate(),
            Err(CommandDefError::InvalidInputStream(
                "test.input_stream".into()
            ))
        );
    }
}
