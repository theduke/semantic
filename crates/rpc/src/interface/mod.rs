//! Owned native interface invocation, independent of the legacy unary registry.
#[cfg(any(feature = "client-http-native", feature = "client-http-web"))]
pub(crate) mod client;
#[cfg(feature = "interface-session")]
pub mod session;
mod validation;
use futures::Stream;
use semantic_data::value::Value;
pub use semantic_rpc_core::interface::{ImplementationDescriptor, InterfaceRef, InvocationError};
use std::{
    any::Any,
    sync::{Arc, atomic::AtomicBool},
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
pub use tokio_util::sync::CancellationToken;
pub use validation::{ConformingImplementation, validate_configuration};

pub type InvocationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<InvocationOutput, InvocationError>> + Send + 'a>>;

pub trait InterfaceImplementation: Send + Sync + 'static {
    fn descriptors(&self) -> &[ImplementationDescriptor];
    fn invoke<'a>(
        &'a self,
        call: ValidatedInvocation,
        context: InvocationContext,
    ) -> InvocationFuture<'a>;
}

#[derive(Clone, Debug, Default)]
pub struct InvocationContext {
    pub generation: u64,
    pub cancellation: CancellationToken,
}

pub struct ValidatedInvocation {
    pub export: String,
    pub method: String,
    pub arguments: Vec<InvocationArgument>,
}

pub enum InvocationArgument {
    Value(Value),
    Stream(OwnedValueStream),
}

pub enum InvocationOutput {
    Values(Vec<Value>),
    Stream(OwnedValueStream),
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    Item(Value),
    End(Option<Value>),
}

/// A single consumer stream. Dropping it drops its producer immediately.
/// Missing explicit terminal events are reported as failures, never successful EOF.
pub struct OwnedValueStream {
    inner: Option<Pin<Box<dyn Stream<Item = Result<StreamEvent, InvocationError>> + Send>>>,
    pub(crate) metadata: Option<Arc<dyn Any + Send + Sync>>,
    pub(crate) origin: Option<StreamOrigin>,
}

#[derive(Clone)]
#[cfg_attr(not(feature = "interface-session"), allow(dead_code))]
pub(crate) struct StreamOrigin {
    pub session: Arc<()>,
    pub id: u64,
    pub pending: Arc<AtomicBool>,
    pub forwarded: Arc<AtomicBool>,
}

impl OwnedValueStream {
    pub fn new(
        stream: impl Stream<Item = Result<StreamEvent, InvocationError>> + Send + 'static,
    ) -> Self {
        Self {
            inner: Some(Box::pin(stream)),
            metadata: None,
            origin: None,
        }
    }

    /// Trusted local metadata is never serialized into protocol messages.
    pub fn with_metadata<T: Any + Send + Sync>(mut self, metadata: T) -> Self {
        self.metadata = Some(Arc::new(metadata));
        self
    }
    pub fn metadata<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.metadata.as_deref()?.downcast_ref()
    }

    pub fn with_cancellation(self, cancellation: CancellationToken) -> Self {
        let metadata = self.metadata.clone();
        let origin = self.origin.clone();
        let mut stream = Self::new(futures::stream::select(
            self,
            futures::stream::once(async move {
                cancellation.cancelled().await;
                Err(InvocationError::new("cancelled", "stream cancelled"))
            }),
        ));
        stream.metadata = metadata;
        stream.origin = origin;
        stream
    }
}

impl Stream for OwnedValueStream {
    type Item = Result<StreamEvent, InvocationError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(inner) = &mut self.inner else {
            return Poll::Ready(None);
        };
        let result = match inner.as_mut().poll_next(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(None) => Err(InvocationError::new(
                "invalid_output",
                "stream ended without a terminal event",
            )),
            Poll::Ready(Some(event)) => event,
        };
        if !matches!(result, Ok(StreamEvent::Item(_))) {
            self.inner = None;
        }
        Poll::Ready(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, executor::block_on, stream};

    #[test]
    fn explicit_terminal_and_truncation() {
        block_on(async {
            let mut stream = OwnedValueStream::new(stream::iter([
                Ok(StreamEvent::End(None)),
                Ok(StreamEvent::Item(Value::Null)),
            ]));
            assert_eq!(stream.next().await, Some(Ok(StreamEvent::End(None))));
            assert!(stream.next().await.is_none());
            let mut stream = OwnedValueStream::new(stream::empty());
            assert_eq!(
                stream.next().await.unwrap().unwrap_err().code,
                "invalid_output"
            );
            assert!(stream.next().await.is_none());
        });
    }
}
