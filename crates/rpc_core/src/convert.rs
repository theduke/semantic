use semantic_data::value::Value;

use crate::error::RpcError;

pub trait RpcEncode {
    fn encode_rpc(self) -> Result<Value, RpcError>;
}

pub trait RpcDecode: Sized {
    fn decode_rpc(value: Value) -> Result<Self, RpcError>;
}

impl RpcEncode for Value {
    fn encode_rpc(self) -> Result<Value, RpcError> {
        Ok(self)
    }
}

impl RpcDecode for Value {
    fn decode_rpc(value: Value) -> Result<Self, RpcError> {
        Ok(value)
    }
}

impl RpcEncode for () {
    fn encode_rpc(self) -> Result<Value, RpcError> {
        Ok(Value::Void)
    }
}

impl RpcDecode for () {
    fn decode_rpc(value: Value) -> Result<Self, RpcError> {
        if matches!(value, Value::Void | Value::Null) {
            Ok(())
        } else {
            Err(RpcError::invalid_payload("expected void payload"))
        }
    }
}
