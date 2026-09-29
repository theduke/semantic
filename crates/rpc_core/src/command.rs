use std::future::Future;
use std::pin::Pin;

use semantic_data::schema::FunctionType;
use semantic_data::value::Value;

use crate::convert::{RpcDecode, RpcEncode};
use crate::error::RpcError;

pub trait RpcCommandSpec {
    type Payload: RpcEncode + RpcDecode + Send + 'static;
    type Output: RpcEncode + RpcDecode + Send + 'static;
    type Error: Send + 'static;

    const NAME: &'static str;

    fn signature(&self) -> FunctionType;
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
    Command(E),
}

impl<E: Into<RpcError>> From<CallError<E>> for RpcError {
    fn from(err: CallError<E>) -> Self {
        match err {
            CallError::UnknownCommand(command) => RpcError::unknown_command(command),
            CallError::InvalidPayload(err) | CallError::InvalidOutput(err) => err,
            CallError::Command(err) => err.into(),
        }
    }
}

pub trait DynCommand<Ctx, E>: Send + Sync {
    fn name(&self) -> &str;

    fn signature(&self) -> &FunctionType;

    fn call_value<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError<E>>> + Send + 'a>>;
}

pub struct CommandAdapter<C> {
    command: C,
    signature: FunctionType,
}

impl<C> CommandAdapter<C>
where
    C: RpcCommandSpec,
{
    pub fn new(command: C) -> Self {
        let signature = command.signature();
        Self { command, signature }
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

    fn signature(&self) -> &FunctionType {
        &self.signature
    }

    fn call_value<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CallError<E>>> + Send + 'a>> {
        Box::pin(async move {
            let payload = C::Payload::decode_rpc(payload).map_err(CallError::InvalidPayload)?;
            let output = self
                .command
                .call(ctx, payload)
                .await
                .map_err(|err| CallError::Command(err.into()))?;
            output.encode_rpc().map_err(CallError::InvalidOutput)
        })
    }
}
