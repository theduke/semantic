//! The command registry exposed as an interface implementation.
//!
//! Every registered command becomes one method of the [`COMMAND_EXPORT`]
//! interface, so unary and streaming commands share one session transport.
//!
//! `()` payloads and results have the schema type `Never`, which no value
//! satisfies; on the wire they are `Null` instead.
use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::{Either, select};
use futures::pin_mut;
use semantic_data::schema::{
    AnyType, FunctionParam, FunctionType, InterfaceMethod, InterfaceType, NullType, Type, TypeKind,
    interface_fingerprint,
};
use semantic_data::value::Value;
use semantic_rpc_core::{CallError, RpcError};

use super::{
    ConformingImplementation, ImplementationDescriptor, InterfaceImplementation, InterfaceRef,
    InvocationArgument, InvocationContext, InvocationError, InvocationFuture, InvocationOutput,
    ValidatedInvocation,
};
use crate::registry::RpcRegistry;
use crate::stream_command::COMMAND_EXPORT;

/// Name of the payload parameter of every command method.
pub const PAYLOAD_PARAM: &str = "payload";
/// Name of the optional input stream parameter of a command method.
pub const INPUT_PARAM: &str = "input";

/// Interface with one method per registered command, ordered by name.
///
/// Methods take the payload and, for commands with an input stream, the
/// stream as a second argument. Command errors may carry any value.
pub fn registry_interface<Ctx, E>(registry: &RpcRegistry<Ctx, E>) -> InterfaceType {
    let methods = registry
        .commands()
        .map(|command| {
            let definition = command.definition();
            let mut params = vec![FunctionParam {
                name: Some(PAYLOAD_PARAM.into()),
                ty: wire_type(&definition.input),
            }];
            if let Some(stream) = &definition.input_stream {
                params.push(FunctionParam {
                    name: Some(INPUT_PARAM.into()),
                    ty: stream.clone(),
                });
            }
            InterfaceMethod {
                name: definition.name.clone(),
                signature: FunctionType {
                    params,
                    results: vec![wire_type(&definition.output)],
                    throws: Some(Box::new(Type::new(TypeKind::Any(AnyType)))),
                    async_fn: true,
                },
            }
        })
        .collect();
    InterfaceType { methods }
}

/// Descriptor of the command export, fingerprinting `interface`.
pub fn registry_descriptor(
    interface: &InterfaceType,
) -> Result<ImplementationDescriptor, InvocationError> {
    // Command types are inline, so there are no named definitions to resolve.
    let fingerprint = interface_fingerprint(interface, &BTreeMap::new())
        .map_err(|error| InvocationError::new("interface_incompatible", error))?;
    Ok(ImplementationDescriptor {
        export: COMMAND_EXPORT.into(),
        interface: InterfaceRef {
            package: "semantic.rpc".into(),
            module: "commands".into(),
            contract: None,
            name: "Commands".into(),
        },
        package_version: env!("CARGO_PKG_VERSION").into(),
        fingerprint,
    })
}

/// Serve `registry` as the [`COMMAND_EXPORT`] export.
///
/// `interface` and `descriptor` must come from [`registry_interface`] and
/// [`registry_descriptor`] for the same registry; they are computed once by
/// the caller because they are identical for every session.
pub fn registry_implementation<Ctx, E>(
    registry: Arc<RpcRegistry<Ctx, E>>,
    context: Arc<Ctx>,
    interface: InterfaceType,
    descriptor: ImplementationDescriptor,
) -> Result<Arc<dyn InterfaceImplementation>, InvocationError>
where
    Ctx: Send + Sync + 'static,
    E: Into<RpcError> + Send + 'static,
{
    let inner = Arc::new(RegistryImplementation {
        registry,
        context,
        descriptors: vec![descriptor],
    });
    Ok(Arc::new(ConformingImplementation::new(
        inner,
        BTreeMap::from([(COMMAND_EXPORT.to_owned(), interface)]),
        BTreeMap::new(),
    )?))
}

