use semantic_data::value::Value;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::protocol::optional_typed_value"
    )]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: impl Into<String>, message: impl Into<String>, data: Value) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn unknown_command(command: impl Into<String>) -> Self {
        let command = command.into();
        Self::new(
            "unknown_command",
            format!("unknown RPC command '{command}'"),
        )
    }

    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self::new("invalid_payload", message)
    }

    pub fn invalid_output(message: impl Into<String>) -> Self {
        Self::new("invalid_output", message)
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new("protocol_error", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

impl From<String> for RpcError {
    fn from(value: String) -> Self {
        Self::new("handler_error", value)
    }
}

impl From<&str> for RpcError {
    fn from(value: &str) -> Self {
        Self::new("handler_error", value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    #[error("duplicate RPC command '{0}'")]
    DuplicateCommand(String),
}

#[derive(Debug, thiserror::Error)]
pub enum RpcClientError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("remote error: {0}: {1}")]
    Remote(String, String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("encode error: {0}")]
    Encode(String),
}

impl From<RpcError> for RpcClientError {
    fn from(value: RpcError) -> Self {
        Self::Remote(value.code, value.message)
    }
}
