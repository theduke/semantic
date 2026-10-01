//! Shared demand-driven session kernel used by host transports and SDK peers.
use super::*;
use futures::{StreamExt, future::BoxFuture, stream::FuturesUnordered};
use semantic_data::value::serde::typed::TypedValue;
use semantic_rpc_core::interface_protocol::{
    InterfaceMessage as Message, SessionId, WireArgument, WireOutput,
};
use std::sync::atomic::Ordering;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot};

type Reply = oneshot::Sender<Result<InvocationOutput, InvocationError>>;
type ItemReply = oneshot::Sender<Result<StreamEvent, InvocationError>>;

/// Upper bound of the credit a consumer keeps granted, which bounds the
/// events buffered per stream.
const MAX_WINDOW: u32 = 64;

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

/// Receiving end of a peer stream.
///
/// Credit starts at one event, so an untouched stream never causes a
/// production, and the window doubles with every grant up to [`MAX_WINDOW`].
struct Consumer {
    /// Sequence of the next event to arrive; sequences start at 1.
    next_sequence: u64,
    buffered: VecDeque<Result<StreamEvent, InvocationError>>,
    /// Granted but not yet received events.
    credit: u32,
    window: u32,
    waiter: Option<ItemReply>,
    /// The terminal event has arrived.
    terminated: bool,
}

impl Consumer {
    fn new() -> Self {
        Self {
            next_sequence: 1,
            buffered: VecDeque::new(),
            credit: 0,
            window: 1,
            waiter: None,
            terminated: false,
        }
    }

    /// Whether the stream is untouched beyond what the consumer received, so
    /// handing it to the peer that produced it loses nothing.
    fn is_idle(&self) -> bool {
        self.credit == 0 && self.buffered.is_empty()
    }

    /// Credit to grant now, topping up to the window once less than half of
    /// it is left so grants are batched.
    fn grant(&mut self) -> Option<u32> {
        let outstanding = self.credit as usize + self.buffered.len();
        if self.terminated || outstanding >= self.window.div_ceil(2) as usize {
            return None;
        }
        let count = self.window - outstanding as u32;
        self.credit += count;
        self.window = (self.window * 2).min(MAX_WINDOW);
        Some(count)
    }
}

/// Sending end of a stream this peer owns.
struct Producer {
    /// Sequence of the last event sent.
    sequence: u64,
    /// `None` while a production is in flight.
    stream: Option<OwnedValueStream>,
    /// Events the consumer allows beyond those already sent.
    credit: u32,
    cancel: CancellationToken,
}

type Production = BoxFuture<
    'static,
    (
        u64,
        OwnedValueStream,
        Option<Result<StreamEvent, InvocationError>>,
    ),
>;