struct RegistryImplementation<Ctx, E> {
    registry: Arc<RpcRegistry<Ctx, E>>,
    context: Arc<Ctx>,
    descriptors: Vec<ImplementationDescriptor>,
}

impl<Ctx, E> InterfaceImplementation for RegistryImplementation<Ctx, E>
where
    Ctx: Send + Sync + 'static,
    E: Into<RpcError> + Send + 'static,
{
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &self.descriptors
    }

    fn invoke<'a>(
        &'a self,
        call: ValidatedInvocation,
        context: InvocationContext,
    ) -> InvocationFuture<'a> {
        Box::pin(async move {
            if call.export != COMMAND_EXPORT {
                return Err(invalid_argument("unknown export"));
            }
            let mut arguments = call.arguments.into_iter();
            let Some(InvocationArgument::Value(payload)) = arguments.next() else {
                return Err(invalid_argument("expected a payload value"));
            };
            let input = match arguments.next() {
                None => None,
                Some(InvocationArgument::Stream(stream)) => Some(stream),
                Some(InvocationArgument::Value(_)) => {
                    return Err(invalid_argument("expected an input stream"));
                }
            };
            if arguments.next().is_some() {
                return Err(invalid_argument("incorrect argument count"));
            }

            if let Some(command) = self.registry.stream(&call.method) {
                return command
                    .invoke(&self.context, payload, input, context.cancellation)
                    .await
                    .map(wire_output)
                    .map_err(invocation_error);
            }
            if input.is_some() {
                return Err(invalid_argument("command does not accept an input stream"));
            }
            let work = self.registry.call(&self.context, &call.method, payload);
            let cancelled = context.cancellation.cancelled();
            pin_mut!(work, cancelled);
            match select(work, cancelled).await {
                Either::Left((result, _)) => result
                    .map(|value| InvocationOutput::Values(vec![wire_value(value)]))
                    .map_err(invocation_error),
                Either::Right(_) => Err(InvocationError::new("cancelled", "call cancelled")),
            }
        })
    }
}

/// `Never` has no values, so `()` is `Null` on the wire.
fn wire_type(ty: &Type) -> Type {
    match ty.kind {
        TypeKind::Never(_) => Type::new(TypeKind::Null(NullType)),
        _ => ty.clone(),
    }
}

fn wire_value(value: Value) -> Value {
    match value {
        Value::Void => Value::Null,
        value => value,
    }
}

fn wire_output(output: InvocationOutput) -> InvocationOutput {
    match output {
        InvocationOutput::Values(values) => {
            InvocationOutput::Values(values.into_iter().map(wire_value).collect())
        }
        stream => stream,
    }
}

fn invalid_argument(message: &str) -> InvocationError {
    InvocationError::new("invalid_argument", message)
}

fn invocation_error<E: Into<RpcError>>(error: CallError<E>) -> InvocationError {
    let error = RpcError::from(error);
    InvocationError {
        code: error.code,
        message: error.message,
        data: error.data,
    }
}

