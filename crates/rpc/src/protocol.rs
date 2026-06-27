use semantic_data::value::Value;
use serde::{Deserialize, Serialize};

use crate::error::RpcError;

pub type RpcRequestId = u64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: RpcRequestId,
    pub command: String,
    #[serde(with = "semantic_data::value::serde::typed")]
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcResponse {
    pub id: RpcRequestId,
    pub result: RpcResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcResult {
    Ok(#[serde(with = "semantic_data::value::serde::typed")] Value),
    Err(RpcError),
}

impl RpcResponse {
    pub fn ok(id: RpcRequestId, value: Value) -> Self {
        Self {
            id,
            result: RpcResult::Ok(value),
        }
    }

    pub fn err(id: RpcRequestId, error: RpcError) -> Self {
        Self {
            id,
            result: RpcResult::Err(error),
        }
    }
}

pub mod optional_typed_value {
    use semantic_data::value::Value;
    use semantic_data::value::serde::typed::TypedValue;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(value: &Option<Value>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.as_ref().map(TypedValueRef).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<TypedValue>::deserialize(deserializer).map(|value| value.map(|value| value.0))
    }

    struct TypedValueRef<'a>(&'a Value);

    impl Serialize for TypedValueRef<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            semantic_data::value::serde::typed::serialize(self.0, serializer)
        }
    }
}
