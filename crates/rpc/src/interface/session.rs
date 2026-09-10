//! Shared demand-driven session kernel used by host transports and SDK peers.
use super::*;
use futures::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use semantic_data::value::serde::typed::TypedValue;
use semantic_rpc_core::interface_protocol::{
    InterfaceMessage as Message, SessionId, WireArgument, WireOutput,
};
use std::sync::atomic::Ordering;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{mpsc, oneshot};

type Reply = oneshot::Sender<Result<InvocationOutput, InvocationError>>;
type ItemReply = oneshot::Sender<Result<StreamEvent, InvocationError>>;

enum Action {
    Call(ValidatedInvocation, InvocationContext, Reply),
    Demand(u64, ItemReply),
    DropStream(u64),
    Stop(oneshot::Sender<()>),
}

/// Cloneable session endpoint. The transport must close incoming on any I/O failure.
#[derive(Clone)]
pub struct Session {
    actions: mpsc::UnboundedSender<Action>,
    descriptors: Vec<ImplementationDescriptor>,
    closed: CancellationToken,
}

impl Session {
    pub fn start(
        incoming: mpsc::UnboundedReceiver<Result<Message, InvocationError>>,
        outgoing: mpsc::UnboundedSender<Message>,
        implementation: Option<Arc<dyn InterfaceImplementation>>,
        descriptors: Vec<ImplementationDescriptor>,
    ) -> Self {
        let (actions, receiver) = mpsc::unbounded_channel();
        let closed = CancellationToken::new();
        let session = Self {
            actions: actions.clone(),
            descriptors,
            closed: closed.clone(),
        };
        let weak_actions = actions.downgrade();
        spawn(async move {
            run(incoming, outgoing, receiver, weak_actions, implementation).await;
            closed.cancel();
        });
        session
    }

    pub async fn closed(&self) {
        self.closed.cancelled().await;
    }

    pub async fn shutdown(&self) -> Result<(), InvocationError> {
        let (tx, rx) = oneshot::channel();
        // A disconnected session is already stopped. Always await local cleanup,
        // including when the actor closes between sending Stop and receiving it.
        let _ = self.actions.send(Action::Stop(tx));
        let _ = rx.await;
        self.closed().await;
        Ok(())
    }
}

fn spawn(future: impl Future<Output = ()> + Send + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::spawn(future);
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(future);
}

impl InterfaceImplementation for Session {
    fn descriptors(&self) -> &[ImplementationDescriptor] {
        &self.descriptors
    }

    fn invoke<'a>(
        &'a self,
        call: ValidatedInvocation,
        mut context: InvocationContext,
    ) -> InvocationFuture<'a> {
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            let cancellation = context.cancellation.clone();
            context.cancellation = context.cancellation.child_token();
            let mut guard = CallGuard(Some(context.cancellation.clone()));
            self.actions
                .send(Action::Call(call, context, tx))
                .map_err(|_| lost())?;
            let output = rx.await.map_err(|_| lost())??;
            guard.0 = None;
            match output {
                InvocationOutput::Stream(stream) => Ok(InvocationOutput::Stream(
                    stream.with_cancellation(cancellation),
                )),
                output => Ok(output),
            }
        })
    }
}

struct CallGuard(Option<CancellationToken>);
impl Drop for CallGuard {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}

fn lost() -> InvocationError {
    InvocationError::new("connection_lost", "interface session closed")
}
fn protocol(message: &str) -> InvocationError {
    InvocationError::new("protocol_violation", message)
}

struct RemoteStream {
    id: u64,
    actions: mpsc::UnboundedSender<Action>,
    pending: Option<oneshot::Receiver<Result<StreamEvent, InvocationError>>>,
    origin: StreamOrigin,
}

impl Stream for RemoteStream {
    type Item = Result<StreamEvent, InvocationError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.origin.pending.store(true, Ordering::SeqCst);
        if self.pending.is_none() {
            let (tx, rx) = oneshot::channel();
            if self.actions.send(Action::Demand(self.id, tx)).is_err() {
                return Poll::Ready(Some(Err(lost())));
            }
            self.pending = Some(rx);
        }
        match Pin::new(self.pending.as_mut().unwrap()).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.pending = None;
                self.origin.pending.store(false, Ordering::SeqCst);
                Poll::Ready(Some(result.unwrap_or_else(|_| Err(lost()))))
            }
        }
    }
}
impl Drop for RemoteStream {
    fn drop(&mut self) {
        if !self.origin.forwarded.load(Ordering::SeqCst) {
            let _ = self.actions.send(Action::DropStream(self.id));
        }
    }
}