fn produce(id: u64, mut stream: OwnedValueStream, cancel: CancellationToken) -> Production {
    Box::pin(async move {
        let event = tokio::select! { item = stream.next() => item, _ = cancel.cancelled() => None };
        (id, stream, event)
    })
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
                credit: 0,
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
        self.consumers.insert(id.0, Consumer::new());
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
        if sequence != consumer.next_sequence {
            return Err(protocol("out-of-order stream item"));
        }
        if consumer.credit == 0 {
            return Err(protocol("unsolicited stream item"));
        }
        consumer.credit -= 1;
        consumer.next_sequence += 1;
        consumer.terminated = !matches!(item, Ok(StreamEvent::Item(_)));
        match consumer.waiter.take() {
            Some(waiter) => {
                let _ = waiter.send(item);
            }
            None => consumer.buffered.push_back(item),
        }
        if consumer.terminated && consumer.buffered.is_empty() {
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
    let mut productions: FuturesUnordered<Production> = FuturesUnordered::new();
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
                                    if let Some(origin) = stream.origin.as_ref().filter(|origin| Arc::ptr_eq(&origin.session, &state.identity) && !origin.pending.load(Ordering::SeqCst) && state.consumers.get(&origin.id).is_some_and(Consumer::is_idle)) {
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
                        if consumer.waiter.is_some() { return Err(protocol("duplicate outstanding demand")); }
                        match consumer.buffered.pop_front() {
                            Some(event) => { let _ = ready.send(event); }
                            None => consumer.waiter = Some(ready),
                        }
                        let grant = consumer.grant();
                        if consumer.terminated && consumer.buffered.is_empty() { state.consumers.remove(&id); }
                        if let Some(count) = grant { state.send(Message::StreamDemand { id: SessionId(id), count })?; }
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
                    producer.sequence += 1;
                    producer.credit -= 1;
                    let sequence = SessionId(producer.sequence);
                    let event = event.unwrap_or_else(|| Err(protocol("producer omitted terminal event")));
                    let terminal = !matches!(event, Ok(StreamEvent::Item(_)));
                    if terminal {
                        state.producers.remove(&id);
                    } else if producer.credit > 0 {
                        productions.push(produce(id, stream, producer.cancel.clone()));
                    } else {
                        producer.stream = Some(stream);
                    }
                    let message = match event {
                        Ok(StreamEvent::Item(value)) => Message::StreamItem { id: SessionId(id), sequence, value },
                        Ok(StreamEvent::End(value)) => Message::StreamEnd { id: SessionId(id), sequence, value },
                        Err(error) => Message::StreamError { id: SessionId(id), sequence, error },
                    };
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
                                if producer.credit != 0 { return Err(protocol("forward with outstanding credit")); }
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
                    Message::StreamDemand { id, count } => {
                        if id.0 > state.next_stream { return Err(protocol("demand for unissued stream")); }
                        if count == 0 { return Err(protocol("empty stream demand")); }
                        let Some(producer) = state.producers.get_mut(&id.0) else { continue };
                        producer.credit = producer.credit.checked_add(count).ok_or_else(|| protocol("stream credit overflow"))?;
                        // With a production in flight, credit is picked up when it completes.
                        if let Some(stream) = producer.stream.take() {
                            productions.push(produce(id.0, stream, producer.cancel.clone()));
                        }
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
                            if let Some(reply) = consumer.waiter { let _ = reply.send(Err(lost())); }
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
        if let Some(reply) = consumer.waiter {
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
pub(crate) mod testing {
    use super::*;

    /// Control messages seen by the relay between the two sessions.
    #[derive(Default)]
    pub(crate) struct Traffic {
        pub demands: std::sync::atomic::AtomicUsize,
        pub cancels: std::sync::atomic::AtomicUsize,
    }

    impl Traffic {
        fn observe(&self, message: &Message) {
            let counter = match message {
                Message::StreamDemand { .. } => &self.demands,
                Message::StreamCancel { .. } => &self.cancels,
                _ => return,
            };
            counter.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub(crate) fn pair(implementation: Arc<dyn InterfaceImplementation>) -> (Session, Session) {
        let (client, server, _) = pair_with_traffic(implementation);
        (client, server)
    }

    pub(crate) fn pair_with_traffic(
        implementation: Arc<dyn InterfaceImplementation>,
    ) -> (Session, Session, Arc<Traffic>) {
        let traffic = Arc::new(Traffic::default());
        let (a_tx, mut a_rx) = mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel();
        let (a_in_tx, a_in) = mpsc::unbounded_channel();
        let (b_in_tx, b_in) = mpsc::unbounded_channel();
        let observer = traffic.clone();
        tokio::spawn(async move {
            while let Some(message) = a_rx.recv().await {
                observer.observe(&message);
                if b_in_tx.send(Ok(message)).is_err() {
                    break;
                }
            }
        });
        let observer = traffic.clone();
        tokio::spawn(async move {
            while let Some(message) = b_rx.recv().await {
                observer.observe(&message);
                if a_in_tx.send(Ok(message)).is_err() {
                    break;
                }
            }
        });
        (
            Session::start(a_in, a_tx, None, vec![]),
            Session::start(b_in, b_tx, Some(implementation), vec![]),
            traffic,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{pair, pair_with_traffic};
    use super::*;
    use futures::stream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    struct Fixture {
        polls: Arc<AtomicUsize>,
        dropped: Arc<std::sync::atomic::AtomicBool>,
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
                    "many" => {
                        let polls = self.polls.clone();
                        let guard = DropFlag(self.dropped.clone());
                        let mut next = 0;
                        Ok(InvocationOutput::Stream(OwnedValueStream::new(
                            stream::poll_fn(move |_| {
                                let _ = &guard;
                                polls.fetch_add(1, Ordering::SeqCst);
                                next += 1;
                                Poll::Ready(Some(Ok(if next <= 200 {
                                    StreamEvent::Item(Value::U64(next))
                                } else {
                                    StreamEvent::End(None)
                                })))
                            }),
                        )))
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
            dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

    #[tokio::test]
    async fn stream_with_unconsumed_credit_is_reproxied() {
        let fixture = fixture();
        let (client, server) = pair(fixture.clone());
        let mut stream = stream_of(&client, "produce").await;
        // The second demand grants credit for two events, so one is buffered.
        for expected in 1..=2 {
            assert_eq!(
                stream.next().await.unwrap().unwrap(),
                StreamEvent::Item(Value::U64(expected))
            );
        }
        client
            .invoke(
                call("transfer", vec![InvocationArgument::Stream(stream)]),
                InvocationContext::default(),
            )
            .await
            .unwrap();
        let mut input = fixture.transferred.lock().unwrap().take().unwrap();
        assert!(input.metadata::<String>().is_none());
        assert_eq!(
            input.next().await.unwrap().unwrap(),
            StreamEvent::Item(Value::U64(3))
        );
        drop(input);
        client.shutdown().await.unwrap();
        drop(server);
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

    async fn stream_of(client: &Session, method: &str) -> OwnedValueStream {
        let InvocationOutput::Stream(stream) = client
            .invoke(call(method, vec![]), InvocationContext::default())
            .await
            .unwrap()
        else {
            panic!("stream")
        };
        stream
    }

    #[tokio::test]
    async fn credit_batches_demand_messages() {
        let fixture = fixture();
        let (client, server, traffic) = pair_with_traffic(fixture.clone());
        let mut stream = stream_of(&client, "many").await;
        for expected in 1..=200 {
            assert_eq!(
                stream.next().await.unwrap().unwrap(),
                StreamEvent::Item(Value::U64(expected))
            );
        }
        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            StreamEvent::End(None)
        );
        assert!(stream.next().await.is_none());
        let demands = traffic.demands.load(Ordering::SeqCst);
        assert!(demands <= 12, "{demands} demand messages for 200 items");
        // Credit never lets the producer run more than the window ahead.
        assert!(fixture.polls.load(Ordering::SeqCst) <= 201 + MAX_WINDOW as usize);
        drop(server);
    }

    #[tokio::test]
    async fn cancel_with_outstanding_credit_drops_the_producer() {
        let fixture = fixture();
        let (client, server, traffic) = pair_with_traffic(fixture.clone());
        let mut stream = stream_of(&client, "many").await;
        for expected in 1..=3 {
            assert_eq!(
                stream.next().await.unwrap().unwrap(),
                StreamEvent::Item(Value::U64(expected))
            );
        }
        drop(stream);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !fixture.dropped.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("producer was not dropped");
        assert_eq!(traffic.cancels.load(Ordering::SeqCst), 1);
        let polls = fixture.polls.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(fixture.polls.load(Ordering::SeqCst), polls);
        drop(server);
    }

    /// A session wired to channels the test drives by hand.
    struct Wire {
        incoming: mpsc::UnboundedSender<Result<Message, InvocationError>>,
        outgoing: mpsc::UnboundedReceiver<Message>,
        session: Session,
    }

    fn wire(implementation: Option<Arc<dyn InterfaceImplementation>>) -> Wire {
        let (incoming, incoming_rx) = mpsc::unbounded_channel();
        let (outgoing_tx, outgoing) = mpsc::unbounded_channel();
        let session = Session::start(incoming_rx, outgoing_tx, implementation, vec![]);
        Wire {
            incoming,
            outgoing,
            session,
        }
    }

    async fn closes(session: &Session) {
        tokio::time::timeout(std::time::Duration::from_secs(5), session.closed())
            .await
            .expect("session should close");
    }

    #[tokio::test]
    async fn zero_credit_demand_closes_session() {
        let mut wire = wire(Some(fixture()));
        wire.incoming
            .send(Ok(Message::Call {
                id: SessionId(1),
                export: "test".into(),
                method: "produce".into(),
                arguments: vec![],
            }))
            .unwrap();
        let Some(Message::Return {
            output: WireOutput::Stream(id),
            ..
        }) = wire.outgoing.recv().await
        else {
            panic!("expected stream return")
        };
        wire.incoming
            .send(Ok(Message::StreamDemand { id, count: 0 }))
            .unwrap();
        closes(&wire.session).await;
    }

    #[tokio::test]
    async fn items_beyond_credit_close_session() {
        let mut wire = wire(None);
        let session = wire.session.clone();
        let call_task = tokio::spawn(async move {
            session
                .invoke(call("produce", vec![]), InvocationContext::default())
                .await
        });
        let Some(Message::Call { id: call_id, .. }) = wire.outgoing.recv().await else {
            panic!("expected call")
        };
        wire.incoming
            .send(Ok(Message::Return {
                id: call_id,
                output: WireOutput::Stream(SessionId(1)),
            }))
            .unwrap();
        let InvocationOutput::Stream(mut stream) = call_task.await.unwrap().unwrap() else {
            panic!("stream")
        };
        let next = tokio::spawn(async move { stream.next().await });
        let Some(Message::StreamDemand { id, count: 1 }) = wire.outgoing.recv().await else {
            panic!("expected initial demand of one")
        };
        for sequence in [1, 2] {
            wire.incoming
                .send(Ok(Message::StreamItem {
                    id: id.clone(),
                    sequence: SessionId(sequence),
                    value: Value::Null,
                }))
                .unwrap();
        }
        assert_eq!(
            next.await.unwrap().unwrap().unwrap(),
            StreamEvent::Item(Value::Null)
        );
        closes(&wire.session).await;
    }
}
