//! The command registry exposed as an interface implementation.
//!
//! Every registered command becomes one method of the [`COMMAND_EXPORT`]
//! interface, so unary and streaming commands share one session transport.
//!
//! The export is deliberately not wrapped in [`ConformingImplementation`]:
//! the interface is synthesized from command definitions, which may use schema
//! kinds the validation profile rejects. Payloads and stream items are instead
//! decoded by the commands' typed `FromValue` implementations, and the
//! interface only provides the descriptor fingerprint. `()` payloads accept
//! both `Void` and `Null`.
use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::{Either, select};
use futures::pin_mut;
use semantic_data::schema::{
    AnyType, FunctionParam, FunctionType, InterfaceMethod, InterfaceType, Type, TypeDef, TypeKind,
    interface_fingerprint,
};
use semantic_rpc_core::{CallError, RpcError};

use super::{
    ImplementationDescriptor, InterfaceImplementation, InterfaceRef, InvocationArgument,
    InvocationContext, InvocationError, InvocationFuture, InvocationOutput, ValidatedInvocation,
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
                ty: definition.input.clone(),
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
                    results: vec![definition.output.clone()],
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
    registry_descriptor_with_definitions(interface, &BTreeMap::new())
}

/// Describe commands with the complete named type graph used by their schemas.
/// Only reachable definitions contribute to the fingerprint.
pub fn registry_descriptor_with_definitions(
    interface: &InterfaceType,
    definitions: &BTreeMap<String, TypeDef>,
) -> Result<ImplementationDescriptor, InvocationError> {
    let fingerprint = interface_fingerprint(interface, definitions)
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
/// `descriptor` must come from [`registry_descriptor`] for the same registry;
/// it is computed once by the caller because it is identical for every
/// session.
pub fn registry_implementation<Ctx, E>(
    registry: Arc<RpcRegistry<Ctx, E>>,
    context: Arc<Ctx>,
    descriptor: ImplementationDescriptor,
) -> Arc<dyn InterfaceImplementation>
where
    Ctx: Send + Sync + 'static,
    E: Into<RpcError> + Send + 'static,
{
    Arc::new(RegistryImplementation {
        registry,
        context,
        descriptors: vec![descriptor],
    })
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

            if self.registry.get(&call.method).is_none() {
                return Err(invocation_error::<E>(CallError::UnknownCommand(
                    call.method,
                )));
            }
            if let Some(command) = self.registry.stream(&call.method) {
                return command
                    .invoke(&self.context, payload, input, context.cancellation)
                    .await
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
                    .map(|value| InvocationOutput::Values(vec![value]))
                    .map_err(invocation_error),
                Either::Right(_) => Err(InvocationError::new("cancelled", "call cancelled")),
            }
        })
    }
}

fn invalid_argument(message: &str) -> InvocationError {
    InvocationError::new("invalid_argument", message)
}

/// Undecodable payloads are `invalid_argument`, unknown commands
/// `unknown_command`; command errors keep their code and data.
fn invocation_error<E: Into<RpcError>>(error: CallError<E>) -> InvocationError {
    let error = match error {
        CallError::InvalidPayload(mut error) => {
            error.code = "invalid_argument".into();
            error
        }
        error => RpcError::from(error),
    };
    InvocationError {
        code: error.code,
        message: error.message,
        data: error.data,
    }
}

#[cfg(test)]
mod named_definition_tests {
    use super::*;
    use semantic_data::value::SemanticType;