struct Consumer {
    sequence: u64,
    ready: Option<ItemReply>,
}
struct Producer {
    sequence: u64,
    stream: Option<OwnedValueStream>,
    cancel: CancellationToken,
}

struct State {
    identity: Arc<()>,
    next_call: u64,
    next_stream: u64,
    peer_call: u64,
    peer_stream: u64,
    calls: BTreeMap<u64, Reply>,
    completed_calls: BTreeMap<u64, CancellationToken>,
    call_cancel: BTreeMap<u64, CancellationToken>,
    consumers: BTreeMap<u64, Consumer>,
    producers: BTreeMap<u64, Producer>,
    actions: mpsc::WeakUnboundedSender<Action>,
    outgoing: mpsc::UnboundedSender<Message>,
}

impl State {
    fn send(&self, message: Message) -> Result<(), InvocationError> {
        self.outgoing.send(message).map_err(|_| lost())
    }
    fn producer(&mut self, stream: OwnedValueStream) -> Result<SessionId, InvocationError> {
        self.next_stream = self
            .next_stream
            .checked_add(1)
            .ok_or_else(|| protocol("stream IDs exhausted"))?;
        self.producers.insert(
            self.next_stream,
            Producer {
                sequence: 0,
                stream: Some(stream),
                cancel: CancellationToken::new(),
            },
        );
        Ok(SessionId(self.next_stream))
    }
    fn consumer(&mut self, id: SessionId) -> Result<OwnedValueStream, InvocationError> {
        if self.peer_stream.checked_add(1) != Some(id.0) {
            return Err(protocol("stream reference was reused or skipped"));
        }
        self.peer_stream = id.0;
        self.consumers.insert(
            id.0,
            Consumer {
                sequence: 0,
                ready: None,
            },
        );
        let origin = StreamOrigin {
            session: self.identity.clone(),
            id: id.0,
            pending: Arc::new(AtomicBool::new(false)),
            forwarded: Arc::new(AtomicBool::new(false)),
        };
        let mut stream = OwnedValueStream::new(RemoteStream {
            id: id.0,
            actions: self.actions.upgrade().ok_or_else(lost)?,
            pending: None,
            origin: origin.clone(),
        });
        stream.origin = Some(origin);
        Ok(stream)
    }
    fn output(
        &mut self,
        output: Result<InvocationOutput, InvocationError>,
    ) -> Result<WireOutput, InvocationError> {
        Ok(match output {
            Ok(InvocationOutput::Values(values)) => {
                WireOutput::Values(values.into_iter().map(TypedValue).collect())
            }
            Ok(InvocationOutput::Stream(stream)) => WireOutput::Stream(self.producer(stream)?),
            Err(error) => WireOutput::Error(error),
        })
    }
    fn item(
        &mut self,
        id: u64,
        sequence: u64,
        item: Result<StreamEvent, InvocationError>,
    ) -> Result<(), InvocationError> {
        if id > self.peer_stream {
            return Err(protocol("unknown stream"));
        }
        let Some(consumer) = self.consumers.get_mut(&id) else {
            return Ok(());
        };
        if sequence != consumer.sequence {
            return Err(protocol("out-of-order stream item"));
        }
        let ready = consumer
            .ready
            .take()
            .ok_or_else(|| protocol("unsolicited stream item"))?;
        let terminal = !matches!(item, Ok(StreamEvent::Item(_)));
        let _ = ready.send(item);
        if terminal {
            self.consumers.remove(&id);
        }
        Ok(())
    }
}

