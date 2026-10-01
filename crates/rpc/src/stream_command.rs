//! Typed streaming commands.
//!
//! Streaming commands take a payload plus an optional client-to-server stream
//! and answer with either a single value or a server-to-client stream. They
//! are served through the native interface session, never through the unary
//! [`RpcCommand`](semantic_rpc_core::RpcCommand) path.
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::future::BoxFuture;
use futures::{Stream, StreamExt};
use semantic_data::schema::Type;
use semantic_data::value::{FromValue, IntoValue, SemanticType, StreamOf, Value};
use semantic_rpc_core::{CallError, CommandDef, RpcError};

use crate::interface::{
    CancellationToken, InvocationArgument, InvocationError, InvocationOutput, OwnedValueStream,
    StreamEvent,
};

/// Export under which the command registry is served over interface sessions.
pub const COMMAND_EXPORT: &str = "semantic.command";

/// The terminal value of a stream.
///
/// Streams whose end type is `()` (schema type `Never`) carry no end value on
/// the wire.
pub trait StreamEnd: SemanticType + IntoValue + FromValue + Send + 'static {
    /// `None` when the end type is `Never`.
    fn into_end(self) -> Option<Value>;

    /// A missing end value decodes like [`Value::Void`].
    fn from_end(value: Option<Value>) -> Result<Self, InvocationError>;
}

impl<T: SemanticType + IntoValue + FromValue + Send + 'static> StreamEnd for T {
    fn into_end(self) -> Option<Value> {
        if matches!(
            Self::semantic_type().kind,
            semantic_data::schema::TypeKind::Never(_)
        ) {
            None
        } else {
            Some(self.into_value())
        }
    }

    fn from_end(value: Option<Value>) -> Result<Self, InvocationError> {
        T::from_value(value.unwrap_or(Value::Void)).map_err(|err| {
            InvocationError::new("invalid_stream_end", err.describe("stream end").to_string())
        })
    }
}

/// An event of a [`TypedStream`].
#[derive(Clone, Debug, PartialEq)]
pub enum TypedEvent<T, End> {
    Item(T),
    End(End),
}

/// A live stream of typed items, terminated by an explicit end event.
///
/// Wraps an [`OwnedValueStream`]; dropping it drops the producer.
pub struct TypedStream<T, End = ()> {
    inner: OwnedValueStream,
    _marker: PhantomData<fn() -> (T, End)>,
}

impl<T, End> TypedStream<T, End> {
    pub fn from_owned(inner: OwnedValueStream) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }

    pub fn into_owned(self) -> OwnedValueStream {
        self.inner
    }
}

impl<T, End> TypedStream<T, End>
where
    T: IntoValue + Send + 'static,
    End: StreamEnd,
{
    /// Build a stream from typed events. The events must include the terminal
    /// [`TypedEvent::End`].
    pub fn from_events(
        events: impl Stream<Item = Result<TypedEvent<T, End>, InvocationError>> + Send + 'static,
    ) -> Self {
        Self::from_owned(OwnedValueStream::new(events.map(|event| {
            event.map(|event| match event {
                TypedEvent::Item(item) => StreamEvent::Item(item.into_value()),
                TypedEvent::End(end) => StreamEvent::End(end.into_end()),
            })
        })))
    }
}

impl<T, End> Unpin for TypedStream<T, End> {}

impl<T: FromValue, End: StreamEnd> Stream for TypedStream<T, End> {
    type Item = Result<TypedEvent<T, End>, InvocationError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let event = match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Ready(Some(event)) => event,
        };
        Poll::Ready(Some(event.and_then(|event| match event {
            StreamEvent::Item(value) => T::from_value(value).map(TypedEvent::Item).map_err(|err| {
                InvocationError::new(
                    "invalid_stream_item",
                    err.describe("stream item").to_string(),
                )
            }),
            StreamEvent::End(end) => End::from_end(end).map(TypedEvent::End),
        })))
    }
}

/// The client-to-server side of a command: nothing or a stream.
pub trait InputShape: 'static {
    type Live: Send + 'static;

    /// The declared input stream type, if any.
    fn stream_type() -> Option<Type>;

    fn from_argument(argument: Option<OwnedValueStream>) -> Result<Self::Live, InvocationError>;

    fn into_argument(live: Self::Live) -> Option<InvocationArgument>;
}

impl InputShape for () {
    type Live = ();

