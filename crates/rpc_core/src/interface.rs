//! Portable interface identities and failures. Live streams belong to semantic_rpc.
use semantic_data::value::Value;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InterfaceRef {
    pub package: String,
    pub module: String,
    pub contract: Option<String>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementationDescriptor {
    pub export: String,
    pub interface: InterfaceRef,
    pub package_version: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct InvocationError {
    pub code: String,
    pub message: String,
    #[serde(default, with = "crate::protocol::optional_typed_value")]
    pub data: Option<Value>,
}

impl InvocationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: None,
        }
    }
}