#[cfg(all(test, feature = "interface-session"))]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Poll;
    use std::time::Duration;

    use futures::future::BoxFuture;
    use futures::{StreamExt, stream};
    use semantic_data::value::StreamOf;
    use semantic_rpc_core::{RpcCommand, RpcCommandSpec};

    use super::*;
    use crate::interface::session::testing::pair;
    use crate::interface::{CancellationToken, StreamEvent};
    use crate::stream_command::{
        RpcStreamCommand, RpcStreamCommandSpec, Single, TypedEvent, TypedStream,
    };

    struct Add;
    impl RpcCommandSpec for Add {
        type Payload = u32;
        type Output = u32;
        type Error = RpcError;
        const NAME: &'static str = "test.add";
    }
    impl RpcCommand<()> for Add {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            payload: u32,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<u32, RpcError>> + Send + 'a>> {
            Box::pin(async move { Ok(payload + 1) })
        }
    }

    struct Unit;
    impl RpcCommandSpec for Unit {
        type Payload = ();
        type Output = ();
        type Error = RpcError;
        const NAME: &'static str = "test.unit";
    }
    impl RpcCommand<()> for Unit {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), RpcError>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct Fail;
    impl RpcCommandSpec for Fail {
        type Payload = ();
        type Output = ();
        type Error = RpcError;
        const NAME: &'static str = "test.fail";
    }
    impl RpcCommand<()> for Fail {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), RpcError>> + Send + 'a>> {
            Box::pin(async move { Err(RpcError::with_data("denied", "nope", Value::U8(7))) })
        }
    }

    /// Never completes; flags when its work future is dropped.
    struct Hang(Arc<AtomicBool>);
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    impl RpcCommandSpec for Hang {
        type Payload = ();
        type Output = ();
        type Error = RpcError;
        const NAME: &'static str = "test.hang";
    }
    impl RpcCommand<()> for Hang {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), RpcError>> + Send + 'a>> {
            let flag = DropFlag(self.0.clone());
            Box::pin(async move {
                let _flag = flag;
                futures::future::pending().await
            })
        }
    }

    struct Count;
    impl RpcStreamCommandSpec for Count {
        type Payload = u32;
        type Input = ();
        type Output = StreamOf<u32, String>;
        type Error = RpcError;
        const NAME: &'static str = "test.count";
    }
    impl RpcStreamCommand<()> for Count {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            payload: u32,
            _input: (),
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<TypedStream<u32, String>, RpcError>> {
            Box::pin(async move {
                Ok(TypedStream::from_events(
                    stream::iter(0..payload)
                        .map(|item| Ok(TypedEvent::Item(item)))
                        .chain(stream::once(async {
                            Ok(TypedEvent::End("done".to_owned()))
                        })),
                ))
            })
        }
    }

    struct Sum;
    impl RpcStreamCommandSpec for Sum {
        type Payload = ();
        type Input = StreamOf<u32>;
        type Output = Single<u64>;
        type Error = RpcError;
        const NAME: &'static str = "test.sum";
    }
    impl RpcStreamCommand<()> for Sum {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
            mut input: TypedStream<u32>,
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<u64, RpcError>> {
            Box::pin(async move {
                let mut total = 0;
                while let Some(event) = input.next().await {
                    match event.map_err(|error| RpcError::new(error.code, error.message))? {
                        TypedEvent::Item(item) => total += u64::from(item),
                        TypedEvent::End(()) => return Ok(total),
                    }
                }
                Err(RpcError::new("truncated", "input ended early"))
            })
        }
    }

    struct Echo;
    impl RpcStreamCommandSpec for Echo {
        type Payload = ();
        type Input = StreamOf<String, u64>;
        type Output = StreamOf<String, u64>;
        type Error = RpcError;
        const NAME: &'static str = "test.echo";
    }
    impl RpcStreamCommand<()> for Echo {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
            input: TypedStream<String, u64>,
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<TypedStream<String, u64>, RpcError>> {
            Box::pin(async move { Ok(TypedStream::from_events(input)) })
        }
    }

    /// Endless producer; flags when the stream is dropped.
    struct Endless(Arc<AtomicBool>);
    impl RpcStreamCommandSpec for Endless {
        type Payload = ();
        type Input = ();
        type Output = StreamOf<u32>;
        type Error = RpcError;
        const NAME: &'static str = "test.endless";
    }
    impl RpcStreamCommand<()> for Endless {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
            _input: (),
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<TypedStream<u32>, RpcError>> {
            let flag = DropFlag(self.0.clone());
            Box::pin(async move {
                Ok(TypedStream::from_events(stream::poll_fn(move |_| {
                    let _ = &flag;
                    Poll::Ready(Some(Ok(TypedEvent::Item(1))))
                })))
            })
        }
    }

    struct Fixture {
        client: crate::interface::session::Session,
        hang_dropped: Arc<AtomicBool>,
        endless_dropped: Arc<AtomicBool>,
    }

    fn fixture() -> Fixture {
        let hang_dropped = Arc::new(AtomicBool::new(false));
        let endless_dropped = Arc::new(AtomicBool::new(false));
        let mut registry = RpcRegistry::<(), RpcError>::new();
        registry.register(Add).unwrap();
        registry.register(Unit).unwrap();
        registry.register(Fail).unwrap();
        registry.register(Hang(hang_dropped.clone())).unwrap();
        registry.register_stream(Count).unwrap();
        registry.register_stream(Sum).unwrap();
        registry.register_stream(Echo).unwrap();
        registry
            .register_stream(Endless(endless_dropped.clone()))
            .unwrap();
        let interface = registry_interface(&registry);
        let descriptor = registry_descriptor(&interface).unwrap();
        let implementation =
            registry_implementation(Arc::new(registry), Arc::new(()), interface, descriptor)
                .unwrap();
        let (client, server) = pair(implementation);
        // The server session lives as long as the client keeps it connected.
        std::mem::forget(server);
        Fixture {
            client,
            hang_dropped,
            endless_dropped,
        }
    }

    fn call(method: &str, arguments: Vec<InvocationArgument>) -> ValidatedInvocation {
        ValidatedInvocation {
            export: COMMAND_EXPORT.into(),
            method: method.into(),
            arguments,
        }
    }

    fn payload(value: Value) -> InvocationArgument {
        InvocationArgument::Value(value)
    }

    async fn invoke(
        fixture: &Fixture,
        method: &str,
        arguments: Vec<InvocationArgument>,
    ) -> Result<InvocationOutput, InvocationError> {
        fixture
            .client
            .invoke(call(method, arguments), InvocationContext::default())
            .await
    }

    async fn wait_for(flag: &AtomicBool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !flag.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("flag was not set in time");
    }

    #[test]
    fn interface_lists_commands_with_stream_parameters() {
        let mut registry = RpcRegistry::<(), RpcError>::new();
        registry.register(Unit).unwrap();
        registry.register_stream(Sum).unwrap();
        registry.register_stream(Count).unwrap();

        let interface = registry_interface(&registry);

        let names: Vec<_> = interface.methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["test.count", "test.sum", "test.unit"]);
        let sum = &interface.methods[1].signature;
        assert_eq!(sum.params.len(), 2);
        assert_eq!(sum.params[1].name.as_deref(), Some(INPUT_PARAM));
        let unit = &interface.methods[2].signature;
        assert_eq!(unit.params.len(), 1);
        assert!(matches!(unit.params[0].ty.kind, TypeKind::Null(_)));
        assert!(matches!(unit.results[0].kind, TypeKind::Null(_)));

        let first = registry_descriptor(&interface).unwrap();
        assert_eq!(first, registry_descriptor(&interface).unwrap());
        registry.register(Add).unwrap();
        let second = registry_descriptor(&registry_interface(&registry)).unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    #[tokio::test]
    async fn unary_commands_run_over_the_session() {
        let fixture = fixture();
        let InvocationOutput::Values(values) =
            invoke(&fixture, "test.add", vec![payload(Value::U32(41))])
                .await
                .unwrap()
        else {
            panic!("values")
        };
        assert_eq!(values, vec![Value::U32(42)]);

        let InvocationOutput::Values(values) =
            invoke(&fixture, "test.unit", vec![payload(Value::Null)])
                .await
                .unwrap()
        else {
            panic!("values")
        };
        assert_eq!(values, vec![Value::Null]);

        let error = invoke(&fixture, "test.fail", vec![payload(Value::Null)])
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "denied");
        assert_eq!(error.data, Some(Value::U8(7)));
    }

    #[tokio::test]
    async fn server_stream_yields_items_then_end_value() {
        let fixture = fixture();
        let InvocationOutput::Stream(stream) =
            invoke(&fixture, "test.count", vec![payload(Value::U32(3))])
                .await
                .unwrap()
        else {
            panic!("stream")
        };
        let events: Vec<_> = TypedStream::<u32, String>::from_owned(stream)
            .collect()
            .await;
        assert_eq!(
            events,
            vec![
                Ok(TypedEvent::Item(0)),
                Ok(TypedEvent::Item(1)),
                Ok(TypedEvent::Item(2)),
                Ok(TypedEvent::End("done".to_owned())),
            ]
        );
    }

    #[tokio::test]
    async fn client_stream_is_consumed_by_the_command() {
        let fixture = fixture();
        let input = TypedStream::<u32>::from_events(stream::iter([
            Ok(TypedEvent::Item(1)),
            Ok(TypedEvent::Item(2)),
            Ok(TypedEvent::Item(3)),
            Ok(TypedEvent::End(())),
        ]));
        let output = invoke(
            &fixture,
            "test.sum",
            vec![
                payload(Value::Null),
                InvocationArgument::Stream(input.into_owned()),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            <Single<u64> as crate::stream_command::OutputShape>::from_output(output),
            Ok(6)
        );
    }

    #[tokio::test]
    async fn bidirectional_stream_echoes_with_end_value() {
        let fixture = fixture();
        let input = TypedStream::<String, u64>::from_events(stream::iter([
            Ok(TypedEvent::Item("a".to_owned())),
            Ok(TypedEvent::Item("b".to_owned())),
            Ok(TypedEvent::End(9)),
        ]));
        let InvocationOutput::Stream(output) = invoke(
            &fixture,
            "test.echo",
            vec![
                payload(Value::Null),
                InvocationArgument::Stream(input.into_owned()),
            ],
        )
        .await
        .unwrap() else {
            panic!("stream")
        };
        let events: Vec<_> = TypedStream::<String, u64>::from_owned(output)
            .collect()
            .await;
        assert_eq!(
            events,
            vec![
                Ok(TypedEvent::Item("a".to_owned())),
                Ok(TypedEvent::Item("b".to_owned())),
                Ok(TypedEvent::End(9)),
            ]
        );
    }

    #[tokio::test]
    async fn unknown_commands_and_invalid_payloads_are_rejected() {
        let fixture = fixture();
        let error = invoke(&fixture, "test.missing", vec![payload(Value::Null)])
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid_argument");

        let error = invoke(
            &fixture,
            "test.add",
            vec![payload(Value::String("x".into()))],
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, "invalid_argument");

        let error = invoke(&fixture, "test.sum", vec![payload(Value::Null)])
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid_argument");

        let mut wrong_export = call("test.add", vec![payload(Value::U32(1))]);
        wrong_export.export = "other".into();
        let error = fixture
            .client
            .invoke(wrong_export, InvocationContext::default())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid_argument");
    }

    #[tokio::test]
    async fn dropping_the_output_stream_drops_the_producer() {
        let fixture = fixture();
        let InvocationOutput::Stream(mut stream) =
            invoke(&fixture, "test.endless", vec![payload(Value::Null)])
                .await
                .unwrap()
        else {
            panic!("stream")
        };
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::U32(1))
        );
        assert!(!fixture.endless_dropped.load(Ordering::SeqCst));
        drop(stream);
        wait_for(&fixture.endless_dropped).await;
    }

    #[tokio::test]
    async fn dropping_a_call_cancels_the_command() {
        let fixture = fixture();
        let call = invoke(&fixture, "test.hang", vec![payload(Value::Null)]);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), call)
                .await
                .is_err(),
            "hanging command must not complete"
        );
        wait_for(&fixture.hang_dropped).await;
    }
}