    fn stream_type() -> Option<Type> {
        None
    }

    fn from_argument(argument: Option<OwnedValueStream>) -> Result<Self::Live, InvocationError> {
        match argument {
            None => Ok(()),
            Some(_) => Err(InvocationError::new(
                "invalid_argument",
                "command does not accept an input stream",
            )),
        }
    }

    fn into_argument(_live: Self::Live) -> Option<InvocationArgument> {
        None
    }
}

impl<T, End> InputShape for StreamOf<T, End>
where
    T: SemanticType + IntoValue + FromValue + Send + 'static,
    End: StreamEnd,
{
    type Live = TypedStream<T, End>;

    fn stream_type() -> Option<Type> {
        Some(StreamOf::<T, End>::semantic_type())
    }

    fn from_argument(argument: Option<OwnedValueStream>) -> Result<Self::Live, InvocationError> {
        argument.map(TypedStream::from_owned).ok_or_else(|| {
            InvocationError::new("invalid_argument", "command requires an input stream")
        })
    }

    fn into_argument(live: Self::Live) -> Option<InvocationArgument> {
        Some(InvocationArgument::Stream(live.into_owned()))
    }
}

/// Marker for a command answering with a single value of type `T`.
///
/// A distinct marker is needed because a blanket impl for value types would
/// overlap with the [`StreamOf`] impl.
pub struct Single<T>(PhantomData<fn() -> T>);

/// The server-to-client side of a command: a value or a stream.
pub trait OutputShape: 'static {
    type Live: Send + 'static;

    fn output_type() -> Type;

    fn into_output(live: Self::Live) -> InvocationOutput;

    fn from_output(output: InvocationOutput) -> Result<Self::Live, InvocationError>;
}

impl<T> OutputShape for Single<T>
where
    T: SemanticType + IntoValue + FromValue + Send + 'static,
{
    type Live = T;

    fn output_type() -> Type {
        T::semantic_type()
    }

    fn into_output(live: Self::Live) -> InvocationOutput {
        InvocationOutput::Values(vec![live.into_value()])
    }

    fn from_output(output: InvocationOutput) -> Result<Self::Live, InvocationError> {
        match output {
            InvocationOutput::Values(mut values) if values.len() == 1 => {
                T::from_value(values.remove(0)).map_err(|err| {
                    InvocationError::new("invalid_output", err.describe("output").to_string())
                })
            }
            InvocationOutput::Values(values) => Err(InvocationError::new(
                "invalid_output",
                format!("expected one output value, got {}", values.len()),
            )),
            InvocationOutput::Stream(_) => Err(InvocationError::new(
                "invalid_output",
                "expected a value output, got a stream",
            )),
        }
    }
}

impl<T, End> OutputShape for StreamOf<T, End>
where
    T: SemanticType + IntoValue + FromValue + Send + 'static,
    End: StreamEnd,
{
    type Live = TypedStream<T, End>;

    fn output_type() -> Type {
        StreamOf::<T, End>::semantic_type()
    }

    fn into_output(live: Self::Live) -> InvocationOutput {
        InvocationOutput::Stream(live.into_owned())
    }

    fn from_output(output: InvocationOutput) -> Result<Self::Live, InvocationError> {
        match output {
            InvocationOutput::Stream(stream) => Ok(TypedStream::from_owned(stream)),
            InvocationOutput::Values(_) => Err(InvocationError::new(
                "invalid_output",
                "expected a stream output, got values",
            )),
        }
    }
}

/// Static description of a streaming command, usable by clients and servers.
pub trait RpcStreamCommandSpec {
    type Payload: SemanticType + IntoValue + FromValue + Send + 'static;
    /// `()` or [`StreamOf`].
    type Input: InputShape;
    /// [`Single`] or [`StreamOf`].
    type Output: OutputShape;
    type Error: Send + 'static;

    const NAME: &'static str;

    fn definition() -> CommandDef {
        let definition = CommandDef::new(
            Self::NAME,
            Self::Payload::semantic_type(),
            Self::Output::output_type(),
        );
        match Self::Input::stream_type() {
            Some(stream) => definition.with_input_stream(stream),
            None => definition,
        }
    }
}

pub trait RpcStreamCommand<Ctx>: RpcStreamCommandSpec + Send + Sync + 'static {
    fn call<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Self::Payload,
        input: <Self::Input as InputShape>::Live,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<<Self::Output as OutputShape>::Live, Self::Error>>;
}