async fn run(
    mut incoming: mpsc::UnboundedReceiver<Result<Message, InvocationError>>,
    outgoing: mpsc::UnboundedSender<Message>,
    mut actions: mpsc::UnboundedReceiver<Action>,
    action_tx: mpsc::WeakUnboundedSender<Action>,
    implementation: Option<Arc<dyn InterfaceImplementation>>,
) {
    let mut state = State {
        identity: Arc::new(()),
        next_call: 0,
        next_stream: 0,
        peer_call: 0,
        peer_stream: 0,
        calls: BTreeMap::new(),
        completed_calls: BTreeMap::new(),
        call_cancel: BTreeMap::new(),
        consumers: BTreeMap::new(),
        producers: BTreeMap::new(),
        actions: action_tx,
        outgoing,
    };
    let mut invocations: FuturesUnordered<
        BoxFuture<'static, (u64, Result<InvocationOutput, InvocationError>)>,
    > = FuturesUnordered::new();
    let mut productions: FuturesUnordered<
        BoxFuture<
            'static,
            (
                u64,
                OwnedValueStream,
                Option<Result<StreamEvent, InvocationError>>,
            ),
        >,
    > = FuturesUnordered::new();
    let mut cancellations: FuturesUnordered<BoxFuture<'static, Option<u64>>> =
        FuturesUnordered::new();
    let mut stop = None;
    let mut peer_stopping = false;
    let result: Result<(), InvocationError> = async {
        loop {
            if peer_stopping && invocations.is_empty() && productions.is_empty() {
                state.send(Message::ShutdownAck)?;
                break;
            }
            tokio::select! {
                action = actions.recv() => match action.ok_or_else(lost)? {
                    Action::Call(call, context, reply) => {
                        if stop.is_some() { let _ = reply.send(Err(lost())); continue; }
                        if context.cancellation.is_cancelled() { let _ = reply.send(Err(InvocationError::new("cancelled", "call cancelled"))); continue; }
                        state.next_call = state.next_call.checked_add(1).ok_or_else(|| protocol("call IDs exhausted"))?;
                        let id = state.next_call;
                        let mut arguments = Vec::new();
                        for argument in call.arguments {
                            arguments.push(match argument {
                                InvocationArgument::Value(value) => WireArgument::Value(TypedValue(value)),
                                InvocationArgument::Stream(stream) => {
                                    if let Some(origin) = stream.origin.as_ref().filter(|origin| Arc::ptr_eq(&origin.session, &state.identity) && !origin.pending.load(Ordering::SeqCst)) {
                                        origin.forwarded.store(true, Ordering::SeqCst);
                                        state.consumers.remove(&origin.id);
                                        WireArgument::ForwardStream(SessionId(origin.id))
                                    } else { WireArgument::Stream(state.producer(stream)?) }
                                }
                            });
                        }
                        state.calls.insert(id, reply);
                        let completed = CancellationToken::new();
                        state.completed_calls.insert(id, completed.clone());
                        cancellations.push(Box::pin(async move { tokio::select! { _ = context.cancellation.cancelled() => Some(id), _ = completed.cancelled() => None } }));
                        state.send(Message::Call { id: SessionId(id), export: call.export, method: call.method, arguments })?;
                    }
                    Action::Demand(id, ready) => {
                        let Some(consumer) = state.consumers.get_mut(&id) else { let _ = ready.send(Err(lost())); continue };
                        if consumer.ready.is_some() { return Err(protocol("duplicate outstanding demand")); }
                        consumer.sequence = consumer.sequence.checked_add(1).ok_or_else(|| protocol("stream sequence exhausted"))?;
                        let sequence = consumer.sequence;
                        consumer.ready = Some(ready);
                        state.send(Message::StreamDemand { id: SessionId(id), sequence: SessionId(sequence) })?;
                    }
                    Action::DropStream(id) => { if state.consumers.remove(&id).is_some() { state.send(Message::StreamCancel { id: SessionId(id) })?; } }
                    Action::Stop(reply) => { stop = Some(reply); state.send(Message::Shutdown)?; }
                },
                Some(cancelled) = cancellations.next(), if !cancellations.is_empty() => {
                    let Some(id) = cancelled else { continue };
                    state.completed_calls.remove(&id);
                    if let Some(reply) = state.calls.remove(&id) {
                        let _ = reply.send(Err(InvocationError::new("cancelled", "call cancelled")));
                        state.send(Message::CancelCall { id: SessionId(id) })?;
                    }
                }
                Some((id, output)) = invocations.next(), if !invocations.is_empty() => {
                    state.call_cancel.remove(&id);
                    let output = state.output(output)?;
                    state.send(Message::Return { id: SessionId(id), output })?;
                }
                Some((id, stream, event)) = productions.next(), if !productions.is_empty() => {
                    let Some(producer) = state.producers.get_mut(&id) else { continue };
                    let sequence = SessionId(producer.sequence);
                    let event = event.unwrap_or_else(|| Err(protocol("producer omitted terminal event")));
                    let terminal = !matches!(event, Ok(StreamEvent::Item(_)));
                    producer.stream = Some(stream);
                    let message = match event {
                        Ok(StreamEvent::Item(value)) => Message::StreamItem { id: SessionId(id), sequence, value },
                        Ok(StreamEvent::End(value)) => Message::StreamEnd { id: SessionId(id), sequence, value },
                        Err(error) => Message::StreamError { id: SessionId(id), sequence, error },
                    };
                    if terminal { state.producers.remove(&id); }
                    state.send(message)?;
                }
                message = incoming.recv() => match message.ok_or_else(lost)?? {
                    Message::Call { id, export, method, arguments } => {
                        if state.peer_call.checked_add(1) != Some(id.0) { return Err(protocol("call ID reused or skipped")); }
                        state.peer_call = id.0;
                        if peer_stopping {
                            state.send(Message::Return { id, output: WireOutput::Error(InvocationError::new("provider_unavailable", "session is stopping")) })?;
                            continue;
                        }
                        let mut args = Vec::new();
                        for arg in arguments { args.push(match arg {
                            WireArgument::Value(value) => InvocationArgument::Value(value.0),
                            WireArgument::Stream(id) => InvocationArgument::Stream(state.consumer(id)?),
                            WireArgument::ForwardStream(id) => {
                                let producer = state.producers.remove(&id.0).ok_or_else(|| protocol("forward of unknown or retired stream"))?;
                                InvocationArgument::Stream(producer.stream.ok_or_else(|| protocol("forward during outstanding demand"))?)
                            }
                        }); }
                        let implementation = implementation.clone().ok_or_else(|| protocol("peer does not accept calls"))?;
                        let context = InvocationContext::default();
                        state.call_cancel.insert(id.0, context.cancellation.clone());
                        invocations.push(Box::pin(async move { (id.0, implementation.invoke(ValidatedInvocation { export, method, arguments: args }, context).await) }));
                    }
                    Message::Return { id, output } => {
                        if id.0 > state.next_call { return Err(protocol("return for unissued call")); }
                        if let Some(completed) = state.completed_calls.remove(&id.0) { completed.cancel(); }
                        // Retired calls may still return an owned stream. Register and
                        // drop it so its producer is cancelled and IDs stay synchronized.
                        let output = match output { WireOutput::Values(values) => Ok(InvocationOutput::Values(values.into_iter().map(|value| value.0).collect())), WireOutput::Stream(id) => Ok(InvocationOutput::Stream(state.consumer(id)?)), WireOutput::Error(error) => Err(error) };
                        if let Some(reply) = state.calls.remove(&id.0) {
                            let _ = reply.send(output);
                        }
                    }
                    Message::CancelCall { id } => {
                        if id.0 > state.peer_call { return Err(protocol("cancel for unissued call")); }
                        if let Some(token) = state.call_cancel.get(&id.0) { token.cancel(); }
                    }
                    Message::StreamDemand { id, sequence } => {
                        if id.0 > state.next_stream { return Err(protocol("demand for unissued stream")); }
                        let Some(producer) = state.producers.get_mut(&id.0) else { continue };
                        if producer.sequence.checked_add(1) != Some(sequence.0) { return Err(protocol("invalid demand sequence")); }
                        let mut stream = producer.stream.take().ok_or_else(|| protocol("demand before prior handoff"))?;
                        producer.sequence = sequence.0;
                        let cancel = producer.cancel.clone();
                        productions.push(Box::pin(async move {
                            let event = tokio::select! { item = stream.next() => item, _ = cancel.cancelled() => None };
                            (id.0, stream, event)
                        }));
                    }
                    Message::StreamItem { id, sequence, value } => state.item(id.0, sequence.0, Ok(StreamEvent::Item(value)))?,
                    Message::StreamEnd { id, sequence, value } => state.item(id.0, sequence.0, Ok(StreamEvent::End(value)))?,
                    Message::StreamError { id, sequence, error } => state.item(id.0, sequence.0, Err(error))?,
                    Message::StreamCancel { id } => {
                        if id.0 > state.next_stream { return Err(protocol("cancel for unissued stream")); }
                        if let Some(producer) = state.producers.remove(&id.0) { producer.cancel.cancel(); }
                    }
                    Message::Shutdown => {
                        peer_stopping = true;
                        for token in state.call_cancel.values() { token.cancel(); }
                        for producer in state.producers.values() { producer.cancel.cancel(); }
                        state.producers.clear();
                        for (_, consumer) in std::mem::take(&mut state.consumers) {
                            if let Some(reply) = consumer.ready { let _ = reply.send(Err(lost())); }
                        }
                    }
                    Message::ShutdownAck if stop.is_some() => break,
                    _ => return Err(protocol("unexpected session control message")),
                }
            }
        }
        Ok(())
    }.await;
    let error = result.err().unwrap_or_else(lost);
    for (_, reply) in state.calls {
        let _ = reply.send(Err(error.clone()));
    }
    for (_, consumer) in state.consumers {
        if let Some(reply) = consumer.ready {
            let _ = reply.send(Err(error.clone()));
        }
    }
    for (_, token) in state.call_cancel {
        token.cancel();
    }
    actions.close();
    while actions.try_recv().is_ok() {}
    // Cooperatively admitted application work owns its completion. Disconnect
    // signals cancellation and failed inputs but never drops an admitted write.
    while invocations.next().await.is_some() {}
    if let Some(reply) = stop {
        let _ = reply.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture {
        polls: Arc<AtomicUsize>,
        transferred: std::sync::Mutex<Option<OwnedValueStream>>,
    }
    impl InterfaceImplementation for Fixture {
        fn descriptors(&self) -> &[ImplementationDescriptor] {
            &[]
        }
        fn invoke<'a>(
            &'a self,
            mut call: ValidatedInvocation,
            _: InvocationContext,
        ) -> InvocationFuture<'a> {
            Box::pin(async move {
                match call.method.as_str() {
                    "values" => Ok(InvocationOutput::Values(vec![Value::U64(u64::MAX)])),
                    "produce" => {
                        let polls = self.polls.clone();
                        let mut next = 0;
                        Ok(InvocationOutput::Stream(
                            OwnedValueStream::new(stream::poll_fn(move |_| {
                                polls.fetch_add(1, Ordering::SeqCst);
                                next += 1;
                                Poll::Ready(Some(Ok(if next < 4 {
                                    StreamEvent::Item(Value::U64(next))
                                } else {
                                    StreamEvent::End(Some(Value::U64(3)))
                                })))
                            }))
                            .with_metadata("trusted origin".to_owned()),
                        ))
                    }
                    "transfer" => {
                        let InvocationArgument::Stream(input) = call.arguments.remove(0) else {
                            panic!("stream input")
                        };
                        *self.transferred.lock().unwrap() = Some(input);
                        Ok(InvocationOutput::Values(vec![Value::U64(42)]))
                    }
                    "transform" => {
                        let InvocationArgument::Stream(input) = call.arguments.remove(0) else {
                            panic!("stream input")
                        };
                        Ok(InvocationOutput::Stream(input))
                    }
                    _ => Err(InvocationError::new("invalid_argument", "unknown method")),
                }
            })
        }
    }

    fn pair(implementation: Arc<dyn InterfaceImplementation>) -> (Session, Session) {
        let (a_tx, mut a_rx) = mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel();
        let (a_in_tx, a_in) = mpsc::unbounded_channel();
        let (b_in_tx, b_in) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(message) = a_rx.recv().await {
                if b_in_tx.send(Ok(message)).is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some(message) = b_rx.recv().await {
                if a_in_tx.send(Ok(message)).is_err() {
                    break;
                }
            }
        });
        (
            Session::start(a_in, a_tx, None, vec![]),
            Session::start(b_in, b_tx, Some(implementation), vec![]),
        )
    }
    fn call(method: &str, arguments: Vec<InvocationArgument>) -> ValidatedInvocation {
        ValidatedInvocation {
            export: "test".into(),
            method: method.into(),
            arguments,
        }
    }
    fn fixture() -> Arc<Fixture> {
        Arc::new(Fixture {
            polls: Arc::new(AtomicUsize::new(0)),
            transferred: std::sync::Mutex::new(None),
        })
    }

    #[tokio::test]
    async fn demand_duplex_and_terminal_values() {
        let fixture = fixture();
        let (client, server) = pair(fixture.clone());
        let InvocationOutput::Stream(mut output) = client
            .invoke(call("produce", vec![]), InvocationContext::default())
            .await
            .unwrap()
        else {
            panic!("stream")
        };
        assert_eq!(
            fixture.polls.load(Ordering::SeqCst),
            0,
            "must not poll without consumer demand"
        );
        assert_eq!(
            output.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::U64(1))
        );
        assert_eq!(fixture.polls.load(Ordering::SeqCst), 1);
        // A blocked consumer cannot prevent other control/call traffic.
        let InvocationOutput::Values(values) = client
            .invoke(call("values", vec![]), InvocationContext::default())
            .await
            .unwrap()
        else {
            panic!("values")
        };
        assert_eq!(values, vec![Value::U64(u64::MAX)]);
        assert_eq!(
            output.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::U64(2))
        );
        assert_eq!(
            output.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::U64(3))
        );
        assert_eq!(
            output.next().await.unwrap().unwrap(),
            StreamEvent::End(Some(Value::U64(3)))
        );
        assert!(output.next().await.is_none());
        let input = OwnedValueStream::new(stream::iter([
            Ok(StreamEvent::Item(Value::Bool(true))),
            Ok(StreamEvent::End(None)),
        ]));
        let InvocationOutput::Stream(mut output) = client
            .invoke(
                call("transform", vec![InvocationArgument::Stream(input)]),
                InvocationContext::default(),
            )
            .await
            .unwrap()
        else {
            panic!("stream")
        };
        assert_eq!(
            output.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::Bool(true))
        );
        assert_eq!(
            output.next().await.unwrap().unwrap(),
            StreamEvent::End(None)
        );
        client.shutdown().await.unwrap();
        drop(server);
    }

    #[tokio::test]
    async fn transferred_input_outlives_return_and_disconnect_fails() {
        let fixture = fixture();
        let (client, server) = pair(fixture.clone());
        let input = OwnedValueStream::new(stream::iter([
            Ok(StreamEvent::Item(Value::Bool(true))),
            Ok(StreamEvent::End(None)),
        ]));
        client
            .invoke(
                call("transfer", vec![InvocationArgument::Stream(input)]),
                InvocationContext::default(),
            )
            .await
            .unwrap();
        let mut transferred = fixture.transferred.lock().unwrap().take().unwrap();
        assert_eq!(
            transferred.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::Bool(true))
        );
        assert_eq!(
            transferred.next().await.unwrap().unwrap(),
            StreamEvent::End(None)
        );
        let input = OwnedValueStream::new(stream::pending());
        client
            .invoke(
                call("transfer", vec![InvocationArgument::Stream(input)]),
                InvocationContext::default(),
            )
            .await
            .unwrap();
        let mut transferred = fixture.transferred.lock().unwrap().take().unwrap();
        client.shutdown().await.unwrap();
        assert_eq!(
            transferred.next().await.unwrap().unwrap_err().code,
            "connection_lost"
        );
        drop(server);
    }

    #[tokio::test]
    async fn unsolicited_data_closes_session() {
        let (incoming_tx, incoming) = mpsc::unbounded_channel();
        let (outgoing, _outgoing_rx) = mpsc::unbounded_channel();
        let session = Session::start(incoming, outgoing, None, vec![]);
        incoming_tx
            .send(Ok(Message::StreamItem {
                id: SessionId(99),
                sequence: SessionId(1),
                value: Value::Null,
            }))
            .unwrap();
        tokio::task::yield_now().await;
        assert!(
            session
                .invoke(call("values", vec![]), InvocationContext::default())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unconsumed_forward_recovers_only_host_metadata() {
        forward_recovers_only_host_metadata(false).await;
        forward_recovers_only_host_metadata(true).await;
    }

    async fn forward_recovers_only_host_metadata(consume_first: bool) {
        let fixture = fixture();
        let (client, server) = pair(fixture.clone());
        let InvocationOutput::Stream(mut stream) = client
            .invoke(call("produce", vec![]), InvocationContext::default())
            .await
            .unwrap()
        else {
            panic!("stream")
        };
        assert!(
            stream.metadata::<String>().is_none(),
            "metadata must not cross wire"
        );
        if consume_first {
            assert_eq!(
                stream.next().await.unwrap().unwrap(),
                StreamEvent::Item(Value::U64(1))
            );
        }
        // Client-authored metadata cannot replace authoritative host metadata.
        let stream = stream.with_metadata("forged".to_owned());
        client
            .invoke(
                call("transfer", vec![InvocationArgument::Stream(stream)]),
                InvocationContext::default(),
            )
            .await
            .unwrap();
        let mut input = fixture.transferred.lock().unwrap().take().unwrap();
        assert_eq!(input.metadata::<String>().unwrap(), "trusted origin");
        assert_eq!(
            fixture.polls.load(Ordering::SeqCst),
            usize::from(consume_first)
        );
        assert_eq!(
            input.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::U64(if consume_first { 2 } else { 1 }))
        );
        drop(input);
        client.shutdown().await.unwrap();
        client.shutdown().await.unwrap();
        server.shutdown().await.unwrap();
        drop(server);
    }
}