    #[test]
    fn fingerprints_resolve_recursive_query_definitions_and_ignore_unreachable_types() {
        let interface = InterfaceType {
            methods: vec![InterfaceMethod {
                name: "query".into(),
                signature: FunctionType {
                    params: vec![FunctionParam {
                        name: Some(PAYLOAD_PARAM.into()),
                        ty: semantic_data::query::Query::semantic_type(),
                    }],
                    results: vec![String::semantic_type()],
                    throws: None,
                    async_fn: true,
                },
            }],
        };
        assert!(registry_descriptor(&interface).is_err());
        let mut definitions = semantic_data::query::semantic::definitions();
        let descriptor = registry_descriptor_with_definitions(&interface, &definitions).unwrap();
        assert_eq!(
            descriptor,
            registry_descriptor_with_definitions(&interface, &definitions).unwrap()
        );
        let TypeKind::Named(reference) = semantic_data::query::Query::semantic_type().kind else {
            panic!("named query")
        };
        let mut extra = definitions.get(&reference.name).unwrap().clone();
        extra.name = "unreachable".into();
        extra.ty = String::semantic_type();
        definitions.insert("unreachable".into(), extra);
        assert_eq!(
            descriptor,
            registry_descriptor_with_definitions(&interface, &definitions).unwrap()
        );
        definitions.get_mut(&reference.name).unwrap().ty = String::semantic_type();
        assert_ne!(
            descriptor.fingerprint,
            registry_descriptor_with_definitions(&interface, &definitions)
                .unwrap()
                .fingerprint
        );
    }
}

#[cfg(all(test, feature = "interface-session"))]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Poll;
    use std::time::Duration;

    use futures::future::BoxFuture;
    use futures::{StreamExt, stream};
    use semantic_data::value::{StreamOf, Value};
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

    /// Endless producer whose `cancel` token is observed by a detached task.
    struct CancelAware(Arc<AtomicBool>);
    impl RpcStreamCommandSpec for CancelAware {
        type Payload = ();
        type Input = ();
        type Output = StreamOf<u32>;
        type Error = RpcError;
        const NAME: &'static str = "test.cancel_aware";
    }
    impl RpcStreamCommand<()> for CancelAware {
        fn call<'a>(
            &'a self,
            _ctx: &'a (),
            _payload: (),
            _input: (),
            cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<TypedStream<u32>, RpcError>> {
            let fired = self.0.clone();
            Box::pin(async move {
                tokio::spawn(async move {
                    cancel.cancelled().await;
                    fired.store(true, Ordering::SeqCst);
                });
                Ok(TypedStream::from_events(stream::repeat_with(|| {
                    Ok(TypedEvent::Item(1))
                })))
            })
        }
    }

    struct Fixture {
        client: crate::interface::session::Session,
        hang_dropped: Arc<AtomicBool>,
        endless_dropped: Arc<AtomicBool>,
        command_cancelled: Arc<AtomicBool>,
    }

    fn fixture() -> Fixture {
        let hang_dropped = Arc::new(AtomicBool::new(false));
        let endless_dropped = Arc::new(AtomicBool::new(false));
        let command_cancelled = Arc::new(AtomicBool::new(false));
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
        registry
            .register_stream(CancelAware(command_cancelled.clone()))
            .unwrap();
        let interface = registry_interface(&registry);
        let descriptor = registry_descriptor(&interface).unwrap();
        let implementation = registry_implementation(Arc::new(registry), Arc::new(()), descriptor);
        let (client, server) = pair(implementation);
        // The server session lives as long as the client keeps it connected.
        std::mem::forget(server);
        Fixture {
            client,
            hang_dropped,
            endless_dropped,
            command_cancelled,
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

        for unit_payload in [Value::Null, Value::Void] {
            let InvocationOutput::Values(values) =
                invoke(&fixture, "test.unit", vec![payload(unit_payload)])
                    .await
                    .unwrap()
            else {
                panic!("values")
            };
            assert_eq!(values, vec![Value::Void]);
        }

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
        assert_eq!(error.code, "unknown_command");

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

        let error = invoke(&fixture, "test.add", vec![]).await.err().unwrap();
        assert_eq!(error.code, "invalid_argument");

        let error = invoke(
            &fixture,
            "test.add",
            vec![payload(Value::U32(1)), payload(Value::U32(1))],
        )
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
    async fn dropping_the_output_stream_cancels_the_command() {
        let fixture = fixture();
        let InvocationOutput::Stream(mut stream) =
            invoke(&fixture, "test.cancel_aware", vec![payload(Value::Null)])
                .await
                .unwrap()
        else {
            panic!("stream")
        };
        assert!(stream.next().await.unwrap().is_ok());
        assert!(!fixture.command_cancelled.load(Ordering::SeqCst));
        drop(stream);
        wait_for(&fixture.command_cancelled).await;
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
