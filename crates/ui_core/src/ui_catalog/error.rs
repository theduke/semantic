#[derive(Clone, Debug, thiserror::Error)]
pub enum UiCatalogError {
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("invalid catalog response: {0}")]
    InvalidResponse(String),
    #[error("catalog decode error: {0}")]
    Decode(String),
}

impl From<semantic_rpc::RpcClientError> for UiCatalogError {
    fn from(value: semantic_rpc::RpcClientError) -> Self {
        Self::Rpc(value.to_string())
    }
}