/// Type-erased streaming command.
pub trait DynStreamCommand<Ctx, E>: Send + Sync {
    fn definition(&self) -> &CommandDef;

    fn invoke<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Value,
        input: Option<OwnedValueStream>,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<InvocationOutput, CallError<E>>>;
}

pub struct StreamCommandAdapter<C> {
    command: C,
    definition: CommandDef,
}

impl<C: RpcStreamCommandSpec> StreamCommandAdapter<C> {
    pub fn new(command: C) -> Self {
        Self {
            command,
            definition: C::definition(),
        }
    }
}

impl<Ctx, E, C> DynStreamCommand<Ctx, E> for StreamCommandAdapter<C>
where
    C: RpcStreamCommand<Ctx>,
    C::Error: Into<E>,
    Ctx: Sync,
{
    fn definition(&self) -> &CommandDef {
        &self.definition
    }

    fn invoke<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Value,
        input: Option<OwnedValueStream>,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<InvocationOutput, CallError<E>>> {
        Box::pin(async move {
            let payload = C::Payload::from_value(payload).map_err(|err| {
                CallError::InvalidPayload(RpcError::invalid_payload(err.describe("payload")))
            })?;
            let input = C::Input::from_argument(input)
                .map_err(|err| CallError::InvalidPayload(RpcError::invalid_payload(err.message)))?;
            let output = self
                .command
                .call(ctx, payload, input, cancel)
                .await
                .map_err(|err| CallError::Command(err.into()))?;
            Ok(C::Output::into_output(output))
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use futures::stream;
    use semantic_data::schema::TypeKind;

    use super::*;

    struct Echo;

    impl RpcStreamCommandSpec for Echo {
        type Payload = u32;
        type Input = StreamOf<String, bool>;
        type Output = StreamOf<String, bool>;
        type Error = RpcError;

        const NAME: &'static str = "test.echo";
    }

    impl RpcStreamCommand<()> for Echo {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: u32,
            input: TypedStream<String, bool>,
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<TypedStream<String, bool>, RpcError>> {
            Box::pin(async move { Ok(TypedStream::from_events(input)) })
        }
    }

    struct Count;

    impl RpcStreamCommandSpec for Count {
        type Payload = u32;
        type Input = ();
        type Output = StreamOf<u32>;
        type Error = RpcError;

        const NAME: &'static str = "test.count";
    }

    struct Total;

    impl RpcStreamCommandSpec for Total {
        type Payload = ();
        type Input = StreamOf<u32>;
        type Output = Single<u64>;
        type Error = RpcError;

        const NAME: &'static str = "test.total";
    }

    fn typed_events(
        items: &[&str],
        end: bool,
    ) -> Vec<Result<TypedEvent<String, bool>, InvocationError>> {
        items
            .iter()
            .map(|item| Ok(TypedEvent::Item((*item).to_owned())))
            .chain([Ok(TypedEvent::End(end))])
            .collect()
    }

    #[test]
    fn typed_stream_round_trips_items_and_end() {
        let stream =
            TypedStream::<String, bool>::from_events(stream::iter(typed_events(&["a", "b"], true)));
        let raw: Vec<_> = block_on(stream.into_owned().collect());
        assert_eq!(
            raw,
            vec![
                Ok(StreamEvent::Item(Value::String("a".into()))),
                Ok(StreamEvent::Item(Value::String("b".into()))),
                Ok(StreamEvent::End(Some(Value::Bool(true)))),
            ]
        );

        let stream = TypedStream::<String, bool>::from_events(stream::iter(typed_events(
            &["a", "b"],
            false,
        )));
        let events: Vec<_> =
            block_on(TypedStream::<String, bool>::from_owned(stream.into_owned()).collect());
        assert_eq!(events, typed_events(&["a", "b"], false));
    }

    #[test]
    fn unit_end_is_absent_on_the_wire() {
        let stream = TypedStream::<u32>::from_events(stream::iter([
            Ok(TypedEvent::Item(1)),
            Ok(TypedEvent::End(())),
        ]));
        let raw: Vec<_> = block_on(stream.into_owned().collect());
        assert_eq!(
            raw.last(),
            Some(&Ok(StreamEvent::End(None))),
            "unexpected events: {raw:?}"
        );

        let stream = TypedStream::<u32>::from_owned(OwnedValueStream::new(stream::iter([
            Ok(StreamEvent::Item(Value::U32(1))),
            Ok(StreamEvent::End(None)),
        ])));
        let events: Vec<_> = block_on(stream.collect());
        assert_eq!(
            events,
            vec![Ok(TypedEvent::Item(1)), Ok(TypedEvent::End(()))]
        );
    }

    #[test]
    fn typed_stream_reports_decode_and_transport_errors() {
        let stream = TypedStream::<u32>::from_owned(OwnedValueStream::new(stream::iter([
            Ok(StreamEvent::Item(Value::String("nope".into()))),
            Err(InvocationError::new("boom", "failed")),
        ])));
        let events: Vec<_> = block_on(stream.collect());
        assert_eq!(events[0].as_ref().unwrap_err().code, "invalid_stream_item");
        assert_eq!(events[1], Err(InvocationError::new("boom", "failed")));
    }

    #[test]
    fn missing_terminal_event_is_an_error() {
        let stream = TypedStream::<u32>::from_owned(OwnedValueStream::new(stream::empty()));
        let events: Vec<_> = block_on(stream.collect());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].as_ref().unwrap_err().code, "invalid_output");
    }

    #[test]
    fn definitions_are_derived_from_shapes() {
        let echo = Echo::definition();
        assert_eq!(echo.name, "test.echo");
        assert_eq!(echo.input, u32::semantic_type());
        assert_eq!(
            echo.input_stream,
            Some(StreamOf::<String, bool>::semantic_type())
        );
        assert_eq!(echo.output, StreamOf::<String, bool>::semantic_type());
        assert_eq!(echo.validate(), Ok(()));

        let count = Count::definition();
        assert_eq!(count.input_stream, None);
        assert!(matches!(count.output.kind, TypeKind::Stream(_)));
        assert!(count.is_streaming());

        let total = Total::definition();
        assert!(total.input_stream.is_some());
        assert_eq!(total.output, u64::semantic_type());
        assert!(total.is_streaming());
        assert_eq!(total.validate(), Ok(()));
    }

    #[test]
    fn single_output_round_trips_and_rejects_streams() {
        let output = <Single<u64> as OutputShape>::into_output(7);
        assert!(matches!(&output, InvocationOutput::Values(v) if v == &[Value::U64(7)]));
        assert_eq!(<Single<u64> as OutputShape>::from_output(output), Ok(7));

        let stream = InvocationOutput::Stream(OwnedValueStream::new(stream::empty()));
        assert!(<Single<u64> as OutputShape>::from_output(stream).is_err());
        let values = InvocationOutput::Values(vec![]);
        assert!(<StreamOf<u32> as OutputShape>::from_output(values).is_err());
    }

    #[test]
    fn input_shape_checks_argument_presence() {
        assert!(<() as InputShape>::from_argument(None).is_ok());
        let stream = OwnedValueStream::new(stream::empty());
        assert!(<() as InputShape>::from_argument(Some(stream)).is_err());
        assert!(<StreamOf<u32> as InputShape>::from_argument(None).is_err());
        assert!(<() as InputShape>::into_argument(()).is_none());
    }

    #[test]
    fn adapter_decodes_payload_and_runs_command() {
        let adapter = StreamCommandAdapter::new(Echo);
        let input = OwnedValueStream::new(stream::iter([
            Ok(StreamEvent::Item(Value::String("x".into()))),
            Ok(StreamEvent::End(Some(Value::Bool(true)))),
        ]));
        let output = block_on(DynStreamCommand::<(), RpcError>::invoke(
            &adapter,
            &(),
            Value::U32(1),
            Some(input),
            CancellationToken::new(),
        ))
        .map_err(|_| ())
        .unwrap();
        let InvocationOutput::Stream(stream) = output else {
            panic!("expected stream output");
        };
        let events: Vec<_> = block_on(stream.collect());
        assert_eq!(
            events,
            vec![
                Ok(StreamEvent::Item(Value::String("x".into()))),
                Ok(StreamEvent::End(Some(Value::Bool(true)))),
            ]
        );

        let err = block_on(DynStreamCommand::<(), RpcError>::invoke(
            &adapter,
            &(),
            Value::String("bad".into()),
            None,
            CancellationToken::new(),
        ));
        assert!(matches!(err, Err(CallError::InvalidPayload(_))));
    }
}
