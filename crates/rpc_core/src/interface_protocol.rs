//! Versioned stream session envelopes. Legacy RPC envelopes remain unchanged.
use crate::interface::{ImplementationDescriptor, InvocationError};
use semantic_data::value::{Value, serde::typed::TypedValue};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const PROFILE: &str = "values-and-top-level-streams-v1";
pub const CODEC: &str = "typed-value-json";
pub const PLUGIN_SUBPROTOCOL: &str = "semantic.plugin.v1";

/// Derive the interface WebSocket path from the configured HTTP RPC path.
/// Conventional `/rpc` endpoints retain a sibling `/interface/ws` endpoint;
/// other RPC paths use an `/interface/ws` suffix.
pub fn interface_ws_path(rpc_path: &str) -> String {
    let path = rpc_path.trim_end_matches('/');
    let prefix = path.strip_suffix("/rpc").unwrap_or(path);
    format!("{prefix}/interface/ws")
}

/// Canonical decimal representation avoids JavaScript integer truncation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionId(pub u64);

impl TryFrom<String> for SessionId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.starts_with('0')
            || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err("expected positive canonical decimal session ID".into());
        }
        value
            .parse()
            .map(Self)
            .map_err(|_| "session ID overflow".into())
    }
}
impl From<SessionId> for String {
    fn from(value: SessionId) -> Self {
        value.0.to_string()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireArgument {
    Value(TypedValue),
    Stream(SessionId),
    /// Transfers a stream originally produced by this same peer, at its current
    /// position and with no outstanding demand.
    /// Its runtime metadata is recovered locally, never supplied by the sender.
    ForwardStream(SessionId),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireOutput {
    Values(Vec<TypedValue>),
    Stream(SessionId),
    Error(InvocationError),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceMessage {
    Hello {
        version: u32,
        profile: String,
        codec: String,
        revision: Option<String>,
        exports: Vec<ImplementationDescriptor>,
    },
    Configure {
        #[serde(with = "semantic_data::value::serde::typed")]
        configuration: Value,
    },
    Ready,
    Call {
        id: SessionId,
        export: String,
        method: String,
        arguments: Vec<WireArgument>,
    },
    Return {
        id: SessionId,
        output: WireOutput,
    },
    CancelCall {
        id: SessionId,
    },
    StreamDemand {
        id: SessionId,
        sequence: SessionId,
    },
    StreamItem {
        id: SessionId,
        sequence: SessionId,
        #[serde(with = "semantic_data::value::serde::typed")]
        value: Value,
    },
    StreamEnd {
        id: SessionId,
        sequence: SessionId,
        #[serde(with = "crate::protocol::optional_typed_value")]
        value: Option<Value>,
    },
    StreamError {
        id: SessionId,
        sequence: SessionId,
        error: InvocationError,
    },
    StreamCancel {
        id: SessionId,
    },
    Shutdown,
    ShutdownAck,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decimal_ids_reject_aliases_and_preserve_wide_values() {
        assert_eq!(
            serde_json::to_string(&SessionId(u64::MAX)).unwrap(),
            "\"18446744073709551615\""
        );
        for invalid in ["\"0\"", "\"01\"", "\"+1\"", "1", "\"18446744073709551616\""] {
            assert!(serde_json::from_str::<SessionId>(invalid).is_err());
        }
        let message = InterfaceMessage::StreamItem {
            id: SessionId(u64::MAX),
            sequence: SessionId(1),
            value: Value::U128(u128::MAX),
        };
        let encoded = serde_json::to_string(&message).unwrap();
        let InterfaceMessage::StreamItem { value, .. } = serde_json::from_str(&encoded).unwrap()
        else {
            panic!("stream item")
        };
        assert_eq!(value, Value::U128(u128::MAX));
    }
}
