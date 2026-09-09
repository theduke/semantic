use std::future::Future;
use std::pin::Pin;

use semantic_data::schema::FunctionType;
use semantic_data::value::Value;

use crate::convert::{RpcDecode, RpcEncode};
use crate::error::RpcError;
use crate::protocol::RpcResult;

pub trait RpcCommandSpec {
    type Payload: RpcEncode + RpcDecode + Send + 'static;
    type Output: RpcEncode + RpcDecode + Send + 'static;
    type Error: Into<RpcError> + Send + 'static;

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

pub trait DynCommand<Ctx>: Send + Sync {
    fn name(&self) -> &str;

    fn signature(&self) -> &FunctionType;

    fn call_value<'a>(
        &'a self,
        ctx: &'a Ctx,
        payload: Value,
    ) -> Pin<Box<dyn Future<Output = RpcResult> + Send + 'a>>;
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

impl<Ctx, C> DynCommand<Ctx> for CommandAdapter<C>
where
    C: RpcCommand<Ctx>,
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
    ) -> Pin<Box<dyn Future<Output = RpcResult> + Send + 'a>> {
        Box::pin(async move {
            let payload = match C::Payload::decode_rpc(payload) {
                Ok(payload) => payload,
                Err(err) => return RpcResult::Err(err),
            };

            match self.command.call(ctx, payload).await {
                Ok(output) => match output.encode_rpc() {
                    Ok(value) => RpcResult::Ok(value),
                    Err(err) => RpcResult::Err(err),
                },
                Err(err) => RpcResult::Err(err.into()),
            }
        })
    }
}
