use dioxus::prelude::{use_context, use_context_provider};
use semantic_rpc::RpcClient;

pub fn provide_rpc_client(client: RpcClient) -> RpcClient {
    use_context_provider(|| client.clone());
    client
}

pub fn use_rpc_client() -> RpcClient {
    use_context::<RpcClient>()
}
